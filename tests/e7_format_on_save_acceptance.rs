// tests/e7_format_on_save_acceptance.rs --- E7.3: format on save.

//! `lsp.format-on-save` asks the buffer's server for
//! `textDocument/formatting` inside `buffer.before-save`, waits at most
//! `lsp.format-on-save.timeout-ms` for the answer, applies the edits,
//! and lets the save proceed. A formatter that answers late, answers
//! with an error, or is not running leaves the buffer unformatted, the
//! save proceeds, and the user is told --- on the status line beside
//! "saved", and in `*errors*` --- and nothing is applied afterwards.
//!
//! The bound was measured before it was chosen: the `#[ignore]`d
//! measurement at the bottom times `textDocument/formatting` on this
//! repository's own `src/editor.rs` against rust-analyzer, cold (the
//! first request after the handshake) and warm.

use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn errors_text(s: &EditorState) -> String {
    s.lua_host.errors_buffer_text()
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7fmt-{tag}-{}", std::process::id()));
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

/// The fake's formatting reply, fixed: delete columns 0--4 of line 0
/// and insert `;` at line 3 column 7. This fixture is shaped so both
/// land: four leading spaces on line 0, and a line 3 at least seven
/// columns long.
const BODY: &str = "    fn main() {\n    let a = 1;\n    let b = 2;\n    let c = a + b\n}\n";
const FORMATTED: &str = "fn main() {\n    let a = 1;\n    let b = 2;\n    let; c = a + b\n}\n";

/// An editor with isolated roots, the rust server pointed at the fake
/// in `mode` with `extra_env` in its environment, visiting `dir/a.rs`
/// holding `BODY`, returning once the fake has initialized.
fn fake_editor(dir: &Path, mode: &str, extra_env: &[(&str, &str)]) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let fake = fake_lsp_path();
    let mut env = format!("PMACS_FAKE_LSP_MODE = '{mode}'");
    for (k, v) in extra_env {
        env.push_str(&format!(", {k} = '{v}'"));
    }
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{ {env} }},
             }}"
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, BODY).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    s
}

/// The path of the visited file as the save message prints it.
fn saved_path(dir: &Path) -> String {
    dir.join("a.rs").display().to_string()
}

fn disk(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("a.rs")).expect("read a.rs")
}

/// Run `buffer.save` and return how long it took.
fn save(s: &EditorState) -> Duration {
    let t0 = Instant::now();
    exec(s, "pmacs.command.invoke('buffer.save')");
    t0.elapsed()
}

// ---------------------------------------------------------------------------
// The knob
// ---------------------------------------------------------------------------

/// Off by default: a save sends no formatting request and writes the
/// buffer's bytes as they are.
#[test]
fn e7_3_off_by_default_a_save_formats_nothing() {
    let dir = temp_dir("off");
    let mut s = fake_editor(&dir, "", &[]);
    assert!(!eval::<bool>(&s, "return pmacs.config.get('lsp.format-on-save')"));
    assert_eq!(
        eval::<i64>(&s, "return pmacs.config.get('lsp.format-on-save.timeout-ms')"),
        1000,
        "the measured default"
    );
    exec(&s, "pmacs.window.buffer():insert(0, '// touched\\n')");
    save(&s);
    tick(&mut s);
    assert_eq!(disk(&dir), format!("// touched\n{BODY}"), "written as typed");
    assert_eq!(status(&s), format!("saved {}", saved_path(&dir)));
    let n: i64 = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment(); return #pmacs.formatting.edits(rec.server, rec.uri)",
    );
    assert_eq!(n, 0, "no formatting response was ever absorbed");
}

/// On: the server's edits land in the buffer before the bytes reach
/// the disk, the status line says so beside "saved", and `*errors*` is
/// untouched. The knob is read against the buffer, so a buffer-local
/// setting is honored with the global off.
#[test]
fn e7_3_on_the_save_writes_the_formatted_text() {
    let dir = temp_dir("on");
    let s = fake_editor(&dir, "", &[]);
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    exec(&s, "pmacs.editor.goto_byte(0)");
    let took = save(&s);
    assert_eq!(disk(&dir), FORMATTED, "the disk holds the formatted text");
    assert_eq!(buffer_text(&s), FORMATTED, "and so does the buffer");
    assert!(
        !eval::<bool>(&s, "return pmacs.describe.buffer(pmacs.window.buffer()).modified"),
        "the buffer is clean after its save"
    );
    assert_eq!(
        status(&s),
        format!("saved {} --- formatted (2 edits)", saved_path(&dir))
    );
    assert!(
        !errors_text(&s).contains("format-on-save"),
        "a formatted save is no error: {:?}",
        errors_text(&s)
    );
    assert!(took < Duration::from_secs(1), "the fake answers at once: {took:?}");

    // Buffer-local, global off.
    let dir2 = temp_dir("local");
    let s2 = fake_editor(&dir2, "", &[]);
    exec(&s2, "pmacs.config.set_local(pmacs.window.buffer(), 'lsp.format-on-save', true)");
    assert!(!eval::<bool>(&s2, "return pmacs.config.get('lsp.format-on-save')"));
    save(&s2);
    assert_eq!(disk(&dir2), FORMATTED, "the buffer-local setting formats");
}

