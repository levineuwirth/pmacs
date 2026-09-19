// tests/e7c_check_acceptance.rs --- E7c.2, a config that checks.

//! The shipped rust-analyzer config until E7c.2 was `cargo.allFeatures
//! = true` with the pre-2023 `checkOnSave = { command }` shape: the
//! first cannot build a workspace whose features exclude each other
//! (this one's Lua bindings, #281), the second is refused on every
//! start. The default is now rust-analyzer's own --- `checkOnSave =
//! true`, `check.command = "check"`, the workspace's default features
//! --- and `init_options` may be a function of the project root, so
//! one `init.lua` can give each project its own check.
//!
//! * The shipped default is pinned as data: no `cargo.allFeatures`, a
//!   boolean `checkOnSave`, the command under `check`.
//! * A function-valued `init_options` is called with the resolved
//!   project root at spawn and its table is what reaches the server's
//!   `initialize`, read back from the fake's init sink.
//! * One `#[ignore]`d measurement against the real rust-analyzer on a
//!   copy of this repository (`PMACS_E7C_MEASURE_ROOT`): warm, append
//!   a lifetime error and a use-after-move --- errors rust-analyzer's
//!   native analysis does not report --- save, and print when the
//!   check's diagnostics land, from which source, at which position,
//!   and what `*diagnostics*` shows; then remove them, save, and print
//!   the clear. Run by hand and recorded in the phase's handoff.

use std::path::PathBuf;
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

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7c-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
        .map(|seg| seg.text)
}

/// The shipped rust config, as data: rust-analyzer's own check over the
/// workspace's default features, the boolean `checkOnSave`, the
/// command under `check`, and no `cargo.allFeatures`. Bitten by
/// restoring any of the three lines E7c.2 replaced.
#[test]
fn e7c_2_the_shipped_rust_config_checks_with_the_default_features() {
    let s = EditorState::new_with_roots(&iso::roots());
    let (all_features, check_on_save, command, cargo_is_nil): (
        Option<bool>,
        Option<bool>,
        Option<String>,
        bool,
    ) = eval(
        &s,
        "local o = pmacs.lsp.config.rust.init_options
         local all = o.cargo and o.cargo.allFeatures or nil
         local cos = (type(o.checkOnSave) == 'boolean') and o.checkOnSave or nil
         local cmd = o.check and o.check.command or nil
         return all, cos, cmd, o.cargo == nil",
    );
    assert_eq!(all_features, None, "no cargo.allFeatures");
    assert!(
        cargo_is_nil,
        "no cargo table at all: the default features are cargo's"
    );
    assert_eq!(check_on_save, Some(true), "checkOnSave is a boolean, true");
    assert_eq!(
        command.as_deref(),
        Some("check"),
        "the command lives under check"
    );
}

/// A function-valued `init_options` is called with the resolved root
/// at spawn and its table is what the server's `initialize` carries.
/// Bitten by dropping the `type(init_options) == "function"` branch
/// in `ensure_server` (the sink then holds `null`, `lua_to_json` of a
/// function).
#[test]
fn e7c_2_init_options_as_a_function_of_the_root_reach_the_server() {
    let dir = temp_dir("fn");
    // A project root the marker walk finds: `Cargo.toml` at the top,
    // the file under `src/`.
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"e7c_probe\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src").join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let sink = dir.join("init.json");
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!(
            "_G.__e7c_roots = {{}}
             pmacs.lsp.config.rust = {{
               command = {:?},
               env = {{ PMACS_FAKE_LSP_INIT_SINK = {:?} }},
               init_options = function(root)
                 _G.__e7c_roots[#_G.__e7c_roots + 1] = tostring(root)
                 return {{ checkOnSave = true, check = {{ command = 'clippy' }},
                          cargo = {{ features = {{ 'from-' .. tostring(root):match('[^/]+$') }} }} }}
               end,
             }}",
            fake_lsp_path(),
            sink.display().to_string()
        ),
    );
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    let roots: Vec<String> = eval(&s, "return _G.__e7c_roots");
    let root_name = dir.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(roots.len(), 1, "called once per spawn: {roots:?}");
    assert!(
        roots[0].ends_with(&root_name),
        "called with the project root, not the file: {roots:?}"
    );
    let sent: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sink).unwrap()).unwrap();
    assert_eq!(sent["checkOnSave"], serde_json::json!(true));
    assert_eq!(sent["check"]["command"], serde_json::json!("clippy"));
    assert_eq!(
        sent["cargo"]["features"],
        serde_json::json!([format!("from-{root_name}")]),
        "the table the function returned for this root is what was sent: {sent}"
    );
    assert_eq!(lsp_segment(&s).as_deref(), Some("LSP:ready"));
}

