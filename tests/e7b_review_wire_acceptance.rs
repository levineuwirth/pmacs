// tests/e7b_review_wire_acceptance.rs --- C7b review 1: the two
// premises E7b.1 corrected, read on the wire.

//! E7b.1's handoff corrects two premises of its row where the code is:
//! the client never declared `window.workDoneProgress`, so
//! rust-analyzer --- which honors the capability gate --- sent no
//! `$/progress` at all; and nothing in pmacs sends
//! `textDocument/didSave`, so a save runs no flycheck. Both are claims
//! about bytes, and this suite reads the bytes: rust-analyzer runs
//! behind a shell wrapper that tees both directions of the wire to
//! files, on a one-file cargo project, and each row parses the frames.
//!
//! * The `initialize` params carry `window.workDoneProgress: true`,
//!   the server asks `window/workDoneProgress/create`, and `$/progress`
//!   arrives. Bitten by `scripts/bite e190f80 src/lsp.rs --test
//!   e7b_review_wire_acceptance -- declares_progress`: against the
//!   pre-branch client the capability is absent and the server sends
//!   no progress, which is the premise measured rather than read.
//! * A save sends one `textDocument/didSave`, shaped as the server's
//!   `save` capability asks (rust-analyzer: `includeText: false`, so
//!   no text), after the edit's `didChange`, and rust-analyzer runs
//!   its flycheck on it --- `$/progress` begin and end on a
//!   `rust-analyzer/flycheck/<n>` token --- with the label reading
//!   `ready·check` in between and never `idx` on its account (E7c.1;
//!   until then this row pinned that no `didSave` was ever sent).
//!   Bitten by removing the after-save hook in `lsp.lua`.
//!
//! Needs `rust-analyzer`, `cargo`, `sh` and `tee` on `PATH`; skips
//! otherwise unless `PMACS_REQUIRE_LSP` is set.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};
use serde_json::Value;

#[path = "common/iso.rs"]
mod iso;
#[path = "support/mod.rs"]
mod support;

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
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Tick for `secs`, reading the modeline's `LSP:` segment each time,
/// and return every distinct text it showed in order.
fn pump_labels(s: &mut EditorState, secs: u64, labels: &mut Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        tick(s);
        let text = lsp_segment(s).unwrap_or_default();
        if labels.last() != Some(&text) {
            labels.push(text);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

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

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(
            s,
            if ch == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(ch)
            },
        );
    }
}