/// Trim-on-save runs first and format-on-save sees the trimmed text.
#[test]
fn e7_3_format_runs_after_trim() {
    let dir = temp_dir("trim");
    let s = fake_editor(&dir, "", &[]);
    exec(&s, "pmacs.config.set('editing.trim-on-save', true)");
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    // Trailing blanks on line 1, which trim removes and which do not
    // move the fake's fixed edit positions.
    exec(
        &s,
        "local b = pmacs.window.buffer(); local at = ('    fn main() {\\n    let a = 1;'):len(); b:insert(at, '   ')",
    );
    assert!(buffer_text(&s).contains("let a = 1;   \n"), "positive control: blanks inserted");
    save(&s);
    assert_eq!(disk(&dir), FORMATTED, "trimmed, then formatted");
}

// ---------------------------------------------------------------------------
// The bound
// ---------------------------------------------------------------------------

/// A formatter that never answers: the save proceeds unformatted once
/// the bound has passed, in about the bound and not a moment near a
/// hang; the status line and `*errors*` both say so.
#[test]
fn e7_3_a_stalled_formatter_saves_unformatted_within_the_bound() {
    let dir = temp_dir("stall");
    let mut s = fake_editor(&dir, "silent", &[]);
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    exec(&s, "pmacs.config.set('lsp.format-on-save.timeout-ms', 300)");
    let took = save(&s);
    assert!(
        took >= Duration::from_millis(300) && took < Duration::from_millis(1300),
        "the save waits the bound and no longer: {took:?}"
    );
    assert_eq!(disk(&dir), BODY, "saved unformatted");
    assert_eq!(buffer_text(&s), BODY);
    assert_eq!(
        status(&s),
        format!(
            "saved {} --- unformatted: the language server did not answer within 300 ms",
            saved_path(&dir)
        )
    );
    assert!(
        errors_text(&s)
            .contains("format-on-save: unformatted: the language server did not answer within 300 ms"),
        "*errors* keeps the line: {:?}",
        errors_text(&s)
    );
    // The server is still up and still silent; nothing changes later.
    for _ in 0..30 {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(buffer_text(&s), BODY, "nothing arrives later");
    assert!(!eval::<bool>(&s, "return pmacs.describe.buffer(pmacs.window.buffer()).modified"));
}

/// A formatter that answers AFTER the bound: the save proceeds
/// unformatted at the bound, and when the answer lands it is not
/// applied --- not to the buffer, not to the disk.
#[test]
fn e7_3_a_late_answer_is_never_applied() {
    let dir = temp_dir("late");
    let mut s = fake_editor(&dir, "", &[("PMACS_FAKE_LSP_FORMAT_HOLD_MS", "900")]);
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    exec(&s, "pmacs.config.set('lsp.format-on-save.timeout-ms', 200)");
    let took = save(&s);
    assert!(
        took >= Duration::from_millis(200) && took < Duration::from_millis(900),
        "the save gives up at the bound, before the answer: {took:?}"
    );
    assert_eq!(disk(&dir), BODY, "saved unformatted");
    assert!(status(&s).contains("did not answer within 200 ms"), "status: {:?}", status(&s));
    // Let the held answer arrive and be processed.
    let deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < deadline {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(buffer_text(&s), BODY, "the late answer touched nothing");
    assert!(
        !eval::<bool>(&s, "return pmacs.describe.buffer(pmacs.window.buffer()).modified"),
        "and left the buffer clean"
    );
    assert_eq!(disk(&dir), BODY);
    // The next save, with the server now answering promptly, formats.
    exec(&s, "pmacs.config.set('lsp.format-on-save.timeout-ms', 3000)");
    exec(&s, "local b = pmacs.window.buffer(); b:insert(0, ' '); b:delete(0, 1)");
    save(&s);
    assert_eq!(disk(&dir), FORMATTED, "a later save with time to spare formats");
}

/// A formatter that answers with an error: unformatted, said.
#[test]
fn e7_3_an_erroring_formatter_saves_unformatted_and_says_so() {
    let dir = temp_dir("error");
    let s = fake_editor(&dir, "error", &[]);
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    let took = save(&s);
    assert!(took < Duration::from_millis(900), "an error answer does not wait: {took:?}");
    assert_eq!(disk(&dir), BODY);
    assert!(
        status(&s).starts_with(&format!(
            "saved {} --- unformatted: formatting failed",
            saved_path(&dir)
        )),
        "status: {:?}",
        status(&s)
    );
    assert!(errors_text(&s).contains("format-on-save: unformatted: formatting failed"));
}

/// A server that is not running is said too; a buffer no server
/// serves saves in silence.
#[test]
fn e7_3_no_server_is_said_only_where_one_is_expected() {
    let dir = temp_dir("dead");
    let mut s = fake_editor(&dir, "", &[]);
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    exec(&s, "pmacs.lsp.stop(pmacs.lsp.active_attachment().server)");
    let stopped = "(function() \
       for _,r in ipairs(pmacs.lsp.list()) do \
         local k = r.state and r.state.kind \
         if k=='initialized' or k=='shutting-down' or k=='starting' or k=='initializing' then return false end \
       end \
       return true \
     end)()";
    assert!(pump_lua_flag(&mut s, stopped, 10), "the server stops");
    save(&s);
    assert_eq!(disk(&dir), BODY);
    assert_eq!(
        status(&s),
        format!(
            "saved {} --- unformatted: the language server is not running",
            saved_path(&dir)
        )
    );

    // A plain text buffer: no attachment, no notice.
    let txt = dir.join("notes.txt");
    std::fs::write(&txt, "notes\n").unwrap();
    exec(&s, &format!("pmacs.buffer.find_or_open({:?})", txt.display().to_string()));
    exec(&s, "pmacs.window.buffer():insert(0, 'more ')");
    save(&s);
    assert_eq!(status(&s), format!("saved {}", txt.display()));
    assert_eq!(std::fs::read_to_string(&txt).unwrap(), "more notes\n");
}

// ---------------------------------------------------------------------------
// The measurement (run by hand: cargo test --test e7_format_on_save_acceptance
// -- --ignored --nocapture e7_3_measure)
// ---------------------------------------------------------------------------

/// `textDocument/formatting` on this repository's `src/editor.rs`
/// against rust-analyzer: the first request right after the handshake
/// (cold), then ten more (warm), each timed from the request to the
/// settled handle through the same tick loop the editor runs. Prints
/// the samples; asserts nothing about them. rust-analyzer formats by
/// running rustfmt on the document, so what this measures is rustfmt
/// on a 14k-line file plus the round trip.
#[test]
#[ignore = "a measurement against rust-analyzer on src/editor.rs; run by hand and record"]
fn e7_3_measure_formatting_latency_on_editor_rs() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = manifest.join("src").join("editor.rs");
    let dir = temp_dir("measure");
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            target.display().to_string()
        ),
    );
    let mut s = s;
    let handshake = Instant::now();
    assert!(pump_lua_flag(&mut s, INITIALIZED, 120), "rust-analyzer init");
    eprintln!(
        "MEASURE handshake answered {} ms after the open",
        handshake.elapsed().as_millis()
    );
    let mut samples = Vec::new();
    for i in 0..11 {
        exec(
            &s,
            "_G.FMT_DONE = nil
             local rec = pmacs.lsp.attachment_for_request()
             pmacs.formatting.clear(rec.server, rec.uri)
             pmacs.async(function()
               local ok, err = pcall(function()
                 pmacs.lsp.request_formatting(rec.server, rec.uri, 4, true):await()
               end)
               _G.FMT_DONE = ok and 'ok' or ('err:' .. tostring(err))
             end)",
        );
        let t0 = Instant::now();
        assert!(
            pump_lua_flag(&mut s, "_G.FMT_DONE ~= nil", 120),
            "formatting request {i} answered"
        );
        let ms = t0.elapsed().as_millis();
        let outcome: String = eval(&s, "return _G.FMT_DONE");
        let edits: i64 = eval(
            &s,
            "local rec = pmacs.lsp.active_attachment(); return #pmacs.formatting.edits(rec.server, rec.uri)",
        );
        eprintln!(
            "MEASURE {} formatting: {ms} ms, {outcome}, {edits} edits",
            if i == 0 { "cold" } else { "warm" }
        );
        samples.push(ms);
    }
    let mut warm: Vec<u128> = samples[1..].to_vec();
    warm.sort_unstable();
    eprintln!(
        "MEASURE cold {} ms; warm p50 {} ms, max {} ms over {} samples",
        samples[0],
        warm[warm.len() / 2],
        warm[warm.len() - 1],
        warm.len()
    );
}