/// A function that raises leaves the server spawned without init
/// options and puts one line in `*errors*`, rather than no server.
#[test]
fn e7c_2_an_init_options_function_that_raises_is_reported_and_the_server_still_spawns() {
    let dir = temp_dir("raise");
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let sink = dir.join("init.json");
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = {:?},
               env = {{ PMACS_FAKE_LSP_INIT_SINK = {:?} }},
               init_options = function(root) error('no options for ' .. tostring(root)) end,
             }}",
            fake_lsp_path(),
            sink.display().to_string()
        ),
    );
    let before = s.lua_host.errors_buffer_text();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(
        pump_lua_flag(&mut s, INITIALIZED, 10),
        "the server spawned anyway"
    );
    let sent = std::fs::read_to_string(&sink).unwrap();
    assert_eq!(sent, "null", "no init options reached the server");
    let after = s.lua_host.errors_buffer_text();
    let new_lines: Vec<&str> = after[before.len()..].lines().collect();
    assert!(
        new_lines
            .iter()
            .any(|l| l.contains("init_options for rust raised") && l.contains("no options for")),
        "*errors* names the function's failure: {new_lines:?}"
    );
}

// ---------------------------------------------------------------------------
// The measurement, by hand
// ---------------------------------------------------------------------------

/// Open `src/lsp.rs` of the repository at `PMACS_E7C_MEASURE_ROOT` (a
/// copy; the round edits it) with the shipped rust config and
/// rust-analyzer behind a wire tee, and wait for the handshake.
fn lsp_rs_with_rust_analyzer(tag: &str) -> (EditorState, PathBuf, support::wire_tee::Capture) {
    let manifest = std::env::var_os("PMACS_E7C_MEASURE_ROOT")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let target = manifest.join("src").join("lsp.rs");
    let dir = temp_dir(tag);
    let cap = support::wire_tee::install(&dir, "rust-analyzer");
    let s = EditorState::new_with_roots(&iso::roots());
    // A frame, so the bottom panel `*diagnostics*` opens in has rows
    // to take and the focus it takes is observable.
    s.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir));
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust.command = {:?}",
            cap.wrapper.display().to_string()
        ),
    );
    // `PMACS_E7C_MEASURE_TARGET` is the cargo target directory the
    // check builds under (the shipped config inherits the editor's
    // environment, as the deployed daemon does), so a measurement can
    // be warm without touching the owner's build directory.
    if let Some(target_dir) = std::env::var_os("PMACS_E7C_MEASURE_TARGET") {
        exec(
            &s,
            &format!(
                "pmacs.lsp.config.rust.env = {{ CARGO_TARGET_DIR = {:?} }}",
                target_dir.to_string_lossy()
            ),
        );
    }
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
    (s, target, cap)
}