/// Tick for `secs` and say whether `method` appeared in the client's
/// capture at least `n` times by then.
fn wait_sent(s: &mut EditorState, cap: &Capture, method: &str, n: usize, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if count_method(&frames(&cap.to_server), method) >= n {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The completion popup as the core holds it: `(prefix, labels)` when
/// open, `None` when closed.
fn popup_snapshot(s: &EditorState) -> Option<(String, Vec<String>)> {
    let core = s.core.borrow();
    let popup = core.completion_popup.lock().unwrap();
    popup.as_ref().map(|p| {
        (
            p.prefix.clone(),
            p.candidates.iter().map(|c| c.label.clone()).collect(),
        )
    })
}

/// Tick for `secs` and log every frame that appears in either capture
/// with the millisecond it was first seen, `>` for client-to-server and
/// `<` for the reverse; `$/progress` frames carry their token and kind.
fn watch_wire(s: &mut EditorState, cap: &Capture, secs: u64) -> Vec<String> {
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(secs);
    let (mut to, mut from) = (Tap::new(&cap.to_server), Tap::new(&cap.from_server));
    // Everything already on the wire is the past.
    to.new_frames();
    from.new_frames();
    let mut log = Vec::new();
    let mut last_label = String::new();
    let mut last_busy = String::new();
    while Instant::now() < deadline {
        tick(s);
        let ms = t0.elapsed().as_millis();
        for f in to.new_frames() {
            log.push(format!("{ms} > {}", describe(&f)));
        }
        for f in from.new_frames() {
            log.push(format!("{ms} < {}", describe(&f)));
        }
        let label = lsp_segment(s).unwrap_or_default();
        if label != last_label {
            log.push(format!("{ms} = {label}"));
            last_label = label;
        }
        // The tracker's busy title beside the label (`b <title>`; `b -`
        // for none): the label shows the kind while it is `idx`, so a
        // check that runs under a reload is busy on the tracker and
        // invisible on the label (E7b.1's rule, the kind wins).
        let busy: Option<String> = s
            .lua_host
            .lua()
            .load(
                "local rec = pmacs.lsp.active_attachment()
                 local st = rec and pmacs.lsp.status_summary(rec.server)
                 return st and st.busy or nil",
            )
            .eval()
            .unwrap_or(None);
        let busy = busy.unwrap_or_else(|| "-".to_owned());
        if busy != last_busy {
            log.push(format!("{ms} b {busy}"));
            last_busy = busy;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    log
}

fn describe(f: &Value) -> String {
    match method_of(f) {
        Some("$/progress") => format!(
            "$/progress {} {}{}",
            f["params"]["token"],
            f["params"]["value"]["kind"].as_str().unwrap_or("?"),
            f["params"]["value"]["title"]
                .as_str()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default()
        ),
        Some("workspace/didChangeWatchedFiles") => {
            format!("workspace/didChangeWatchedFiles {}", f["params"]["changes"])
        }
        Some(m) => m.to_owned(),
        None => "<response>".to_owned(),
    }
}

/// Tick until the label has read exactly `LSP:ready` for two seconds
/// --- priming done, the flycheck done --- or `secs` pass.
fn wait_warm(s: &mut EditorState, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut ready_since: Option<Instant> = None;
    loop {
        tick(s);
        if lsp_segment(s).as_deref() == Some("LSP:ready") {
            let since = *ready_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(2) {
                return true;
            }
        } else {
            ready_since = None;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7b-rev-{tag}-{}", std::process::id()));
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

fn lsp_segment(s: &EditorState) -> Option<String> {
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
        .find(|seg| seg.face == "ui.modeline.lsp")
        // The `ready` family carries a fixed slot for the busy suffix
        // (C7c fix round 3); the label is what these rows read.
        .map(|seg| seg.text.trim_end().to_owned())
}

/// The capture: a one-file cargo project, the rust server pointed at a
/// wrapper that tees the client's bytes to `to_server` and the
/// server's to `from_server`, and the editor visiting `src/main.rs`
/// with the handshake answered.
struct Capture {
    to_server: PathBuf,
    from_server: PathBuf,
    main_rs: PathBuf,
}

fn open_with_captured_rust_analyzer(tag: &str) -> (EditorState, Capture) {
    let dir = temp_dir(tag);
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"e7b_review_wire\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let main_rs = dir.join("src").join("main.rs");
    std::fs::write(&main_rs, "fn main() {\n    let n = 1;\n    let _ = n;\n}\n").unwrap();
    let to_server = dir.join("to-server.jsonrpc");
    let from_server = dir.join("from-server.jsonrpc");
    let wrapper = dir.join("rust-analyzer-captured");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n\
             tee -a {to} | rust-analyzer \"$@\" 2>>{err} | tee -a {from}\n",
            to = shell_quote(&to_server),
            err = shell_quote(&dir.join("rust-analyzer.stderr")),
            from = shell_quote(&from_server),
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    // The default rust config with only the command replaced, so the
    // init options are the shipped ones; the check a save now runs
    // (E7c.1) builds under the project's own `target`, not the
    // ambient `CARGO_TARGET_DIR` a gate or a shell exports.
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust.command = {:?}
             pmacs.lsp.config.rust.env = {{ CARGO_TARGET_DIR = {:?} }}",
            wrapper.display().to_string(),
            dir.join("target").display().to_string()
        ),
    );
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            main_rs.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 90), "rust-analyzer init");
    (
        s,
        Capture {
            to_server,
            from_server,
            main_rs,
        },
    )
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Every complete JSON-RPC frame in a capture file, in order; a
/// trailing partial frame is ignored. For the small captures the
/// one-file project produces; the watchers use [`Tap`].
fn frames(path: &Path) -> Vec<Value> {
    parse_frames(&std::fs::read(path).unwrap_or_default()).0
}

/// An incremental reader over one capture file: `new_frames` returns
/// the complete frames appended since the last call and keeps a
/// partial trailing frame for the next. A 14k-line file's semantic
/// tokens make the server's capture tens of megabytes in a minute, so
/// a watcher that re-parsed it whole each tick would measure itself.
struct Tap {
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
}

impl Tap {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            offset: 0,
            pending: Vec::new(),
        }
    }

    fn new_frames(&mut self) -> Vec<Value> {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut fresh = Vec::new();
        if file.read_to_end(&mut fresh).is_err() {
            return Vec::new();
        }
        self.offset += fresh.len() as u64;
        self.pending.extend_from_slice(&fresh);
        let (out, consumed) = parse_frames(&self.pending);
        self.pending.drain(..consumed);
        out
    }
}

/// Every complete frame at the front of `bytes`, and how many bytes
/// they took.
fn parse_frames(bytes: &[u8]) -> (Vec<Value>, usize) {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let Some(header_end) = find(&bytes[at..], b"\r\n\r\n") else {
            break;
        };
        let header = String::from_utf8_lossy(&bytes[at..at + header_end]);
        let length = header
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|n| n.trim().parse::<usize>().ok())
            .expect("a Content-Length header on every frame");
        let body_start = at + header_end + 4;
        let body_end = body_start + length;
        if body_end > bytes.len() {
            break;
        }
        out.push(serde_json::from_slice(&bytes[body_start..body_end]).expect("a JSON body"));
        at = body_end;
    }
    (out, at)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn method_of(frame: &Value) -> Option<&str> {
    frame.get("method").and_then(Value::as_str)
}

fn count_method(frames: &[Value], method: &str) -> usize {
    frames
        .iter()
        .filter(|f| method_of(f) == Some(method))
        .count()
}

fn histogram(frames: &[Value]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for f in frames {
        let name = method_of(f).unwrap_or("<response>").to_owned();
        match counts.iter_mut().find(|(m, _)| *m == name) {
            Some((_, n)) => *n += 1,
            None => counts.push((name, 1)),
        }
    }
    counts
}

/// E7b.1's first premise, on the wire: the client declares
/// `window.workDoneProgress`, the server asks to create tokens, and
/// `$/progress` arrives --- an indexing token among them, since a
/// fresh project primes its cache. Against `e190f80`'s `src/lsp.rs`
/// the `initialize` params have no `window` member and the count of
/// `$/progress` frames is zero.
#[test]
fn the_client_declares_progress_and_rust_analyzer_sends_it() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let (mut s, cap) = open_with_captured_rust_analyzer("progress");
    let mut labels = Vec::new();
    // Give the server its warm-up: fetching, roots, the crate graph,
    // cache priming and the first flycheck on a one-file project are
    // seconds, and the row waits for the priming's `end` or thirty.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        pump_labels(&mut s, 1, &mut labels);
        let primed = frames(&cap.from_server).iter().any(|f| {
            method_of(f) == Some("$/progress")
                && f["params"]["token"] == "rustAnalyzer/cachePriming"
                && f["params"]["value"]["kind"] == "end"
        });
        if primed || Instant::now() >= deadline {
            break;
        }
    }
    let sent = frames(&cap.to_server);
    let received = frames(&cap.from_server);
    let init = sent
        .iter()
        .find(|f| method_of(f) == Some("initialize"))
        .expect("an initialize request on the wire");
    eprintln!(
        "WIRE initialize capabilities.window = {}",
        init["params"]["capabilities"]["window"]
    );
    let tokens: Vec<String> = received
        .iter()
        .filter(|f| method_of(f) == Some("$/progress"))
        .filter_map(|f| f["params"]["token"].as_str().map(str::to_owned))
        .fold(Vec::new(), |mut acc, t| {
            if !acc.contains(&t) {
                acc.push(t);
            }
            acc
        });
    eprintln!("WIRE $/progress tokens seen: {tokens:?}");
    eprintln!("WIRE modeline labels seen: {labels:?}");
    eprintln!("WIRE server->client histogram: {:?}", histogram(&received));
    assert_eq!(
        init["params"]["capabilities"]["window"]["workDoneProgress"],
        Value::Bool(true),
        "the initialize params declare window.workDoneProgress"
    );
    assert!(
        count_method(&received, "window/workDoneProgress/create") >= 1,
        "the server asks to create a progress token"
    );
    assert!(
        count_method(&received, "$/progress") >= 1,
        "the server sends $/progress once the capability is declared"
    );
    // The priming token by name on the version this was written
    // against, and any `INDEXING_TOKENS` member on another: CI's
    // rust-analyzer is the pinned toolchain's, not this machine's.
    assert!(
        tokens
            .iter()
            .any(|t| pmacs::lsp_status::is_indexing_token(t)),
        "an indexing token is among them: {tokens:?}"
    );
}

