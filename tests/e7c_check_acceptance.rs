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
//! * A second `#[ignore]`d measurement (E7c fix round 2): the mode line
//!   frame by frame while typing after a save --- the first keystroke
//!   50 ms after the save, then one every 150 ms for five seconds ---
//!   on both frontends' composition, with the wire beside it, so what
//!   the owner saw change "for the duration of the typing" is named
//!   from the frames.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::{ByteRange, FrontendId, InstanceMessage};
use pmacs::semantic_render::SemanticRenderState;
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
        // The `ready` family carries a fixed slot for the busy suffix
        // (C7c fix round 3); the label is what these rows read.
        .map(|seg| seg.text.trim_end().to_owned())
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

// ---------------------------------------------------------------------------
// The mode line while typing after a save, by hand
// ---------------------------------------------------------------------------

/// The grid frontend's mode line for the document's window, painted
/// through `paint_frame` at the frame geometry the fixture declared
/// (40 x 100): the text of the row carrying the LSP segment.
fn grid_modeline(s: &EditorState) -> String {
    let (rows, cols) = (40u32, 100u32);
    let size = CellSize::new(rows, cols);
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    let text_of = |row: u32| -> String {
        (0..cols)
            .map(|col| match &cells[(row * cols + col) as usize].glyph {
                Glyph::Char(c) => *c,
                _ => ' ',
            })
            .collect()
    };
    // One window in the frame: its mode line is the row above the
    // status row. Read by position, since what is on it is the
    // question: a segment wide enough to push the rest off the row
    // must be seen doing so, not searched for.
    let row = text_of(rows - 2);
    assert!(
        row.contains(":C"),
        "the row above the status row is the mode line (its cursor readout): {row:?}; the frame: {:?}",
        (0..rows).map(text_of).collect::<Vec<_>>()
    );
    row
}

/// What a semantic frontend composes its right group from, as the
/// last `StatuslineSegments` and `StatusFacts` it received say (both
/// are suppressed while unchanged, so the last is carried): the custom
/// right segments in order, then `E:`/`W:` as `compose_status_runs`
/// appends them, then the modified flag and the transient message.
#[derive(Default, Clone, PartialEq, Eq)]
struct WireModeline {
    right: Vec<String>,
    left: Vec<String>,
    errors: u32,
    warnings: u32,
    modified: bool,
    message: Option<String>,
}

impl WireModeline {
    fn absorb(&mut self, frame: &[InstanceMessage]) {
        for m in frame {
            match m {
                InstanceMessage::StatuslineSegments { left, right, .. } => {
                    self.left = left.iter().map(|seg| seg.text.clone()).collect();
                    self.right = right.iter().map(|seg| seg.text.clone()).collect();
                }
                InstanceMessage::StatusFacts {
                    modified,
                    diag_errors,
                    diag_warnings,
                    message,
                    ..
                } => {
                    self.modified = *modified;
                    self.errors = *diag_errors;
                    self.warnings = *diag_warnings;
                    self.message.clone_from(message);
                }
                _ => {}
            }
        }
    }

    fn text(&self) -> String {
        let mut right = self.right.clone();
        if self.errors > 0 {
            right.push(format!("E:{}", self.errors));
        }
        if self.warnings > 0 {
            right.push(format!("W:{}", self.warnings));
        }
        format!(
            "left={:?} right={:?} modified={} message={:?}",
            self.left, right, self.modified, self.message
        )
    }
}

const UNUSED_PROBE: &str = "\nfn e7c_probe_unused() {\n    let unused_e7c = 1;\n}\n";

