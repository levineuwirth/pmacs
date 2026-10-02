//! E7i.1's witnesses: a parse behind each candidate boundary, through the
//! real daemon (`syntax.lua`'s dispatch, the async runtime, the unit, the
//! settle and install), observed by the daemon's own `init.lua`, which
//! writes what `pmacs.parse._unit_report` says to a file the test reads.
//!
//! The process arm runs in the gate: `cargo build --workspace` builds
//! `pmacs-parse-unit` beside `pmacs`, where the editor finds it. The wasm
//! arm needs `pmacs-parse-unit.wasm` beside `pmacs`, which the gate does
//! not build (`scripts/build-parse-unit-wasm`, wasi-sdk 29.0), so its rows
//! are `#[ignore]` and run by hand with the module copied into the
//! daemon's directory.
//!
//! #301's nested image openers (`fuzz/accepted/301-nested-openers-98.input`)
//! never return from `ts_parser__condense_stack` inside a deadline, and
//! #296's underscore paragraph grows inside `ts_parser__accept`; neither
//! reaches tree-sitter's progress callback, so in-process nothing stops
//! them. Behind a boundary the time and memory limits stop the unit.

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::daemon::TestDaemon;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// #296's paragraph as the issue generates it, `lines` lines long.
fn underscores(lines: usize) -> String {
    let line = format!("{}a `_`_", "_".repeat(582));
    vec![line; lines].join("\n") + "\n"
}

/// A daemon in `mode` whose init.lua opens `victim` (markdown) and, after
/// it, `typed` (a small Rust file), then reports every 50 ms to
/// `<dir>/report.txt` until the victim's unit has died once and the Rust
/// buffer has a settled parse from its own unit.
fn daemon(mode: &str, victim: &str, settings: &str) -> (TestDaemon, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let victim_path = dir.join("victim.md");
    std::fs::write(&victim_path, victim).expect("victim");
    let typed_path = dir.join("typed.rs");
    std::fs::write(&typed_path, "fn main() {\n    let x = 1;\n}\n").expect("typed");
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', {mode:?})\n\
         {settings}\n\
         local victim = pmacs.buffer.find_or_open({victim:?})\n\
         local typed = pmacs.buffer.find_or_open({typed:?})\n\
         local function line(b)\n\
           local r = pmacs.parse._unit_report(b)\n\
           local t = pmacs.parse.tree(b)\n\
           if not r then return 'none' end\n\
           if r.busy then return 'busy' end\n\
           return string.format('mode=%s deaths=%d tree=%s len=%s death=%s',\n\
             r.mode, r.deaths, t and t:language() or '-', t and t:source_len() or '-',\n\
             tostring(r.last_death))\n\
         end\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 120000 do\n\
             pmacs.workers.sleep(50):await()\n\
             local v, r = line(victim), line(typed)\n\
             local said = ''\n\
             for _, b in ipairs(pmacs.buffer.list()) do\n\
               if b:name() == '*errors*' then said = b:slice(0, b:len()) end\n\
             end\n\
             local _, told = said:gsub('was stopped', '')\n\
             local f = assert(io.open({report:?}, 'w'))\n\
             f:write('victim ' .. v .. '\\n' .. 'typed ' .. r .. '\\n' .. 'told ' .. told .. '\\n')\n\
             f:close()\n\
           end\n\
         end)\n",
        victim = victim_path.display().to_string(),
        typed = typed_path.display().to_string(),
        report = report.display().to_string(),
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    (daemon, dir, report)
}

/// Wait until `report` satisfies `done`, returning its last text.
fn wait_report(
    report: &Path,
    stderr_log: &Path,
    limit: Duration,
    done: impl Fn(&str) -> bool,
) -> String {
    let started = Instant::now();
    let mut last = String::new();
    while started.elapsed() < limit {
        if let Ok(text) = std::fs::read_to_string(report) {
            if done(&text) {
                return text;
            }
            last = text;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let stderr = std::fs::read_to_string(stderr_log).unwrap_or_default();
    panic!(
        "the report did not reach the expected state in {limit:?}; last:\n{last}\n\
         daemon stderr:\n{stderr}"
    );
}

/// The victim's unit has been discarded at least once.
fn victim_died(text: &str) -> bool {
    text.lines()
        .find(|l| l.starts_with("victim mode="))
        .is_some_and(|l| !l.contains(" deaths=0 "))
}

fn field<'a>(text: &'a str, row: &str, key: &str) -> &'a str {
    let line = text
        .lines()
        .find(|l| l.starts_with(row))
        .unwrap_or_else(|| panic!("no {row} row in\n{text}"));
    let start = line
        .find(&format!("{key}="))
        .unwrap_or_else(|| panic!("no {key} in {line}"))
        + key.len()
        + 1;
    let rest = &line[start..];
    rest.split(' ').next().unwrap_or("")
}