/// E7b.1's second premise, on the wire: a save of a modified buffer
/// sends no `textDocument/didSave`. The positive controls are the
/// `didOpen` of the visit and the `didChange` of the typed edit before
/// the save, read from the same capture after the same point, and the
/// save's own after-save hook and bytes on disk.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one save read on the wire from the edit before it to the check after it"
)]
fn a_save_sends_did_save_and_rust_analyzer_flychecks_on_it() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let (mut s, cap) = open_with_captured_rust_analyzer("save");
    // Warm: priming done and the server's own first check over, so the
    // flycheck the save starts is the one the watch attributes to it.
    assert!(
        wait_warm(&mut s, 90),
        "the label reads exactly ready for two seconds"
    );
    exec(
        &s,
        "_G.__e7b_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7b_saved = true end)
         pmacs.editor.goto_byte(0)",
    );
    type_str(&mut s, "// a line the review typed");
    // The auto-completion popup may be open on the last word; the row
    // is about the save, so it is dismissed before the newline, and
    // what it held is printed for the record.
    let mut labels = Vec::new();
    pump_labels(&mut s, 1, &mut labels);
    let popup = popup_snapshot(&s);
    eprintln!("WIRE the popup after typing the comment: {popup:?}");
    if popup.is_some() {
        press(&mut s, KeyCode::Esc);
    }
    press(&mut s, KeyCode::Enter);
    assert!(
        wait_sent(&mut s, &cap, "textDocument/didChange", 1, 10),
        "the typed edit reaches the server as didChange"
    );
    let before_save = frames(&cap.to_server).len();
    let flychecks_before = frames(&cap.from_server)
        .iter()
        .filter(|f| is_flycheck_begin(f))
        .count();
    exec(&s, "pmacs.command.invoke('buffer.save')");
    assert!(
        pump_lua_flag(&mut s, "_G.__e7b_saved", 10),
        "buffer.after-save fires; status {:?}",
        s.core.borrow().status
    );
    let on_disk = std::fs::read_to_string(&cap.main_rs).unwrap();
    assert!(
        on_disk.starts_with("// a line the review typed\n"),
        "the save wrote the edit: {on_disk:?}"
    );
    // Ten seconds for the check to start and finish on a one-file
    // project, the label sampled every few milliseconds.
    let wire = watch_wire(&mut s, &cap, 10);
    eprintln!("WIRE after the save, by the millisecond:");
    for line in &wire {
        eprintln!("WIRE   {line}");
    }
    let sent = frames(&cap.to_server);
    let after_save: Vec<&Value> = sent[before_save..].iter().collect();
    eprintln!(
        "WIRE client->server after the save: {:?}",
        after_save
            .iter()
            .filter_map(|f| method_of(f))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        count_method(&sent, "textDocument/didOpen"),
        1,
        "one didOpen"
    );
    let saves: Vec<&Value> = sent
        .iter()
        .filter(|f| method_of(f) == Some("textDocument/didSave"))
        .collect();
    assert_eq!(
        saves.len(),
        1,
        "one didSave for one save: {:?}",
        histogram(&sent)
    );
    assert!(
        after_save
            .iter()
            .any(|f| method_of(f) == Some("textDocument/didSave")),
        "and it went out after the save command"
    );
    // The shape the server asked for: rust-analyzer declares
    // `save: { includeText: false }`, read from its own initialize
    // answer in the capture rather than assumed.
    let include_text = frames(&cap.from_server)
        .iter()
        .find_map(|f| {
            f["result"]["capabilities"]["textDocumentSync"]["save"]
                .as_object()
                .map(|save| {
                    save.get("includeText")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
        })
        .expect("rust-analyzer's initialize answer declares save");
    eprintln!("WIRE rust-analyzer negotiated includeText = {include_text}");
    assert_eq!(
        saves[0]["params"].get("text").is_some(),
        include_text,
        "the text rides only when asked: {:?}",
        saves[0]["params"]
    );
    assert_eq!(
        saves[0]["params"]["textDocument"]["uri"],
        sent.iter()
            .find(|f| method_of(f) == Some("textDocument/didOpen"))
            .map(|f| f["params"]["textDocument"]["uri"].clone())
            .unwrap(),
        "the saved document is the opened one"
    );
    // The flycheck: a begin and an end on the token, after the save.
    let from = frames(&cap.from_server);
    let flychecks_after = from.iter().filter(|f| is_flycheck_begin(f)).count();
    assert!(
        flychecks_after > flychecks_before,
        "a flycheck began after the save ({flychecks_before} before, {flychecks_after} after)"
    );
    let token = from
        .iter()
        .rev()
        .find(|f| is_flycheck_begin(f))
        .map(|f| f["params"]["token"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(
        from.iter().any(|f| method_of(f) == Some("$/progress")
            && f["params"]["token"].as_str() == Some(token.as_str())
            && f["params"]["value"]["kind"].as_str() == Some("end")),
        "and ended on {token}"
    );
    // The label: the suffix while the check ran, and `ready` on its
    // own account throughout --- a `$/progress` cycle on a flycheck
    // token never reads `idx` (E7b.1). When the server reloads or
    // re-primes across the check (CI's ubuntu lua54 leg read `idx`
    // from the save to past the check, its cache priming restarted by
    // the edit), the kind masks the suffix by E7b.1's own rule, and the
    // check's title is read on the tracker's busy field instead.
    let seen: Vec<&String> = wire.iter().filter(|l| l.contains(" = ")).collect();
    let busy: Vec<&String> = wire.iter().filter(|l| l.contains(" b ")).collect();
    eprintln!("WIRE labels after the save: {seen:?}; busy: {busy:?}");
    assert!(
        seen.iter()
            .any(|l| l.ends_with("= LSP:ready·check") || l.ends_with("= LSP:ready·clippy"))
            || busy
                .iter()
                .any(|l| l.ends_with("b cargo check") || l.ends_with("b cargo clippy")),
        "the check showed as a suffix on ready, or as the tracker's busy title under a reload: {seen:?} {busy:?}"
    );
    assert!(
        !seen.iter().any(|l| l.ends_with("= LSP:degraded")),
        "and never as degraded: {seen:?}"
    );
    assert_eq!(
        lsp_segment(&s).as_deref(),
        Some("LSP:ready"),
        "ready once the check ended"
    );
    let clean: bool = eval(
        &s,
        "return pmacs.lsp.status_summary(pmacs.lsp.active_attachment().server).last_error == nil",
    );
    assert!(clean, "no error response during the sequence");
}

/// A `$/progress` `begin` on rust-analyzer's flycheck token.
fn is_flycheck_begin(f: &Value) -> bool {
    method_of(f) == Some("$/progress")
        && f["params"]["token"]
            .as_str()
            .is_some_and(|t| t.starts_with("rust-analyzer/flycheck/"))
        && f["params"]["value"]["kind"].as_str() == Some("begin")
}

/// E7c.1's retry on the trigger the owner ruled at fix round 3, against
/// the real server: a save followed at once by a keystroke sends the
/// save again right behind the keystroke's `didChange` --- two
/// `didSave` for one save, the second the frame after a `didChange`
/// --- and a check runs on it. On this one-file project rust-analyzer
/// begins the save's check about 70 ms after the `didSave`, and the
/// coalesced flush would put a plain keystroke's `didChange` 75 ms
/// behind the key, after the begin had stood the watch down; so the
/// key is `(`, one of rust-analyzer's signature-help triggers, whose
/// after-edit hook flushes the `didChange` in the keystroke itself,
/// and it is pressed the moment the save command returns --- the
/// `didChange` a few milliseconds behind the `didSave`, inside the
/// window review 1 measured (6--54 ms lost, 126 ms and later kept).
/// How many checks begin is then the server's: one when the first was
/// lost, two when it was not; the count is printed and at least one is
/// asserted. No third `didSave` follows, the flycheck's begin having
/// stood the watch down. The row above, a save with nothing typed
/// after it, counts one `didSave` however late the server begins
/// (#285's case, closed by this trigger). Bitten by removing the
/// resend from `flush_did_change`.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one save and one keystroke read on the wire from the edit before them to the check after"
)]
fn a_keystroke_after_the_save_resends_it_behind_the_did_change() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let (mut s, cap) = open_with_captured_rust_analyzer("resend");
    assert!(
        wait_warm(&mut s, 90),
        "the label reads exactly ready for two seconds"
    );
    exec(
        &s,
        "_G.__e7b_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7b_saved = true end)
         pmacs.editor.goto_byte(0)",
    );
    type_str(&mut s, "// a line typed before the save");
    let mut labels = Vec::new();
    pump_labels(&mut s, 1, &mut labels);
    if popup_snapshot(&s).is_some() {
        press(&mut s, KeyCode::Esc);
    }
    press(&mut s, KeyCode::Enter);
    assert!(
        wait_sent(&mut s, &cap, "textDocument/didChange", 1, 10),
        "the typed edit reaches the server as didChange"
    );
    let before_save = frames(&cap.to_server).len();
    let flychecks_before = frames(&cap.from_server)
        .iter()
        .filter(|f| is_flycheck_begin(f))
        .count();
    let t_save = Instant::now();
    exec(&s, "pmacs.command.invoke('buffer.save')");
    // The keystroke the moment the save command returns (the save is
    // synchronous: the file written, the after-save hook fired and the
    // `didSave` sent inside it), on the line below the typed one; `(`
    // flushes its own `didChange`.
    press(&mut s, KeyCode::Char('('));
    let typed_at = t_save.elapsed().as_millis();
    let saved: bool = eval(&s, "return _G.__e7b_saved == true");
    assert!(
        saved,
        "buffer.after-save fired inside the save command; status {:?}",
        s.core.borrow().status
    );
    let wire = watch_wire(&mut s, &cap, 10);
    eprintln!(
        "WIRE the keystroke went in {typed_at} ms after the save; after the save, by the millisecond:"
    );
    for line in &wire {
        eprintln!("WIRE   {line}");
    }
    let sent = frames(&cap.to_server);
    let after_save: Vec<&Value> = sent[before_save..].iter().collect();
    let methods: Vec<&str> = after_save.iter().filter_map(|f| method_of(f)).collect();
    eprintln!("WIRE client->server after the save: {methods:?}");
    let save_positions: Vec<usize> = methods
        .iter()
        .enumerate()
        .filter(|(_, m)| **m == "textDocument/didSave")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        save_positions.len(),
        2,
        "two didSave for one save and one keystroke: {methods:?}"
    );
    let (first, second) = (save_positions[0], save_positions[1]);
    assert!(
        methods[second - 1] == "textDocument/didChange",
        "the second didSave is the frame after the keystroke's didChange: {methods:?}"
    );
    // The `didChange` before the first `didSave` is the save's own
    // flush of the typed line; between the save and its resend there
    // is the keystroke's alone.
    assert!(
        methods[first..second]
            .iter()
            .filter(|m| **m == "textDocument/didChange")
            .count()
            == 1,
        "one didChange between the save and its resend: {methods:?}"
    );
    let from = frames(&cap.from_server);
    let begins = from.iter().filter(|f| is_flycheck_begin(f)).count() - flychecks_before;
    eprintln!("WIRE flycheck begins after the save: {begins}");
    assert!(
        begins >= 1,
        "a check began on the save or on its resend ({flychecks_before} before)"
    );
    let token = from
        .iter()
        .rev()
        .find(|f| is_flycheck_begin(f))
        .map(|f| f["params"]["token"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(
        from.iter().any(|f| method_of(f) == Some("$/progress")
            && f["params"]["token"].as_str() == Some(token.as_str())
            && f["params"]["value"]["kind"].as_str() == Some("end")),
        "and the last check ended on {token}"
    );
    let watches: std::collections::HashMap<String, u32> =
        eval(&s, "return pmacs.lsp._save_watches()");
    assert!(
        watches.is_empty(),
        "the watch stood down on the check's begin: {watches:?}"
    );
    let clean: bool = eval(
        &s,
        "return pmacs.lsp.status_summary(pmacs.lsp.active_attachment().server).last_error == nil",
    );
    assert!(clean, "no error response during the sequence");
}

/// The measurement's instrument, pinned: an edit made from Lua outside
/// any command --- `buf:insert` from a test's `exec`, which is how the
/// handoff's "broken" and "completion" rounds put their text into
/// `src/editor.rs` --- fires no `buffer.after-edit` hook and so queues
/// no `didChange`; the server's document is the one it had. A typed
/// keystroke on the same buffer sends one. The row is what makes a
/// "five saves of an unparsable file" measurement readable: the file
/// on disk was unparsable, the document rust-analyzer held was not.
#[test]
fn a_lua_insert_outside_a_command_reaches_no_server_and_a_keystroke_does() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let (mut s, cap) = open_with_captured_rust_analyzer("luainsert");
    exec(&s, "pmacs.window.buffer():insert(0, 'X')");
    let lua_reached = wait_sent(&mut s, &cap, "textDocument/didChange", 1, 2);
    let text: String = eval(&s, "return pmacs.window.buffer():slice(0, 12)");
    eprintln!("WIRE after the Lua insert: buffer starts {text:?}, didChange sent = {lua_reached}");
    assert!(
        !lua_reached,
        "a Lua insert outside a command queued a didChange; the handoff's rounds meant more than this row says"
    );
    exec(&s, "pmacs.editor.goto_byte(1)");
    press(&mut s, KeyCode::Char('Y'));
    let typed_reached = wait_sent(&mut s, &cap, "textDocument/didChange", 1, 10);
    assert!(
        typed_reached,
        "a keystroke's edit reaches the server as didChange"
    );
    let sent = frames(&cap.to_server);
    let change = sent
        .iter()
        .find(|f| method_of(f) == Some("textDocument/didChange"))
        .unwrap();
    eprintln!(
        "WIRE the first didChange: {}",
        change["params"]["contentChanges"]
    );
    // What the server now holds begins with both characters only if
    // the change went whole; a ranged change carries the `Y` alone and
    // the `X` is the review's to name as never sent.
    let changes = change["params"]["contentChanges"].as_array().unwrap();
    let whole = changes.len() == 1 && changes[0].get("range").is_none();
    eprintln!("WIRE the change went whole: {whole}");
}