/// The owner's sequence, driven: with the `unused_e7c` warning on its
/// line from an earlier save, save again and type a comment above the
/// warning --- the first keystroke 50 ms after the save, then one every
/// 150 ms for five seconds --- and print the mode line every time it
/// changes, on the grid (the painted row) and on the wire (the segments
/// and facts a semantic frontend composes its own from), with the
/// wire's frames and each keystroke's millisecond beside them; then
/// keep reading until the label has been `LSP:ready` for three seconds
/// past the typing, so a suffix that outlives the typing is seen going.
/// Then the typed text and the probe are removed, saved, and the clear
/// waited for, so the copy is as it was. Run with
/// `PMACS_E7C_MEASURE_ROOT=<copy> cargo test --test e7c_check_acceptance -- --ignored --nocapture measure_the_modeline`.
#[test]
#[ignore = "a measurement against rust-analyzer on a copy of this repository; run by hand and record"]
#[allow(
    clippy::too_many_lines,
    reason = "one linear measurement session, printed as it goes"
)]
fn measure_the_modeline_while_typing_after_a_save() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    assert!(
        std::env::var_os("PMACS_E7C_MEASURE_ROOT").is_some(),
        "point PMACS_E7C_MEASURE_ROOT at a copy of the repository; the round edits src/lsp.rs"
    );
    let (mut s, target, cap) = lsp_rs_with_rust_analyzer("modeline");
    let original = std::fs::read_to_string(&target).unwrap();
    let uri: String = eval(&s, "return pmacs.lsp.active_attachment().uri");
    let buffer_id = s.core.borrow().active_window().buffer_id;
    // A semantic frontend's view of the top of the file, the forty
    // rows a window shows and not the whole document: the frame's
    // style spans are computed for the viewport, and a viewport of
    // the whole 200 KB file made every frame cost seconds, which
    // starved the keystroke cadence the measurement is about.
    let mut r = SemanticRenderState::new(FrontendId::LOCAL);
    r.set_viewport(
        buffer_id,
        ByteRange {
            start: 0,
            end: 4096,
        },
        0,
    );
    let mut wire_modeline = WireModeline::default();
    let t0 = Instant::now();
    let trace = wait_warm(&mut s, 900, Duration::from_secs(3));
    eprintln!(
        "MODELINE warm after {} ms; label transitions: {trace:?}",
        t0.elapsed().as_millis()
    );
    assert!(
        diagnostics_of(&s, &uri).is_empty(),
        "the copy is not clean; restore it before measuring: {:?}",
        diagnostics_of(&s, &uri)
    );

    // The warning first: the probe typed at the end and saved, its
    // check's warning landed and the label back at ready.
    exec(
        &s,
        "pmacs.editor.goto_byte(pmacs.window.buffer():len())
         _G.__e7c_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7c_saved = true end)",
    );
    for ch in UNUSED_PROBE.chars() {
        press(
            &mut s,
            if ch == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(ch)
            },
        );
    }
    exec(&s, "pmacs.command.invoke('buffer.save')");
    assert!(
        pump_lua_flag(&mut s, "_G.__e7c_saved", 10),
        "saved the probe"
    );
    let with_probe = std::fs::read_to_string(&target).unwrap();
    let (landed, first, trace) = wait_check_diagnostic(&mut s, &uri, 90, true);
    eprintln!(
        "MODELINE the probe's check: landed={landed}, first in the store at {first:?} ms after the save; label transitions: {trace:?}; diagnostics {:?}",
        diagnostics_of(&s, &uri)
    );
    assert!(landed, "the unused_e7c warning landed");
    let settled = wait_warm(&mut s, 60, Duration::from_secs(3));
    eprintln!("MODELINE settled: {settled:?}");
    wire_modeline.absorb(&r.render_frame(&s));
    eprintln!("MODELINE grid before the save: {:?}", grid_modeline(&s));
    eprintln!("MODELINE wire before the save: {}", wire_modeline.text());

    // The sequence. A comment opener at the top so the buffer is
    // modified and the save writes; the save; the first keystroke at
    // 50 ms; then one every 150 ms for five seconds; every frame
    // read on both frontends, logged when either changes.
    // `PMACS_E7C_ACTIVITY=off` turns the activity indicator off for the
    // sequence, so the busy suffix's own effect on the mode line can
    // be read apart from the indicator's.
    if std::env::var("PMACS_E7C_ACTIVITY").as_deref() == Ok("off") {
        exec(&s, "pmacs.config.set('ui.activity-indicator', false)");
        eprintln!("MODELINE the activity indicator is OFF for this run");
    }
    exec(&s, "pmacs.editor.goto_byte(0)\n_G.__e7c_saved = false");
    for ch in "//".chars() {
        press(&mut s, KeyCode::Char(ch));
    }
    let (mut to, mut from) = (
        support::wire_tee::Tap::new(&cap.to_server),
        support::wire_tee::Tap::new(&cap.from_server),
    );
    to.new_frames();
    from.new_frames();
    let t_save = Instant::now();
    exec(&s, "pmacs.command.invoke('buffer.save')");
    let mut log: Vec<String> = Vec::new();
    let ms = |t_save: Instant| t_save.elapsed().as_millis();
    log.push(format!("{} = save command returned", ms(t_save)));
    // Thirty-four keystrokes: the first at 50 ms, then one every 150
    // ms, five seconds of typing at the cadence asked for --- as early
    // as the loop can dispatch each, since a tick on this file costs
    // what it costs and the daemon would take the keystrokes in the
    // same order at the same pace.
    let typing: Vec<char> = " e7c typing a comment above the warning, "
        .chars()
        .take(34)
        .collect();
    let mut next_key_at = Duration::from_millis(50);
    let mut typed = 0usize;
    let mut last_grid = String::new();
    let mut last_wire = String::new();
    let mut keystrokes: Vec<u128> = Vec::new();
    let mut quiet_since: Option<Instant> = None;
    let deadline = t_save + Duration::from_secs(90);
    // The instrument's own cost, so a slipped cadence can be read
    // against it: the slowest tick, grid paint and wire frame.
    let mut slowest = [(0u128, "tick"), (0u128, "grid"), (0u128, "wire")];
    let note = |slot: usize, began: Instant, slowest: &mut [(u128, &str); 3]| {
        let took = began.elapsed().as_millis();
        if took > slowest[slot].0 {
            slowest[slot].0 = took;
        }
        took
    };
    loop {
        let now = t_save.elapsed();
        if typed < typing.len() && now >= next_key_at {
            press(&mut s, KeyCode::Char(typing[typed]));
            typed += 1;
            keystrokes.push(ms(t_save));
            log.push(format!("{} k {:?}", ms(t_save), typing[typed - 1]));
            next_key_at += Duration::from_millis(150);
        }
        let began = Instant::now();
        tick(&mut s);
        let took = note(0, began, &mut slowest);
        if took > 100 {
            log.push(format!("{} = a tick took {took} ms", ms(t_save)));
        }
        for f in to.new_frames() {
            log.push(format!(
                "{} > {}",
                ms(t_save),
                support::wire_tee::describe(&f)
            ));
        }
        for f in from.new_frames() {
            let d = support::wire_tee::describe(&f);
            // The server's answers to the per-keystroke pulls are the
            // bulk of the traffic and say nothing about the mode line.
            if !d.starts_with("<response") {
                log.push(format!("{} < {d}", ms(t_save)));
            }
        }
        let began = Instant::now();
        let grid = grid_modeline(&s);
        let took = note(1, began, &mut slowest);
        if took > 100 {
            log.push(format!("{} = a grid paint took {took} ms", ms(t_save)));
        }
        if grid != last_grid {
            log.push(format!("{} G {grid:?}", ms(t_save)));
            last_grid = grid;
        }
        let began = Instant::now();
        wire_modeline.absorb(&r.render_frame(&s));
        let took = note(2, began, &mut slowest);
        if took > 100 {
            log.push(format!("{} = a wire frame took {took} ms", ms(t_save)));
        }
        let wire = wire_modeline.text();
        if wire != last_wire {
            log.push(format!("{} W {wire}", ms(t_save)));
            last_wire = wire;
        }
        // After the last keystroke: read on until the label has been
        // `LSP:ready` and nothing transient (`⋯`, the activity
        // indicator) has been on either mode line for three seconds,
        // so whatever the typing brought is seen going.
        let label = lsp_segment(&s).unwrap_or_default();
        if typed >= typing.len() {
            if label == "LSP:ready" && !last_grid.contains('⋯') && !last_wire.contains('⋯') {
                let since = *quiet_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(3) {
                    break;
                }
            } else {
                quiet_since = None;
            }
        }
        if Instant::now() >= deadline {
            log.push(format!("{} = deadline", ms(t_save)));
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    eprintln!(
        "MODELINE keystrokes at (ms after the save, {} of them): {keystrokes:?}",
        keystrokes.len()
    );
    eprintln!(
        "MODELINE the frames (ms after the save; k a keystroke, > client to server, < server to client, G the grid's mode line row, W the wire's segments and facts):"
    );
    for line in &log {
        eprintln!("MODELINE   {line}");
    }
    let gaps: Vec<u128> = keystrokes.windows(2).map(|w| w[1] - w[0]).collect();
    eprintln!(
        "MODELINE keystroke gaps: min {:?} max {:?} ms; first at {:?} ms; the instrument's slowest calls: {slowest:?}",
        gaps.iter().min(),
        gaps.iter().max(),
        keystrokes.first()
    );

    // Back to the original: the typed line removed, the probe removed,
    // saved, the clear waited for.
    let in_buffer: String = eval(
        &s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    // What was typed at the top is the buffer's excess over the file
    // as the probe's save wrote it (the probe went through auto-indent,
    // so its length on disk is read, not assumed).
    let typed_len = in_buffer.len().saturating_sub(with_probe.len());
    exec(&s, &format!("pmacs.editor.goto_byte({typed_len})"));
    for _ in 0..typed_len {
        press(&mut s, KeyCode::Backspace);
    }
    exec(&s, "pmacs.editor.goto_byte(pmacs.window.buffer():len())");
    loop {
        let now: String = eval(
            &s,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())",
        );
        if now == original {
            break;
        }
        assert!(
            now.len() > original.len(),
            "overshot the original: {:?}",
            &now[now.len().saturating_sub(60)..]
        );
        press(&mut s, KeyCode::Backspace);
    }
    exec(
        &s,
        "_G.__e7c_saved = false\npmacs.command.invoke('buffer.save')",
    );
    assert!(
        pump_lua_flag(&mut s, "_G.__e7c_saved", 10),
        "saved the original"
    );
    let (cleared, first, trace) = wait_check_diagnostic(&mut s, &uri, 600, false);
    eprintln!(
        "MODELINE after the revert's save: cleared={cleared}, first at {first:?} ms; label transitions: {trace:?}; diagnostics now {:?}",
        diagnostics_of(&s, &uri)
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        original,
        "the copy is as it was"
    );
    assert!(cleared, "and cleared once the probe was gone");
}