fn wasm_module_beside(daemon_binary: &Path) -> bool {
    daemon_binary
        .parent()
        .is_some_and(|d| d.join("pmacs-parse-unit.wasm").exists())
}

/// #301 behind `mode`: stopped at the deadline (300 ms here), the daemon
/// alive, and the other buffer's parse installed from its own unit.
fn stops_301_at_the_deadline(mode: &str, limit: Duration) {
    let victim = std::fs::read_to_string(repo().join("fuzz/accepted/301-nested-openers-98.input"))
        .expect("#301's input");
    let (mut daemon, _dir, report) = daemon(
        mode,
        &victim,
        "pmacs.config.set('syntax.parse-deadline-ms', 300)",
    );
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, limit, |t| {
        victim_died(t) && t.contains("tree=rust")
    });
    assert_eq!(field(&text, "victim", "mode"), mode, "{text}");
    assert!(
        text.lines()
            .find(|l| l.starts_with("victim"))
            .is_some_and(|l| l.contains("death=time:")),
        "#301's unit was stopped by the time limit: {text}"
    );
    assert_eq!(field(&text, "typed", "deaths"), "0", "{text}");
    assert_eq!(
        field(&text, "typed", "len"),
        "29",
        "the Rust buffer's parse installed: {text}"
    );
    assert!(daemon.is_alive(), "the daemon outlived the stopped parse");
}

/// #296 behind `mode` with a 64 MiB unit: stopped at the memory limit, the
/// user told once, the daemon alive.
fn stops_296_at_its_memory_limit(mode: &str, limit: Duration) {
    let (mut daemon, _dir, report) = daemon(
        mode,
        &underscores(14),
        "pmacs.config.set('syntax.parse-memory-limit-mb', 64)\n\
         pmacs.config.set('syntax.parse-deadline-ms', 20000)",
    );
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, limit, |t| {
        victim_died(t) && t.contains("told 1")
    });
    assert!(
        text.lines()
            .find(|l| l.starts_with("victim"))
            .is_some_and(|l| l.contains("death=memory:")),
        "#296's unit was stopped by its memory limit: {text}"
    );
    assert!(text.contains("told 1"), "the user is told once: {text}");
    assert!(daemon.is_alive(), "the daemon outlived the stopped parse");
}

#[test]
fn e7i_a_process_unit_stops_301_at_the_deadline_and_the_daemon_lives() {
    stops_301_at_the_deadline("process", Duration::from_secs(20));
}

#[test]
fn e7i_a_process_unit_stops_296_at_its_memory_limit_and_says_so_once() {
    stops_296_at_its_memory_limit("process", Duration::from_secs(25));
}

/// Run by hand: `scripts/build-parse-unit-wasm --profile dev` puts the
/// module beside the debug `pmacs` (the gate builds no wasm).
#[test]
#[ignore = "needs pmacs-parse-unit.wasm beside the debug pmacs (scripts/build-parse-unit-wasm)"]
fn e7i_a_wasm_unit_stops_301_at_the_deadline_and_the_daemon_lives() {
    assert!(
        wasm_module_beside(Path::new(env!("CARGO_BIN_EXE_pmacs"))),
        "no pmacs-parse-unit.wasm beside {}",
        env!("CARGO_BIN_EXE_pmacs")
    );
    // A debug build compiles the module in about 16 s before the first parse.
    stops_301_at_the_deadline("wasm", Duration::from_secs(90));
}

/// Run by hand, as the row above.
#[test]
#[ignore = "needs pmacs-parse-unit.wasm beside the debug pmacs (scripts/build-parse-unit-wasm)"]
fn e7i_a_wasm_unit_stops_296_at_its_memory_limit_and_says_so_once() {
    assert!(
        wasm_module_beside(Path::new(env!("CARGO_BIN_EXE_pmacs"))),
        "no pmacs-parse-unit.wasm beside {}",
        env!("CARGO_BIN_EXE_pmacs")
    );
    stops_296_at_its_memory_limit("wasm", Duration::from_secs(90));
}