// ---------------------------------------------------------------------------
// A measurement against rust-analyzer on this repository, by hand
// ---------------------------------------------------------------------------

/// The state the handoff's five-saves measurement could not reach,
/// because a save of unchanged bytes changes nothing the server holds:
/// typing. With rust-analyzer warm on `src/editor.rs`, type inside a
/// function body with pauses, then a new top-level item, then delete
/// it, and record every transition of the modeline label and every
/// `$/progress` begin on the wire with its token. The buffer is never
/// saved, so the file on disk is untouched. Run with
/// `PMACS_REQUIRE_LSP=1 cargo test --test e7b_review_wire_acceptance
/// -- --ignored --nocapture measure_typing`.
#[test]
#[ignore = "a measurement against rust-analyzer on src/editor.rs; run by hand and record"]
#[allow(
    clippy::too_many_lines,
    reason = "one linear measurement session, printed as it goes"
)]
fn measure_typing_on_editor_rs_with_the_server_warm() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let dir = temp_dir("typing");
    let to_server = dir.join("to-server.jsonrpc");
    let from_server = dir.join("from-server.jsonrpc");
    let wrapper = dir.join("rust-analyzer-captured");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n\
             tee -a {to} | rust-analyzer \"$@\" 2>>{err} | tee -a {from}\n",
            to = shell_quote(&to_server),
            err = shell_quote(&dir.join("rust-analyzer.stderr")),
            from = shell_quote(&from_server),
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("editor.rs");
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust.command = {:?}",
            wrapper.display().to_string()
        ),
    );
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
    let cap = Capture {
        to_server,
        from_server,
        main_rs: target,
    };
    // Warm: the label exactly `LSP:ready` for three seconds.
    let warm_deadline = Instant::now() + Duration::from_mins(10);
    let mut ready_since: Option<Instant> = None;
    let mut warm_labels = Vec::new();
    loop {
        pump_labels(&mut s, 1, &mut warm_labels);
        if lsp_segment(&s).as_deref() == Some("LSP:ready") {
            let since = *ready_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(3) {
                break;
            }
        } else {
            ready_since = None;
        }
        assert!(
            Instant::now() < warm_deadline,
            "never warm: {warm_labels:?}"
        );
    }
    eprintln!(
        "MEASURE warm at {} ms; labels on the way: {warm_labels:?}",
        t0.elapsed().as_millis()
    );
    let rounds: [(&str, &str, &str); 3] = [
        (
            "comment",
            // Inside the first line's comment: no item tree change.
            "pmacs.editor.goto_byte(3)",
            "review typed here ",
        ),
        (
            "item",
            // A new top-level item at the end of the file.
            "local b = pmacs.window.buffer() pmacs.editor.goto_byte(b:len())",
            "\nfn e7b_review_probe() {}\n",
        ),
        (
            "body",
            // Inside the new item's body.
            "local b = pmacs.window.buffer() pmacs.editor.goto_byte(b:len() - 2)",
            " let _x = 1; ",
        ),
    ];
    let mut errors_seen = Vec::new();
    let (mut to, mut from) = (Tap::new(&cap.to_server), Tap::new(&cap.from_server));
    to.new_frames();
    from.new_frames();
    for (name, goto, text) in rounds {
        exec(&s, goto);
        let t = Instant::now();
        let mut log = Vec::new();
        let mut last_label = String::new();
        let mut note = |s: &mut EditorState, log: &mut Vec<String>, last_label: &mut String| {
            tick(s);
            let ms = t.elapsed().as_millis();
            for f in to.new_frames() {
                if method_of(&f).is_some_and(|m| m.starts_with("textDocument/did")) {
                    log.push(format!("{ms} > {}", describe(&f)));
                }
            }
            for f in from.new_frames() {
                if method_of(&f) == Some("$/progress") && f["params"]["value"]["kind"] != "report" {
                    log.push(format!("{ms} < {}", describe(&f)));
                }
            }
            let label = lsp_segment(s).unwrap_or_default();
            if label != *last_label {
                log.push(format!("{ms} = {label}"));
                *last_label = label;
            }
        };
        for ch in text.chars() {
            press(
                &mut s,
                if ch == '\n' {
                    KeyCode::Enter
                } else {
                    KeyCode::Char(ch)
                },
            );
            let until = Instant::now() + Duration::from_millis(120);
            while Instant::now() < until {
                note(&mut s, &mut log, &mut last_label);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        // Then quiet until the label has read exactly ready for three
        // seconds, or twenty seconds.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut ready_since: Option<Instant> = None;
        loop {
            note(&mut s, &mut log, &mut last_label);
            if lsp_segment(&s).as_deref() == Some("LSP:ready") {
                let since = *ready_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(3) {
                    break;
                }
            } else {
                ready_since = None;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        eprintln!(
            "MEASURE round {name}: {} keystrokes at 120 ms",
            text.chars().count()
        );
        for line in &log {
            eprintln!("MEASURE   {line}");
        }
        let errors: Vec<String> = eval(
            &s,
            "local out = {}
             local rec = pmacs.lsp.active_attachment()
             local st = pmacs.lsp.status_summary(rec.server)
             if st.last_error then
               out[#out + 1] = tostring(st.last_error.source) .. ': ' .. tostring(st.last_error.message)
             end
             for _, m in ipairs(pmacs.lsp.recent_messages(rec.server)) do
               if m.channel == 'error' then
                 out[#out + 1] = m.summary .. (m.detail and (' -- ' .. m.detail) or '')
               end
             end
             return out",
        );
        eprintln!("MEASURE round {name}: errors {errors:?}");
        errors_seen.extend(errors);
    }
    eprintln!("MEASURE errors over the session: {errors_seen:?}");
}

/// The one exit from `ready` a user could reach with one command,
/// closed by the owner's ruling at C7b fix round 1: a rename asked
/// where there is nothing to rename. rust-analyzer answers
/// `textDocument/prepareRename` with `-32602 InvalidParams`, "No
/// references found at position" --- a client-caused code, the server
/// answering rather than failing --- and the modeline stays
/// `LSP:ready`; `*errors*` gains one line naming the method and the
/// code, `*lsp*` logs the response, and `last_error` stays `nil` so no
/// sticky window is armed. At review 1 the same command read
/// `LSP:degraded` for fifteen seconds; bitten by `scripts/bite HEAD^
/// src/lsp_status.rs` (the label degrades) and `scripts/bite HEAD^
/// builtin/runtime/lsp.lua` (`*errors*` gains nothing).
#[test]
fn a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let (mut s, cap) = open_with_captured_rust_analyzer("rename");
    assert!(
        wait_warm(&mut s, 60),
        "the label reads exactly ready for two seconds"
    );
    // The blank between `fn main() {` and `let`: line 1, column 0.
    exec(&s, "pmacs.editor.goto_byte(12)");
    let before = lsp_segment(&s);
    let errors_before = s.lua_host.errors_buffer_text();
    // rust-analyzer answers a request that lands while its VFS is
    // mid-load with `-32801 content modified` (its dispatcher's
    // not-ready arm), which under the ruling is a moot request and
    // silent --- CI's Linux legs met it once, at `ab8e488`, on the
    // first rename after warm-up. Ask again until the server answers
    // on the merits; a retry answer is the wrong positive control,
    // not the wrong verdict.
    let mut status = String::new();
    for attempt in 1..=6 {
        exec(&s, "pmacs.command.invoke('lsp.rename')");
        let wire = watch_wire(&mut s, &cap, 3);
        eprintln!("WIRE before the rename: {before:?}");
        for line in &wire {
            eprintln!("WIRE   {line}");
        }
        status = s.core.borrow().status.clone();
        eprintln!("WIRE attempt {attempt}: status after the rename: {status:?}");
        if !(status.contains("error -32801") || status.contains("error -32800")) {
            break;
        }
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            tick(&mut s);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let (label, error, messages): (String, Option<String>, Vec<String>) = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment()
         local st = pmacs.lsp.status_summary(rec.server)
         local err = st.last_error and (tostring(st.last_error.source) .. ': ' .. tostring(st.last_error.message)
           .. ' (code ' .. tostring(st.last_error.code) .. ')')
         local out = {}
         for _, m in ipairs(pmacs.lsp.recent_messages(rec.server)) do
           if m.channel == 'error' then
             out[#out + 1] = m.summary .. (m.detail and (' -- ' .. m.detail) or '')
           end
         end
         return pmacs.lsp.modeline_label(rec.server), err, out",
    );
    let errors_after = s.lua_host.errors_buffer_text();
    let new_errors: Vec<&str> = errors_after[errors_before.len()..].lines().collect();
    eprintln!("WIRE label {label:?}, last_error {error:?}, *lsp* errors {messages:?}");
    eprintln!("WIRE *errors* gained {new_errors:?}");
    assert_eq!(
        before.as_deref(),
        Some("LSP:ready"),
        "ready before the rename"
    );
    // The positive control: the server did answer the request with
    // the client-caused code (read from the whole capture, since the
    // exchange can complete before the watch takes its baseline), and
    // `*lsp*` holds that answer.
    let answered = frames(&cap.from_server)
        .iter()
        .any(|f| f["error"]["code"].as_i64() == Some(-32602));
    assert!(
        answered,
        "rust-analyzer answered it with -32602 InvalidParams; the last status: {status:?}"
    );
    assert_eq!(
        lsp_segment(&s).as_deref(),
        Some("LSP:ready"),
        "the declined rename leaves the server ready on the modeline"
    );
    assert_eq!(label, "ready");
    assert!(
        messages
            .iter()
            .any(|m| m.contains("response error: textDocument/prepareRename")
                && m.contains("-32602")),
        "*lsp* names the request and the code: {messages:?}"
    );
    assert!(
        error.is_none(),
        "a client-caused code arms no sticky window: {error:?}"
    );
    assert_eq!(
        new_errors.len(),
        1,
        "*errors* gained the one line: {new_errors:?}"
    );
    assert!(
        new_errors[0].contains("textDocument/prepareRename")
            && new_errors[0].contains("-32602 InvalidParams"),
        "and it names the method and the code: {:?}",
        new_errors[0]
    );
}

/// #279's cost, measured: the file-watch poll's tree walk over this
/// repository's root, fifteen times --- one minute at the 4 s resting
/// cap --- with the entry count, three ways: the unpruned walk the
/// poll ran until C7b fix round 1, the same walk over the `.git`
/// subtree alone (most of what it visited), and the walk the poll
/// runs now, pruning `.git` and `target` as `lsp.lua` asks. Run with
/// `cargo test --test e7b_review_wire_acceptance -- --ignored --nocapture measure_the_watch_walk`.
#[test]
#[ignore = "a measurement of the watch walk on this repository; run by hand and record"]
fn measure_the_watch_walk_on_this_repository() {
    use pmacs::fs::{walk_tree_blocking, walk_tree_blocking_pruning};
    use pmacs::worker::CancellationToken;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cancel = CancellationToken::new();
    let prune = [".git".to_owned(), "target".to_owned()];
    let report = |name: &str, walk: &dyn Fn() -> usize| {
        let mut us = Vec::new();
        let mut entries = 0;
        for _ in 0..15 {
            let t = Instant::now();
            entries = walk();
            us.push(t.elapsed().as_micros());
        }
        us.sort_unstable();
        eprintln!(
            "MEASURE {name}: {entries} entries; 15 walks min {} us, median {} us, max {} us, sum {} ms per minute at the 4 s cap",
            us[0],
            us[7],
            us[14],
            us.iter().sum::<u128>() / 1000
        );
    };
    report("unpruned walk of the root", &|| {
        walk_tree_blocking(&root, &cancel)
            .expect("the walk")
            .entries
            .len()
    });
    let git = root.join(".git");
    report("walk of .git alone", &|| {
        walk_tree_blocking(&git, &cancel)
            .expect("the walk")
            .entries
            .len()
    });
    report("the poll's walk now (.git and target pruned)", &|| {
        walk_tree_blocking_pruning(&root, &cancel, &prune)
            .expect("the walk")
            .entries
            .len()
    });
}