/// Tick for `secs`, logging every frame on the wire with the
/// millisecond it was first seen (`>` client to server, `<` the
/// reverse) and the label's transitions, and stop early once `until`
/// holds.
fn watch_wire(
    s: &mut EditorState,
    cap: &support::wire_tee::Capture,
    secs: u64,
    until: impl Fn(&EditorState) -> bool,
) -> Vec<String> {
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(secs);
    let (mut to, mut from) = (
        support::wire_tee::Tap::new(&cap.to_server),
        support::wire_tee::Tap::new(&cap.from_server),
    );
    to.new_frames();
    from.new_frames();
    let mut log = Vec::new();
    let mut last_label = String::new();
    while Instant::now() < deadline {
        tick(s);
        let ms = t0.elapsed().as_millis();
        for f in to.new_frames() {
            log.push(format!("{ms} > {}", support::wire_tee::describe(&f)));
        }
        for f in from.new_frames() {
            log.push(format!("{ms} < {}", support::wire_tee::describe(&f)));
        }
        let label = lsp_segment(s).unwrap_or_default();
        if label != last_label {
            log.push(format!("{ms} = {label}"));
            last_label = label;
        }
        if until(s) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    log
}

/// Tick until the label has read exactly `LSP:ready` for `quiet`, or
/// `secs` pass; the transitions seen, with their millisecond.
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

/// Every diagnostic the store holds for `uri`, one line each:
/// `<source> <severity> <line>:<col>-<end_line>:<end_col> <message>`.
fn diagnostics_of(s: &EditorState, uri: &str) -> Vec<String> {
    eval(
        s,
        &format!(
            "local out = {{}}
             for _, d in ipairs(pmacs.diag.list({uri:?})) do
               out[#out + 1] = string.format('%s %s %d:%d-%d:%d %s', tostring(d.source),
                 tostring(d.severity), d.start_line, d.start_col, d.end_line or 0, d.end_col or 0,
                 ((d.message or ''):gsub('\\n.*$', '')))
             end
             return out"
        ),
    )
}

/// Tick until a diagnostic for `uri` whose source is not the server's
/// own analysis (`rust-analyzer`) is in the store, or `secs` pass.
fn wait_check_diagnostic(
    s: &mut EditorState,
    uri: &str,
    secs: u64,
    want: bool,
) -> (bool, Option<u128>, Vec<(u128, String)>) {
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(secs);
    let mut trace: Vec<(u128, String)> = Vec::new();
    let mut first: Option<u128> = None;
    loop {
        tick(s);
        let text = lsp_segment(s).unwrap_or_default();
        if trace.last().map(|(_, t)| t) != Some(&text) {
            trace.push((t0.elapsed().as_millis(), text.clone()));
        }
        let has = diagnostics_of(s, uri)
            .iter()
            .any(|d| !d.starts_with("rust-analyzer "));
        if has == want {
            first.get_or_insert(t0.elapsed().as_millis());
            if text == "LSP:ready" {
                return (true, first, trace);
            }
        }
        if Instant::now() >= deadline {
            return (false, first, trace);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The text a diagnostic's range covers in `text`, for the record's
/// "with its position right": `<line>:<col>-<end_line>:<end_col>`
/// parsed from a `diagnostics_of` line, the covered bytes quoted.
fn covered(text: &str, line: &str) -> String {
    let range = line.split(' ').nth(2).unwrap_or("");
    let parse = |s: &str| -> Option<(usize, usize)> {
        let (l, c) = s.split_once(':')?;
        Some((l.parse().ok()?, c.parse().ok()?))
    };
    let Some((start, end)) = range.split_once('-') else {
        return "?".into();
    };
    let (Some((sl, sc)), Some((el, ec))) = (parse(start), parse(end)) else {
        return "?".into();
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(l) = lines.get(sl) else {
        return "<line past the end>".into();
    };
    if sl == el {
        format!("{:?}", &l[sc.min(l.len())..ec.min(l.len())])
    } else {
        format!("{:?}...", &l[sc.min(l.len())..])
    }
}

const PROBE: &str = "\nfn e7c_probe_lifetime<'a>(x: &'a str, y: &str) -> &'a str {\n    let _ = x;\n    y\n}\n\nfn e7c_probe_move() {\n    let s = String::new();\n    let t = s;\n    let _ = s.len();\n    let _ = t;\n}\n\nfn e7c_probe_types() {\n    let _z: i32 = 1.5;\n}\n";

/// E7c.2's witness against the real server on this workspace: warm,
/// then two errors rust-analyzer's native analysis does not report ---
/// a lifetime error (E0621) and a use after move (E0382) --- and one
/// both report (a mismatched type, E0308), so the record can say how
/// the two sources show side by side (E7c.4), appended
/// through the production dispatch and saved; the check's diagnostics
/// land in the store with their source and position, and
/// `*diagnostics*` lists them; then the probe is deleted and saved and
/// the diagnostics clear. Printed as it goes. Run with
/// `PMACS_E7C_MEASURE_ROOT=<copy> cargo test --test e7c_check_acceptance -- --ignored --nocapture measure_the_check`.
#[test]
#[ignore = "a measurement against rust-analyzer on a copy of this repository; run by hand and record"]
#[allow(
    clippy::too_many_lines,
    reason = "one linear measurement session, printed as it goes"
)]
fn measure_the_check_on_this_workspace() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    assert!(
        std::env::var_os("PMACS_E7C_MEASURE_ROOT").is_some(),
        "point PMACS_E7C_MEASURE_ROOT at a copy of the repository; the round edits src/lsp.rs"
    );
    let (mut s, target, cap) = lsp_rs_with_rust_analyzer("measure");
    let original = std::fs::read_to_string(&target).unwrap();
    let uri: String = eval(&s, "return pmacs.lsp.active_attachment().uri");
    let t0 = Instant::now();
    let trace = wait_warm(&mut s, 900, Duration::from_secs(3));
    eprintln!(
        "MEASURE warm after {} ms; label transitions: {trace:?}",
        t0.elapsed().as_millis()
    );
    let clean = diagnostics_of(&s, &uri);
    eprintln!("MEASURE diagnostics on the clean tree for src/lsp.rs: {clean:?}");
    assert!(
        clean.is_empty(),
        "the copy is not clean; restore it before measuring: {clean:?}"
    );
    let all_uris: Vec<String> = eval(&s, "return pmacs.diag.uris()");
    let totals: (u32, u32) = eval(
        &s,
        "local t = pmacs.diag.totals() return t.error, t.warning",
    );
    eprintln!(
        "MEASURE diagnostics on the clean tree, every uri: {} uris, {} errors, {} warnings: {all_uris:?}",
        all_uris.len(),
        totals.0,
        totals.1
    );

    // The probe, typed at the end through the production dispatch and
    // saved; the after-edit hook flushes the didChange before the save.
    let probe_line = original.matches('\n').count() as u32 + 1;
    exec(
        &s,
        "pmacs.editor.goto_byte(pmacs.window.buffer():len())
         _G.__e7c_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7c_saved = true end)",
    );
    for ch in PROBE.chars() {
        press(
            &mut s,
            if ch == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(ch)
            },
        );
    }
    // Auto-indent and auto-pair may have shaped the typed probe; the
    // file is what the check reads, so it is printed.
    let t_save = Instant::now();
    exec(&s, "pmacs.command.invoke('buffer.save')");
    assert!(pump_lua_flag(&mut s, "_G.__e7c_saved", 10), "saved");
    // The save's own wire, before anything else happens: the negotiated
    // save capability, whether the notification went out, and the
    // capture files, kept for reading.
    let negotiated: Option<bool> = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment()
         return rec and pmacs.lsp.save_negotiated(rec.server) or nil",
    );
    let sent_now = support::wire_tee::frames(&cap.to_server)
        .iter()
        .filter(|f| support::wire_tee::method_of(f) == Some("textDocument/didSave"))
        .count();
    eprintln!(
        "MEASURE after the save command: save negotiated {negotiated:?}, didSave frames in the capture so far {sent_now}; captures at {} and {}",
        cap.to_server.display(),
        cap.from_server.display()
    );
    let on_disk = std::fs::read_to_string(&target).unwrap();
    eprintln!(
        "MEASURE saved; the tail of the file as written:\n{}",
        &on_disk[original.len().min(on_disk.len())..]
    );
    // E7c.3's witness on the real server: before the check's answer
    // lands, type a line at the top of the file, so every position the
    // check reports is one line and thirteen bytes stale by the time
    // it arrives; the store must carry it to the text as it is now.
    // `PMACS_E7C_TYPE_AFTER_MS` delays the typing (0: at once); the
    // wire around the save is printed either way, since a save
    // followed at once by typing was seen to run no check at all.
    let type_after: u64 = std::env::var("PMACS_E7C_TYPE_AFTER_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut wire = Vec::new();
    if type_after > 0 {
        let until = Instant::now() + Duration::from_millis(type_after);
        wire.extend(watch_wire(&mut s, &cap, 60, |_| Instant::now() >= until));
    }
    exec(&s, "pmacs.editor.goto_byte(0)");
    for ch in "//0123456789\n".chars() {
        press(
            &mut s,
            if ch == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(ch)
            },
        );
    }
    let typed_at = t_save.elapsed().as_millis();
    wire.push(format!("{typed_at} = (typed a line at the top)"));
    // Eight seconds of wire after the typing, or until the check's
    // diagnostics are in the store.
    wire.extend(watch_wire(&mut s, &cap, 8, |s| {
        diagnostics_of(s, &uri)
            .iter()
            .any(|d| !d.starts_with("rust-analyzer "))
    }));
    eprintln!(
        "MEASURE the wire from the save on (ms after the typing's watch began; the first block, when present, precedes the typing):"
    );
    for line in &wire {
        eprintln!("MEASURE   {line}");
    }
    let (landed, first, trace) = wait_check_diagnostic(&mut s, &uri, 90, true);
    eprintln!(
        "MEASURE a line typed at the top {typed_at} ms after the save; check diagnostic {}, the label ready again {} ms after the save; label transitions: {trace:?}",
        first.map_or("never landed".to_owned(), |ms| format!(
            "first in the store {ms} ms"
        )),
        t_save.elapsed().as_millis()
    );
    let found = diagnostics_of(&s, &uri);
    let in_buffer: String = eval(
        &s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    assert!(
        in_buffer.starts_with("//0123456789\n"),
        "the typed line is at the top: {:?}",
        &in_buffer[..40.min(in_buffer.len())]
    );
    eprintln!(
        "MEASURE diagnostics for src/lsp.rs after the save (probe starts at line {} in the buffer as it is now), each with the text its range covers in the BUFFER, the typed line above it:",
        probe_line + 1
    );
    for d in &found {
        eprintln!("MEASURE   {d:?} -> {}", covered(&in_buffer, d));
    }
    let errors: Vec<&String> = found
        .iter()
        .filter(|d| d.starts_with("rustc error"))
        .collect();
    assert_eq!(errors.len(), 3, "three rustc errors: {found:?}");
    assert!(
        errors.iter().all(|d| {
            let text = covered(&in_buffer, d);
            text == "\"y\"" || text == "\"s\"" || text == "\"1.5\""
        }),
        "each error covers the expression it names, in the buffer as it is now: {errors:?}"
    );
    let native: Vec<&String> = found
        .iter()
        .filter(|d| d.starts_with("rust-analyzer "))
        .collect();
    eprintln!(
        "MEASURE the server's own analysis reports {} of these beside rustc's: {native:?}",
        native.len()
    );
    // What `*diagnostics*` shows.
    exec(&s, "pmacs.command.invoke('lsp.diagnostics')");
    for _ in 0..20 {
        tick(&mut s);
    }
    let panel: String = eval(
        &s,
        "for _, b in ipairs(pmacs.buffer.list()) do
           if b:name() == '*diagnostics*' then return b:slice(0, b:len()) end
         end
         return '<no panel>'",
    );
    eprintln!("MEASURE *diagnostics*:\n{panel}");
    // RET on the panel's first error visits it where it is now (E7c.4):
    // the panel opens on the first row under the buffer's label; `n`
    // walks the rows, the errors are the rows whose text says `error`.
    let mut visited = Vec::new();
    for _ in 0..12 {
        let row: String = eval(
            &s,
            "local b = pmacs.window.buffer()
             local l = pmacs.editor.cursor_line()
             local text = b:slice(0, b:len())
             local lines = {}
             for ln in (text .. '\\n'):gmatch('(.-)\\n') do lines[#lines + 1] = ln end
             return lines[l + 1] or ''",
        );
        if row.contains("  error  ") {
            press(&mut s, KeyCode::Enter);
            for _ in 0..5 {
                tick(&mut s);
            }
            let (name, line, col, under): (String, u32, u32, String) = eval(
                &s,
                "local b = pmacs.window.buffer()
                 local at = pmacs.editor.cursor()
                 return b:name(), pmacs.editor.cursor_line(), pmacs.editor.cursor_col(), b:slice(at, math.min(at + 3, b:len()))",
            );
            visited.push(format!("{row:?} -> {name} {line}:{col} at {under:?}"));
            break;
        }
        press(&mut s, KeyCode::Char('n'));
    }
    eprintln!("MEASURE RET from the panel: {visited:?}");
    assert_eq!(visited.len(), 1, "RET visited one error");
    assert!(
        visited[0].contains("at \"y")
            || visited[0].contains("at \"1.5")
            || visited[0].contains("at \"s"),
        "the cursor landed on the expression the error names: {visited:?}"
    );
    // Back to the document: the panel took the focus.
    exec(
        &s,
        &format!(
            "pmacs.window.display_file({:?}, {{ select = true }})",
            target.display().to_string()
        ),
    );
    // Remove the typed line and the probe as a user would, one
    // Backspace per byte (both are ASCII), save, and wait for the clear.
    exec(&s, "pmacs.editor.goto_byte(13)");
    for _ in 0..13 {
        press(&mut s, KeyCode::Backspace);
    }
    exec(
        &s,
        "pmacs.editor.goto_byte(pmacs.window.buffer():len())
         _G.__e7c_saved = false",
    );
    for _ in original.len()..on_disk.len() {
        press(&mut s, KeyCode::Backspace);
    }
    let reverted: String = eval(
        &s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    assert_eq!(reverted, original, "the buffer holds the original again");
    let t_save2 = Instant::now();
    exec(&s, "pmacs.command.invoke('buffer.save')");
    assert!(
        pump_lua_flag(&mut s, "_G.__e7c_saved", 10),
        "saved the original"
    );
    let (cleared, first, trace) = wait_check_diagnostic(&mut s, &uri, 600, false);
    eprintln!(
        "MEASURE after the revert's save: cleared={cleared}, the store empty of check diagnostics first at {first:?} ms, the label ready again {} ms after the save; label transitions: {trace:?}; diagnostics now {:?}",
        t_save2.elapsed().as_millis(),
        diagnostics_of(&s, &uri)
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        original,
        "the copy is as it was"
    );
    assert!(landed, "the check's diagnostic landed");
    assert!(cleared, "and cleared once the probe was gone");
}
