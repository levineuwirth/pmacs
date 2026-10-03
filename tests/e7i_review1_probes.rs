//! E7i review round 1's probes, against PR #309 at `fa176de`: the states a
//! user reaches through the parse boundary that the build's witnesses did not
//! choose. Each row says what it shows. A row that witnesses a defect is
//! `#[ignore]`d with the finding it fails on at `fa176de`, for the fix round to
//! un-ignore with its fix; every other row runs live and passes at `fa176de`,
//! as a control or a measurement printed to the test's stderr.
//!
//! `PMACS_REVIEW_UNIT` names a worker binary for the hammer rows (a release or
//! a `ThreadSanitizer` build); unset, they drive the debug worker beside cargo's
//! `deps/`. `PMACS_REVIEW_ITERS` raises the hammer's iterations.

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use common::daemon::TestDaemon;
use pmacs_parse_unit::{ParseCall, Request, Response, TextUpdate, WireEdit};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn say(line: &str) {
    let _ = writeln!(std::io::stderr(), "e7i review 1: {line}");
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

/// A daemon whose init.lua is `init`, with `{report}` replaced by the path
/// its Lua writes to; the report's text once `done` holds.
fn run_init(init: &str, limit: Duration, done: impl Fn(&str) -> bool) -> (TestDaemon, String) {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let report = dir.join("report.txt");
    let init = init.replace("{report}", &report.display().to_string());
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, limit, done);
    (daemon, text)
}

/// A stand-in worker that answers `Hello` with this build's protocol and
/// then dies by SIGSEGV at the first parse, as a worker whose grammar C met
/// a memory error does (the stand-in of `e7i_the_fuzz_replay_fails_a_worker_that_crashes`).
fn crashing_unit(dir: &Path) -> PathBuf {
    let fake = dir.join("crashing-unit");
    let protocol = u8::try_from(pmacs_parse_unit::PROTOCOL).expect("a one-byte protocol");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nhead -c 10 >/dev/null\n\
             printf '\\003\\000\\000\\000\\001\\000\\{protocol:03o}\\000\\000\\000\\000'\n\
             head -c 1 >/dev/null\nkill -SEGV $$\n"
        ),
    )
    .expect("fake unit");
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    fake
}

/// The four-crash sequence, and what follows it. At `fa176de` the open's
/// parse and three edits 250 ms apart each started a worker that crashed,
/// four deaths in a second, told to no one. Fix round 1 backs off after a
/// crash (1 s, then 2 s) and stops parsing the buffer at its third crash in
/// a row, so the scenario goes on: past the first back-off an edit crashes
/// a second worker, an edit inside the second back-off starts none, past it
/// a third crash stops the buffer's parsing, and an edit 5 s later still
/// starts none. The report counts deaths at each point, each read once
/// from a report that is not mid-parse, and the `syntax: parsing
/// <path>/a.rs as rust: parse unit crashed` lines in `*errors*`. (As the
/// review committed it the wait read `_unit_report` twice per test, and a
/// parse starting between the two made the second `false` and the
/// coroutine raise: a probe's own race, found here.)
const CRASH_INIT: &str = "pmacs.lsp.config = {}\n\
     pmacs.config.set('syntax.isolation', 'process')\n\
     pmacs.config.set('syntax.parse-unit-path', {unit:?})\n\
     local b = pmacs.buffer.find_or_open({file:?})\n\
     pmacs.async(function()\n\
       local function report() local r = pmacs.parse._unit_report(b); return r and not r.busy and r end\n\
       local function deaths(n, ms)\n\
         local t0 = pmacs.editor.monotonic_ms()\n\
         while true do\n\
           local r = report()\n\
           if r and (r.deaths >= n or pmacs.editor.monotonic_ms() - t0 >= ms) then return r.deaths end\n\
           if pmacs.editor.monotonic_ms() - t0 >= ms + 5000 then return -1 end\n\
           pmacs.workers.sleep(20):await()\n\
         end\n\
       end\n\
       local function edit(i)\n\
         b:insert(b:len(), '// ' .. i .. '\\n')\n\
         pmacs.parse._dispatch(b, 'rust')\n\
       end\n\
       deaths(1, 30000)\n\
       for i = 1, 3 do edit(i); pmacs.workers.sleep(250):await() end\n\
       local burst = deaths(99, 0)\n\
       pmacs.workers.sleep(1000):await()\n\
       edit(4)\n\
       local second = deaths(2, 10000)\n\
       edit(5)\n\
       pmacs.workers.sleep(300):await()\n\
       local held = deaths(99, 0)\n\
       pmacs.workers.sleep(2200):await()\n\
       edit(6)\n\
       local third = deaths(3, 10000)\n\
       pmacs.workers.sleep(5000):await()\n\
       edit(7)\n\
       pmacs.workers.sleep(500):await()\n\
       local r = pmacs.parse._unit_report(b)\n\
       local said = ''\n\
       for _, x in ipairs(pmacs.buffer.list()) do\n\
         if x:name() == '*errors*' then said = x:slice(0, x:len()) end\n\
       end\n\
       local _, told = said:gsub('/a%.rs as rust: parse unit crashed', '')\n\
       local f = assert(io.open('{report}', 'w'))\n\
       f:write(string.format('burst=%d second=%d held=%d third=%d deaths=%d known=%s told=%d named=%s stopped=%s\\n',\n\
         burst, second, held, third, r.deaths,\n\
         tostring(tostring(r.last_death):find('signal 11 (SIGSEGV)', 1, true) ~= nil), told,\n\
         tostring(said:find('/a.rs as rust: parse unit crashed: signal 11 (SIGSEGV)', 1, true) ~= nil),\n\
         tostring(said:find('parsing stopped', 1, true) ~= nil)))\n\
       f:close()\n\
     end)\n";

fn crash_report() -> String {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let unit = crashing_unit(&dir);
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn main() {}\n").expect("a.rs");
    let init = CRASH_INIT
        .replace("{unit:?}", &format!("{:?}", unit.display().to_string()))
        .replace("{file:?}", &format!("{:?}", file.display().to_string()));
    let (_daemon, text) = run_init(&init, Duration::from_mins(1), |t| t.ends_with('\n'));
    say(&format!("a crashing worker: {}", text.trim()));
    text
}

/// Control, live: the editor knows when a buffer's worker crashed. Its unit
/// report's last death names signal 11. Fix round 1: and it backs off. At
/// `fa176de` every keystroke started another worker, which crashed again
/// (four deaths for the open and three edits); now the three edits inside
/// the first second start none, a second worker starts only after the
/// back-off, and after the third crash none starts at all.
#[test]
fn e7i_review1_the_editor_knows_its_worker_crashed() {
    let text = crash_report();
    assert!(text.contains("known=true"), "{text}");
    assert!(
        text.contains("burst=1 second=2 held=2 third=3 deaths=3 "),
        "a crash backs the buffer off, and the third stops its parsing: {text}"
    );
}

/// A worker that crashes is the event containment exists for: a memory
/// error in the grammar's C running a crafted file's bytes (CLAUDE.md:
/// "containment is not safety"). In-process it aborted the editor, loudly.
/// At `fa176de` `_install_settled` reported it as `"failed"`, and
/// `syntax.lua` said nothing for `"failed"`: the buffer went unhighlighted
/// and every keystroke respawned a worker that crashed again, in silence.
/// Fix round 1: it is told in `*errors*`, naming the buffer, the grammar
/// and the signal, once for the streak of crashes and once when the streak
/// stops the buffer's parsing.
#[test]
fn e7i_review1_a_crashing_worker_is_told_to_the_user() {
    let text = crash_report();
    assert!(
        text.contains("told=2 named=true stopped=true"),
        "the user is told that the buffer's parse worker crashed, once, and \
         that its parsing stopped: {text}"
    );
}

/// A stand-in worker whose parse returns at once and whose install then
/// never finishes: it answers `Hello`, and at the first parse request sends
/// `ParseReturned` for it (id 2, after Hello's 1) and stalls. The editor's
/// bound after a returned parse is `AFTER_PARSE_HARD`, 10 s, not the
/// deadline.
fn stalling_after_parse_unit(dir: &Path) -> PathBuf {
    let fake = dir.join("stalling-unit");
    let protocol = u8::try_from(pmacs_parse_unit::PROTOCOL).expect("a one-byte protocol");
    // `(0u64, Response::ParseReturned(2))` in postcard: 0, variant 9, 2.
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nhead -c 10 >/dev/null\n\
             printf '\\003\\000\\000\\000\\001\\000\\{protocol:03o}\\000\\000\\000\\000'\n\
             head -c 1 >/dev/null\n\
             printf '\\003\\000\\000\\000\\000\\011\\002\\000\\000\\000\\000'\n\
             exec sleep 60\n"
        ),
    )
    .expect("fake unit");
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    fake
}

/// A worker killed after its parse returned is reported as having run past
/// the deadline. `ProcessUnit::call` makes every timeout a `Death::Time`
/// whose message is `DeadlineExceeded` with the parse's deadline: the 10 s
/// bound after `ParseReturned` (and a read's 2 s `READ_HARD`, described as a
/// 1,900 ms deadline) included. So the user is told the parse "ran past
/// syntax.parse-deadline-ms", which it did not, and the death's kind cannot
/// tell the deadline from what follows it. At `fa176de` this stand-in's
/// buffer reports `time: parse ran past its deadline of 5000 ms and was
/// cancelled after 10 s`.
#[test]
#[ignore = "E7i review 1, Low: a kill after the parse returned is told as the deadline (fails at fa176de)"]
fn e7i_review1_a_kill_after_the_parse_returned_is_not_called_the_deadline() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let unit = stalling_after_parse_unit(&dir);
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn main() {}\n").expect("a.rs");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         pmacs.config.set('syntax.parse-unit-path', {unit:?})\n\
         local b = pmacs.buffer.find_or_open({file:?})\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           local function report() local r = pmacs.parse._unit_report(b); return r and not r.busy and r end\n\
           while not (report() and report().deaths >= 1) and pmacs.editor.monotonic_ms() - t0 < 40000 do\n\
             pmacs.workers.sleep(50):await()\n\
           end\n\
           pmacs.workers.sleep(500):await()\n\
           local said = ''\n\
           for _, x in ipairs(pmacs.buffer.list()) do\n\
             if x:name() == '*errors*' then said = x:slice(0, x:len()) end\n\
           end\n\
           local r = pmacs.parse._unit_report(b)\n\
           local f = assert(io.open('{{report}}', 'w'))\n\
           f:write(string.format('after=%d death=%s told_deadline=%s\\n', pmacs.editor.monotonic_ms() - t0,\n\
             tostring(r.last_death), tostring(said:find('ran past syntax.parse-deadline-ms', 1, true) ~= nil)))\n\
           f:close()\n\
         end)\n",
        unit = unit.display().to_string(),
        file = file.display().to_string(),
    );
    let (_daemon, text) = run_init(&init, Duration::from_mins(1), |t| t.ends_with('\n'));
    say(&format!("a kill after the parse returned: {}", text.trim()));
    assert!(
        text.contains("told_deadline=false"),
        "a parse that returned is not told as one that ran past the deadline: {text}"
    );
}

/// A worker outlives its editor when the editor dies during a parse that
/// never returns. The deadline is the editor's (a kill 100 ms after it), so
/// once the editor is gone (a crash, an OOM kill of the daemon, `kill -9`)
/// nothing enforces it: the worker's serve loop sees its stdin end and
/// joins its parse thread, which is still in #301's condensation. No
/// parent-death signal is set (no `prctl`, no `PR_SET_PDEATHSIG` in the
/// worker). Here #301's opener series at 40 rather than 98 bytes' 28
/// (exponential in them), the daemon killed a second into the parse, the
/// worker checked 8 s later, past the 5.1 s the editor would have allowed;
/// then killed by this test. Fix round 1: the worker exits when its stdin
/// ends, which the editor's death closes, so it is polled from the kill on
/// and must be gone within a second, well inside the deadline it outlived.
#[test]
fn e7i_review1_a_worker_does_not_outlive_its_editor() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let file = dir.join("victim.md");
    std::fs::write(&file, format!("*bar**\n{}f*bark]", "f![".repeat(40))).expect("victim");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         local b = pmacs.buffer.find_or_open({file:?})\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 30000 do\n\
             local r = pmacs.parse._unit_report(b)\n\
             if r and r.busy and r.unit ~= '' then\n\
               local f = assert(io.open('{{report}}', 'w'))\n\
               f:write(r.unit:match('%d+') .. '\\n')\n\
               f:close()\n\
               return\n\
             end\n\
             pmacs.workers.sleep(20):await()\n\
           end\n\
         end)\n",
        file = file.display().to_string(),
    );
    let (daemon, text) = run_init(&init, Duration::from_secs(40), |t| t.ends_with('\n'));
    let worker = text.trim().to_owned();
    std::thread::sleep(Duration::from_secs(1));
    let stat_of = || std::fs::read_to_string(format!("/proc/{worker}/stat")).unwrap_or_default();
    let running = |stat: &str| !stat.is_empty() && !stat.contains(") Z ");
    assert!(
        running(&stat_of()),
        "the worker is parsing when its daemon is killed"
    );
    let _ = Command::new("kill")
        .args(["-KILL", &daemon.pid().to_string()])
        .status();
    let killed = std::time::Instant::now();
    let mut stat = stat_of();
    while running(&stat) && killed.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(20));
        stat = stat_of();
    }
    let gone_after = killed.elapsed();
    let alive = running(&stat);
    let cpu_ticks: u64 = stat
        .rsplit(')')
        .next()
        .and_then(|rest| rest.split_whitespace().nth(11))
        .and_then(|t| t.parse().ok())
        .unwrap_or(0);
    let _ = Command::new("kill").args(["-KILL", &worker]).status();
    say(&format!(
        "after its daemon was killed: worker {worker} alive={alive} after {gone_after:?}, utime {cpu_ticks} ticks"
    ));
    drop(daemon);
    assert!(
        !alive,
        "the worker ended with its editor rather than spinning on (utime {cpu_ticks} ticks)"
    );
    assert!(
        gone_after < Duration::from_secs(1),
        "the worker ended at once, not at some later bound: {gone_after:?}"
    );
}

/// Folds through the worker are the folds the editor computed in-process:
/// over a deeply nested file and over a 600 KB real one, at many positions,
/// `pmacs.fold.close` then `pmacs.fold.folds` give the same ranges under
/// `none` and `process`, and `close_all` the same top-level set. The time a
/// whole sweep of fold commands takes in each mode goes to stderr.
#[test]
fn e7i_review1_folds_through_the_unit_are_the_folds_in_the_editor() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let mut deep = String::from("fn deep() {\n");
    for i in 0..250 {
        let _ = writeln!(deep, "{}if a{i} {{", "    ".repeat(i % 40 + 1));
    }
    for i in (0..250).rev() {
        let _ = writeln!(deep, "{}}}", "    ".repeat(i % 40 + 1));
    }
    deep.push_str("}\n\nfn after() {\n    let x = 1;\n}\n");
    let deep_path = dir.join("deep.rs");
    std::fs::write(&deep_path, &deep).expect("deep.rs");
    let big_path = repo().join("src/editor.rs");
    for (path, step) in [(&deep_path, 97usize), (&big_path, 7919usize)] {
        let len = std::fs::metadata(path).expect("file").len() as usize;
        let positions: Vec<usize> = (0..len).step_by(step).take(60).collect();
        let list = positions
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut seen = Vec::new();
        for mode in ["none", "process"] {
            let init = format!(
                "pmacs.lsp.config = {{}}\n\
                 pmacs.config.set('syntax.isolation', '{mode}')\n\
                 local b = pmacs.buffer.find_or_open({path:?})\n\
                 local function folds()\n\
                   local out = {{}}\n\
                   for _, r in ipairs(pmacs.fold.folds(b)) do out[#out + 1] = r.start .. '-' .. r['end'] end\n\
                   table.sort(out)\n\
                   return table.concat(out, ',')\n\
                 end\n\
                 pmacs.async(function()\n\
                   local t0 = pmacs.editor.monotonic_ms()\n\
                   while not pmacs.parse.tree(b) and pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
                     pmacs.workers.sleep(50):await()\n\
                   end\n\
                   local r = pmacs.parse._unit_report(b)\n\
                   while r and r.busy do pmacs.workers.sleep(50):await(); r = pmacs.parse._unit_report(b) end\n\
                   local rows = {{}}\n\
                   local s0 = pmacs.editor.monotonic_ms()\n\
                   for _, p in ipairs({{{list}}}) do\n\
                     pmacs.fold.open_all(b)\n\
                     local ok = pmacs.fold.close(b, p)\n\
                     rows[#rows + 1] = p .. ':' .. tostring(ok) .. ':' .. folds()\n\
                   end\n\
                   pmacs.fold.open_all(b)\n\
                   local n = pmacs.fold.close_all(b)\n\
                   rows[#rows + 1] = 'all:' .. tostring(n) .. ':' .. folds()\n\
                   pmacs.fold.open_all(b)\n\
                   local took = pmacs.editor.monotonic_ms() - s0\n\
                   local f = assert(io.open('{{report}}', 'w'))\n\
                   f:write('took=' .. took .. '\\n' .. table.concat(rows, '\\n') .. '\\nend\\n')\n\
                   f:close()\n\
                 end)\n",
                path = path.display().to_string(),
            );
            let (_daemon, text) = run_init(&init, Duration::from_mins(3), |t| t.ends_with("end\n"));
            let (took, rows) = text.split_once('\n').expect("took line");
            say(&format!(
                "folds over {} in {mode}: {took} ms for {} commands",
                path.display(),
                positions.len() + 1
            ));
            seen.push(rows.to_owned());
        }
        assert!(
            seen[0].lines().any(|l| l.contains(":true:")),
            "some position folds in-process, so the comparison is not vacuous: {}",
            seen[0]
        );
        assert_eq!(
            seen[0],
            seen[1],
            "folds over {} agree in-process and through the unit",
            path.display()
        );
    }
}

/// `pmacs.parse._parse_now` from Lua on a buffer whose parse is in flight:
/// with #301's openers running to a 2 s deadline in the buffer's worker, the
/// synchronous call waits for the in-flight parse (the per-buffer order
/// lock) before its own, under its own 1 s deadline. How long it blocked the
/// main thread, what it returned, and what is installed after the async
/// parse settles go to stderr, for each mode.
#[test]
fn e7i_review1_parse_now_on_a_buffer_mid_parse() {
    let openers =
        std::fs::read_to_string(repo().join("fuzz/regress/markdown/301-nested-openers-98.input"))
            .expect("#301's input");
    for mode in ["none", "process"] {
        let dir = tempfile::tempdir().expect("tempdir").keep();
        let file = dir.join("mid.md");
        std::fs::write(&file, "# A heading\n\nSome *emphasis*.\n").expect("mid.md");
        let init = format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.config.set('syntax.isolation', '{mode}')\n\
             pmacs.config.set('syntax.parse-deadline-ms', 2000)\n\
             local b = pmacs.buffer.find_or_open({file:?})\n\
             pmacs.async(function()\n\
               local t0 = pmacs.editor.monotonic_ms()\n\
               while not pmacs.parse.tree(b) and pmacs.editor.monotonic_ms() - t0 < 30000 do\n\
                 pmacs.workers.sleep(50):await()\n\
               end\n\
               pmacs.workers.sleep(300):await()\n\
               b:insert(b:len(), {openers:?})\n\
               pmacs.parse._dispatch(b, 'markdown')\n\
               pmacs.workers.sleep(200):await()\n\
               local s0 = pmacs.editor.monotonic_ms()\n\
               local ok, got = pcall(pmacs.parse._parse_now, b, 'markdown', 1000)\n\
               local blocked = pmacs.editor.monotonic_ms() - s0\n\
               pmacs.workers.sleep(4000):await()\n\
               local t = pmacs.parse.tree(b)\n\
               local f = assert(io.open('{{report}}', 'w'))\n\
               f:write(string.format('blocked=%d ok=%s got=%s installed=%s buffer=%d\\n', blocked,\n\
                 tostring(ok), ok and 'tree' or tostring(got):sub(1, 80), t and t:source_len() or '-', b:len()))\n\
               f:close()\n\
             end)\n",
            file = file.display().to_string(),
        );
        let (daemon, text) = run_init(&init, Duration::from_mins(2), |t| t.ends_with('\n'));
        say(&format!("_parse_now mid-parse in {mode}: {}", text.trim()));
        drop(daemon);
    }
}

/// The TUI grid across its worker's death: a real `paint_frame` over a
/// 120 KB Rust file styles row 0's `fn` from its unit; the worker is killed
/// from outside (SIGKILL, as the OOM killer would); the window moves to the
/// end of the file, which no span the editor holds covers. The grid then
/// restyles from a fresh worker the editor starts and re-parses the previous
/// text into, on the main thread, inside `paint_frame`. How many frames that
/// took and how long each blocked goes to stderr; the row asserts the end is
/// styled again within five frames and that the restart is counted.
#[test]
#[allow(clippy::too_many_lines)] // one user's path, frame by frame
fn e7i_review1_the_grid_restyles_after_its_worker_is_killed() {
    use pmacs::cell::{Cell, CellGrid, CellSize, Color};
    use pmacs::editor::EditorState;
    use pmacs::protocol::FrontendId;

    const MARK: Color = Color::Rgb(0x7b, 0x1f, 0xa2);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("big.rs");
    let mut text = String::new();
    let mut n = 0;
    while text.len() < 120_000 {
        let _ = write!(text, "fn f{n}() {{\n    let x = {n};\n}}\n");
        n += 1;
    }
    std::fs::write(&path, &text).expect("big.rs");
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    state
        .lua_host
        .lua()
        .load(format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.config.set('syntax.isolation', 'process')\n\
             pmacs.theme.merge {{ keyword = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}\n\
             pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ))
        .exec()
        .expect("open big.rs in process mode");
    let eval = |state: &EditorState, lua: &str| -> String {
        state
            .lua_host
            .lua()
            .load(lua)
            .eval::<String>()
            .unwrap_or_default()
    };
    let deadline = Instant::now() + Duration::from_mins(1);
    while eval(
        &state,
        "local b = pmacs.window.buffer() local r = pmacs.parse._unit_report(b) \
         return tostring(r ~= nil and not r.busy and r.unit ~= '' and pmacs.parse.tree(b) ~= nil)",
    ) != "true"
    {
        state.tick_processes();
        state.tick_async();
        assert!(Instant::now() < deadline, "the parse never settled");
        std::thread::sleep(Duration::from_millis(10));
    }
    let (rows, cols) = (24u32, 80u32);
    let size = CellSize::new(rows, cols);
    let paint = |state: &EditorState| -> (usize, Duration) {
        let mut cells = vec![Cell::default(); (rows * cols) as usize];
        let mut grid = CellGrid {
            cells: &mut cells,
            stride: cols,
            size,
        };
        let t0 = Instant::now();
        pmacs::editor::paint_frame(state, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
        let took = t0.elapsed();
        (cells.iter().filter(|c| c.style.fg == MARK).count(), took)
    };
    let (before, _) = paint(&state);
    assert!(before > 0, "the first screen is styled from the unit");
    let pid = eval(
        &state,
        "return (pmacs.parse._unit_report(pmacs.window.buffer()).unit:match('%d+'))",
    );
    let killed = Command::new("kill")
        .args(["-KILL", &pid])
        .status()
        .expect("kill");
    assert!(killed.success(), "killed worker {pid}");
    std::thread::sleep(Duration::from_millis(200));
    state.core.borrow_mut().move_buffer_end();
    let mut frames = Vec::new();
    let mut styled_at = None;
    for frame in 0..5 {
        let (marked, took) = paint(&state);
        frames.push(format!("{marked}@{}ms", took.as_millis()));
        if marked > 0 {
            styled_at = Some(frame);
            break;
        }
        state.tick_processes();
        state.tick_async();
    }
    let report = eval(
        &state,
        "local r = pmacs.parse._unit_report(pmacs.window.buffer()) \
         return string.format('deaths=%d reestablished=%d last=%s', r.deaths, r.reestablished, tostring(r.last_death))",
    );
    say(&format!(
        "the grid after its worker was killed: frames [{}], {report}",
        frames.join(", ")
    ));
    assert!(
        styled_at.is_some(),
        "the end of the file is styled again within five frames: {frames:?} {report}"
    );
    assert!(
        report.contains("reestablished=1"),
        "the previous text was re-parsed into a fresh worker: {report}"
    );
}

/// Reads beside an allocation-heavy parse: the worker runs with one malloc
/// arena (`MALLOC_ARENA_MAX=1`, `d9fa165`), so its read thread and its parse
/// thread take one allocator lock, and #296 allocates about a gigabyte a
/// second. With #296's 16 KB paragraph parsing under a 4 GiB allowance and a
/// 10 s deadline, a held tree of the buffer's previous text is read 40
/// times; each read's latency, and whether any came near `READ_HARD` (2 s,
/// past which the editor kills the worker), goes to stderr. Live: it fails
/// only if a read beside the parse is lost.
#[test]
fn e7i_review1_reads_beside_an_allocating_parse() {
    let line = format!("{}a `_`_", "_".repeat(582));
    let victim = vec![line; 28].join("\n") + "\n";
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let file = dir.join("grow.md");
    std::fs::write(&file, "# A heading\n\nSome *emphasis*.\n").expect("grow.md");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         pmacs.config.set('syntax.parse-memory-limit-mb', 4096)\n\
         pmacs.config.set('syntax.parse-memory-total-mb', 0)\n\
         pmacs.config.set('syntax.parse-deadline-ms', 10000)\n\
         local b = pmacs.buffer.find_or_open({file:?})\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while not pmacs.parse.tree(b) and pmacs.editor.monotonic_ms() - t0 < 30000 do\n\
             pmacs.workers.sleep(50):await()\n\
           end\n\
           local t = pmacs.parse.tree(b)\n\
           b:insert(b:len(), {victim:?})\n\
           pmacs.parse._dispatch(b, 'markdown')\n\
           pmacs.workers.sleep(100):await()\n\
           local times, lost, busy = {{}}, 0, 0\n\
           for i = 1, 40 do\n\
             if pmacs.parse._unit_report(b).busy then busy = busy + 1 end\n\
             local s0 = pmacs.editor.monotonic_ms()\n\
             local kind = t:root():child_count()\n\
             times[#times + 1] = pmacs.editor.monotonic_ms() - s0\n\
             if kind == nil then lost = lost + 1 end\n\
             t = pmacs.parse.tree(b)\n\
             pmacs.workers.sleep(50):await()\n\
           end\n\
           table.sort(times)\n\
           local r = pmacs.parse._unit_report(b)\n\
           local f = assert(io.open('{{report}}', 'w'))\n\
           f:write(string.format('busy=%d lost=%d p50=%d max=%d deaths=%d death=%s\\n', busy, lost,\n\
             times[20], times[40], r.deaths, tostring(r.last_death)))\n\
           f:close()\n\
         end)\n",
        file = file.display().to_string(),
    );
    let (_daemon, text) = run_init(&init, Duration::from_mins(2), |t| t.ends_with('\n'));
    say(&format!("reads beside #296's parse: {}", text.trim()));
    assert!(text.contains("lost=0 "), "{text}");
}

/// A read through the boundary is bounded where it was not in-process:
/// `READ_HARD`, 2 s, after which the editor kills the worker (and any parse
/// in it) and the read answers nothing. `tree:sexp()` of a large file's root
/// is such a read. This row times it in-process and through the unit for a
/// file of `PMACS_REVIEW_SEXP_KB` kilobytes of Rust (default 600) and prints
/// both, with whether the unit survived.
#[test]
fn e7i_review1_a_long_read_through_the_unit() {
    let kb: usize = std::env::var("PMACS_REVIEW_SEXP_KB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let file = dir.join("long.rs");
    let mut text = String::new();
    let mut n = 0;
    while text.len() < kb * 1024 {
        let _ = write!(
            text,
            "fn f{n}(a: u32) -> u32 {{\n    let x = a + {n};\n    x * 2\n}}\n"
        );
        n += 1;
    }
    std::fs::write(&file, &text).expect("long.rs");
    for mode in ["none", "process"] {
        let init = format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.config.set('syntax.isolation', '{mode}')\n\
             local b = pmacs.buffer.find_or_open({file:?})\n\
             pmacs.async(function()\n\
               local t0 = pmacs.editor.monotonic_ms()\n\
               while not pmacs.parse.tree(b) and pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
                 pmacs.workers.sleep(50):await()\n\
               end\n\
               local t = pmacs.parse.tree(b)\n\
               local s0 = pmacs.editor.monotonic_ms()\n\
               local sexp = t:sexp()\n\
               local took = pmacs.editor.monotonic_ms() - s0\n\
               local r = pmacs.parse._unit_report(b)\n\
               local f = assert(io.open('{{report}}', 'w'))\n\
               f:write(string.format('took=%d len=%s deaths=%s\\n', took, sexp and #sexp or 'nil',\n\
                 r and r.deaths or '-'))\n\
               f:close()\n\
             end)\n",
            file = file.display().to_string(),
        );
        let (_daemon, text) = run_init(&init, Duration::from_mins(3), |t| t.ends_with('\n'));
        say(&format!("sexp of {kb} KB in {mode}: {}", text.trim()));
    }
}

// ---------------------------------------------------------------------------
// The worker, driven directly: reads beside a parse
// ---------------------------------------------------------------------------

type Frame = std::io::Result<Option<((u64, Response), Vec<u8>)>>;

struct Worker {
    child: Child,
    stdin: ChildStdin,
    answers: Receiver<Frame>,
    next: u64,
    early: HashMap<u64, Response>,
    /// The order frames arrived in: each answer's, and each parse's
    /// `ParseReturned`, so a read answered before its parse returned is
    /// known to have been answered while that parse ran.
    seq: u64,
    answered_at: HashMap<u64, u64>,
    returned_at: HashMap<u64, u64>,
}

fn review_unit() -> PathBuf {
    std::env::var_os("PMACS_REVIEW_UNIT").map_or_else(
        || {
            Path::new(env!("CARGO_BIN_EXE_pmacs"))
                .parent()
                .expect("target dir")
                .join("pmacs-parse-unit")
        },
        PathBuf::from,
    )
}

impl Worker {
    fn spawn(stderr: &Path) -> Self {
        Self::spawn_with(
            stderr,
            pmacs_parse_unit::WORKER_ENV,
            &["--memory-enforcement", "watch"],
        )
    }

    /// A worker with exactly `env` added (the editor's `WORKER_ENV`, or not)
    /// and `args` after its 4 GiB limit.
    fn spawn_with(stderr: &Path, env: &[(&str, &str)], args: &[&str]) -> Self {
        let err = std::fs::File::create(stderr).expect("stderr file");
        let mut child = Command::new(review_unit())
            .env_remove("MALLOC_ARENA_MAX")
            .envs(env.iter().copied())
            .args(["--memory-limit-mb", "4096"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(err)
            .spawn()
            .expect("spawn the worker");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, answers) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = std::io::BufReader::new(stdout);
            loop {
                let frame = pmacs_parse_unit::read_frame(&mut r);
                let end = !matches!(frame, Ok(Some(_)));
                if tx.send(frame).is_err() || end {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            answers,
            next: 1,
            early: HashMap::new(),
            seq: 0,
            answered_at: HashMap::new(),
            returned_at: HashMap::new(),
        }
    }

    fn send(&mut self, request: &Request, payload: &[u8]) -> u64 {
        let id = self.next;
        self.next += 1;
        pmacs_parse_unit::write_frame(&mut self.stdin, &(id, request), payload).expect("send");
        id
    }

    /// The answer to `id`, keeping others that arrive first.
    fn wait(&mut self, id: u64) -> Response {
        if let Some(r) = self.early.remove(&id) {
            return r;
        }
        loop {
            let frame = self.answers.recv_timeout(Duration::from_mins(5));
            self.seq += 1;
            match frame {
                Ok(Ok(Some(((0, Response::ParseReturned(parse)), _)))) => {
                    self.returned_at.insert(parse, self.seq);
                }
                Ok(Ok(Some(((0, _), _)))) => {}
                Ok(Ok(Some(((got, response), _)))) if got == id => {
                    self.answered_at.insert(got, self.seq);
                    return response;
                }
                Ok(Ok(Some(((got, response), _)))) => {
                    self.answered_at.insert(got, self.seq);
                    self.early.insert(got, response);
                }
                Ok(other) => panic!("the worker ended: {other:?}"),
                Err(RecvTimeoutError::Timeout) => panic!("no answer to {id} in 5 min"),
                Err(e) => panic!("the worker ended: {e}"),
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A tiny deterministic generator, so a failing iteration can be replayed.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn point_at(text: &[u8], at: usize) -> (u32, u32) {
    let before = &text[..at];
    let row = before.split(|&b| b == b'\n').count() - 1;
    let col = at
        - before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |p| p + 1);
    (row as u32, col as u32)
}

fn call(language: &str, text: &[u8], update: TextUpdate, edits: Vec<WireEdit>) -> ParseCall {
    ParseCall {
        language: language.to_owned(),
        text: update,
        edits,
        expect_len: text.len() as u32,
        aliases: Vec::new(),
        deadline_ms: None,
        interest: vec![(0, 4096)],
    }
}

/// The reads a renderer, a fold command and a node walk make of `generation`,
/// shaped by `rng` over a text of `len` bytes.
fn reads(generation: u64, len: usize, rng: &mut Lcg) -> Vec<Request> {
    let mut out = Vec::new();
    for _ in 0..3 {
        let s = rng.below(len) as u32;
        out.push(Request::Spans {
            generation,
            ranges: vec![(s, (s + 4096).min(len as u32))],
        });
    }
    out.push(Request::Folds {
        generation,
        at: Some(rng.below(len) as u64),
    });
    out.push(Request::Folds {
        generation,
        at: None,
    });
    out.push(Request::Describe {
        generation,
        path: Vec::new(),
        children: true,
    });
    let first = rng.below(8) as u32;
    out.push(Request::Describe {
        generation,
        path: vec![first],
        children: true,
    });
    out.push(Request::Sexp {
        generation,
        path: vec![first, 0],
    });
    out
}

/// The hammer: `iters` times, an edit's parse is sent, then reads of the two
/// trees the worker keeps, made while that parse runs; then, the parse
/// answered, the same reads of the older tree again (still kept), which must
/// answer exactly as they did beside the parse. Also, every few iterations,
/// a whole-file read of the newest tree is sent and two parses behind it, so
/// that the read thread may hold the last handle on a tree the parses evict
/// and free it beside the next parse (tree-sitter's subtree refcounts).
#[allow(clippy::too_many_lines)] // one interleaving schedule, read in order
fn hammer(language: &str, source: &[u8], iters: usize, seed: u64) -> (usize, usize) {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let stderr = dir.join(format!("{language}.stderr"));
    let mut worker = Worker::spawn(&stderr);
    let mut text = source.to_vec();
    let mut rng = Lcg(seed);
    let id = worker.send(
        &Request::Parse(call(language, &text, TextUpdate::Full, Vec::new())),
        &text,
    );
    let Response::Parsed(first) = worker.wait(id) else {
        panic!("the first parse installed")
    };
    let (mut older, mut newest) = (first.generation, first.generation);
    let (mut compared, mut beside) = (0usize, 0usize);
    for iter in 0..iters {
        let at = rng.below(text.len());
        let insert = b"x";
        let p = point_at(&text, at);
        let edit = WireEdit {
            start_byte: at as u32,
            old_end_byte: at as u32,
            new_end_byte: (at + insert.len()) as u32,
            start: p,
            old_end: p,
            new_end: (p.0, p.1 + insert.len() as u32),
        };
        text.splice(at..at, insert.iter().copied());
        let parse = worker.send(
            &Request::Parse(call(language, &text, TextUpdate::Edits, vec![edit])),
            insert,
        );
        if iter % 5 == 4 {
            // Two more parses queued behind this one, then a whole-file read
            // of the tree they will evict: the read's walk runs on the serve
            // thread while they install, so it may hold that tree's last
            // handle and free it beside the third parse.
            let mut queued = vec![parse];
            for _ in 0..2 {
                let at = rng.below(text.len());
                let p = point_at(&text, at);
                let edit = WireEdit {
                    start_byte: at as u32,
                    old_end_byte: at as u32,
                    new_end_byte: at as u32 + 1,
                    start: p,
                    old_end: p,
                    new_end: (p.0, p.1 + 1),
                };
                text.splice(at..at, *b"y");
                queued.push(worker.send(
                    &Request::Parse(call(language, &text, TextUpdate::Edits, vec![edit])),
                    b"y",
                ));
            }
            let whole = worker.send(
                &Request::Spans {
                    generation: newest,
                    ranges: vec![(0, text.len() as u32)],
                },
                &[],
            );
            for id in queued {
                let Response::Parsed(next) = worker.wait(id) else {
                    panic!("iteration {iter}: a queued parse installed")
                };
                newest = next.generation;
            }
            let _ = worker.wait(whole);
            older = newest - 1;
            continue;
        }
        let mut during = Vec::new();
        for generation in [newest, older] {
            for read in reads(generation, text.len(), &mut rng) {
                let id = worker.send(&read, &[]);
                during.push((read, id));
            }
        }
        let answers: Vec<(Request, u64, Response)> = during
            .into_iter()
            .map(|(read, id)| {
                let answer = worker.wait(id);
                (read, id, answer)
            })
            .collect();
        let Response::Parsed(parsed) = worker.wait(parse) else {
            panic!("iteration {iter}: the edit's parse installed")
        };
        // Reads answered before the parse said it returned were answered
        // while it ran.
        let returned = worker.returned_at.get(&parse).copied().unwrap_or(u64::MAX);
        beside += answers
            .iter()
            .filter(|(_, id, _)| worker.answered_at.get(id).is_some_and(|&at| at < returned))
            .count();
        // The tree that was newest beside the parse is now the older one,
        // still kept: its reads must answer as they did beside the parse.
        for (read, _, answer) in &answers {
            let generation = match read {
                Request::Spans { generation, .. }
                | Request::Folds { generation, .. }
                | Request::Describe { generation, .. }
                | Request::Sexp { generation, .. } => *generation,
                _ => continue,
            };
            if generation != newest {
                continue;
            }
            let id = worker.send(read, &[]);
            let again = worker.wait(id);
            assert_eq!(
                format!("{again:?}"),
                format!("{answer:?}"),
                "iteration {iter} (seed {seed}): a read of generation {generation} beside a \
                 parse answered differently from the same read after it: {read:?}"
            );
            compared += 1;
        }
        older = newest;
        newest = parsed.generation;
    }
    let log = std::fs::read_to_string(&stderr).unwrap_or_default();
    assert!(
        !log.contains("ThreadSanitizer") && !log.contains("Sanitizer"),
        "the worker reported nothing under a sanitizer ({}):\n{}",
        stderr.display(),
        &log[..log.len().min(20_000)]
    );
    (compared, beside)
}

/// Reads beside a parse agree with the same reads after it, over a 600 KB
/// Rust file (681 injection layers in the comparison's frozen copy) and a
/// markdown note with fenced code, `PMACS_REVIEW_ITERS` iterations each
/// (default 20), against `PMACS_REVIEW_UNIT` or the debug worker. Under a
/// `ThreadSanitizer` worker the row also fails on any report the worker
/// writes.
#[test]
fn e7i_review1_reads_beside_a_parse_agree_with_reads_after_it() {
    let iters: usize = std::env::var("PMACS_REVIEW_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let rust = std::fs::read(repo().join("src/editor.rs")).expect("editor.rs");
    let mut markdown = std::fs::read_to_string(repo().join("docs/invariants.md")).expect("md");
    for i in 0..40 {
        let _ = write!(
            markdown,
            "\n## Section {i}\n\nSome *emphasis* and `code`, a [link](https://example.org).\n\n\
             ```rust\nfn f{i}() {{ let x = {i}; }}\n```\n\n```lua\nlocal y = {i}\n```\n"
        );
    }
    let started = Instant::now();
    let (r_cmp, r_bes) = hammer("rust", &rust, iters, 20_261_003);
    let (m_cmp, m_bes) = hammer("markdown", markdown.as_bytes(), iters, 20_261_004);
    say(&format!(
        "hammer against {}: rust {r_cmp} reads compared and {r_bes} answered while their parse ran, markdown {m_cmp} and {m_bes}, \
         {iters} iterations each, {:.1} s",
        review_unit().display(),
        started.elapsed().as_secs_f64()
    ));
    assert!(
        r_cmp > 0 && m_cmp > 0,
        "the hammer compared reads in both languages"
    );
}

// ---------------------------------------------------------------------------
// The records and the release
// ---------------------------------------------------------------------------

/// What a release archive must carry now that every buffer's parse runs in
/// a worker by default: `release.yml`'s archive assertion, the loudest cheap
/// failure, requires `pmacs-parse-unit` in every archive and checks its
/// executable bit, so a release that omits the worker fails before upload.
/// At `fa176de` it requires `pmacs` and `pmacs-gpu` only, and its explicit
/// staging (layer 2) copies only those, so the omission ships green. Fix
/// round 1: the worker is built by a command of its own (`--bin` applies
/// to every package a command names, so a shared command builds none),
/// staged, required in both lists, version-checked and glibc-checked.
#[test]
fn e7i_review1_the_release_archive_requires_the_parse_worker() {
    let workflow =
        std::fs::read_to_string(repo().join(".github/workflows/release.yml")).expect("release.yml");
    let required: Vec<&str> = workflow
        .lines()
        .filter(|l| l.trim_start().starts_with("for required in"))
        .collect();
    assert!(!required.is_empty(), "the archive assertion's lists");
    for line in &required {
        // A list's last word carries the `;` before `do` (fix round 1: as
        // first committed this compared `pmacs-parse-unit;`, so the
        // review's own prescribed line could not pass).
        assert!(
            line.split_whitespace()
                .any(|w| w.trim_end_matches(';') == "pmacs-parse-unit"),
            "the archive assertion requires the worker: {line}"
        );
    }
    assert!(
        workflow.contains("cp target/release/pmacs-parse-unit"),
        "the archive stages the worker"
    );
    let builds: Vec<&str> = workflow
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("cargo build") && l.contains("pmacs-parse-unit"))
        .collect();
    assert!(
        !builds.is_empty() && builds.iter().all(|l| !l.contains("--bin")),
        "the worker is built by a command naming no --bin, which would select \
         targets in every package it names and build no worker: {builds:?}"
    );
    assert!(
        workflow.contains("\"staging/$name/pmacs-parse-unit\" --version"),
        "the staged worker is run and its version checked"
    );
    assert!(
        workflow.contains("for bin in pmacs pmacs-gpu pmacs-parse-unit; do"),
        "the worker is held to the glibc floor"
    );
}

/// The strict-aliasing guard (E7h fix round 1) refuses a build of the
/// grammar C that `-fno-strict-aliasing` does not reach. It runs in the root
/// package's `build.rs`, so it guards a build of `pmacs`; since E7i the C
/// runs in `pmacs-parse-unit`, whose build (and `pmacs-syntax`'s, which
/// links the grammars) has no build script, so `cargo install` of the
/// worker from outside a checkout compiles the grammars without the flag and
/// without refusal (the review's probe: `HOST_CFLAGS = None` in every grammar's
/// build record). Fix round 1: `pmacs-syntax`'s build script makes the
/// refusal, and a check of the worker started outside the checkout, as
/// `cargo install --git` starts one, fails in it, naming the flag.
#[test]
fn e7i_review1_the_aliasing_guard_reaches_the_worker_s_build() {
    let guarded = ["pmacs-syntax/build.rs", "pmacs-parse-unit/build.rs"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(repo().join(p)).ok())
        .any(|s| s.contains("strict_aliasing"));
    assert!(
        guarded,
        "a build script of the worker's own crates runs the strict-aliasing verdict"
    );
    // cargo reads `.cargo/config.toml` only from the directory a build
    // starts in and its ancestors, so a build of this manifest started from
    // a temporary directory sees no `-fno-strict-aliasing`.
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(env!("CARGO"))
        .args([
            "check",
            "--offline",
            "-p",
            "pmacs-parse-unit",
            "--manifest-path",
        ])
        .arg(repo().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(dir.path().join("target"))
        .current_dir(dir.path())
        // a user config's sccache would wrap the C too, and can fail on its
        // own
        .env("RUSTC_WRAPPER", "")
        .env_remove("CARGO_TARGET_DIR")
        // this test runs under the checkout's `[env]`; the build it starts
        // must not inherit it
        .env_remove("HOST_CFLAGS")
        .env_remove("TARGET_CFLAGS")
        .env_remove("CFLAGS")
        .output()
        .expect("cargo check from outside the checkout");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && stderr.contains("failed to run custom build command for `pmacs-syntax")
            && stderr.contains(
                "refusing to build: the C compiler would not receive -fno-strict-aliasing"
            ),
        "the worker's build started outside the checkout is refused by pmacs-syntax's \
         build script, naming the flag:\n{stderr}"
    );
}

/// `fuzz/regress/markdown/301-hang-7672.input` begins with 32 spaces, so
/// markdown reads its line as an indented code block and never injects
/// `markdown_inline`: the replay "answers" it and proves nothing about #301.
/// Without the indent (`strip-7672`) the same bytes hang through markdown's
/// paragraph route, and wrapped in a ```` ```markdown_inline ```` fence they
/// reach the inline grammar verbatim. This row replays the stored input and
/// the de-indented one through the worker as markdown, 1 s deadline: the
/// stored one must be contained as the de-indented one is.
#[test]
#[ignore = "E7i review 1, Low: the stored 301-hang-7672 never reaches #301 by the replay's route (fails at fa176de, answered)"]
fn e7i_review1_the_regress_hang_reaches_its_defect_through_markdown() {
    let stored = repo().join("fuzz/regress/markdown/301-hang-7672.input");
    let dir = tempfile::tempdir().expect("tempdir");
    let bytes = std::fs::read(&stored).expect("stored input");
    let start = bytes.iter().position(|&b| b != b' ').unwrap_or(0);
    let stripped = dir.path().join("strip-7672.input");
    std::fs::write(&stripped, &bytes[start..]).expect("stripped");
    let corpus = dir.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus");
    let out = dir.path().join("out");
    let worker = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("target dir")
        .join("pmacs-parse-unit");
    let status = Command::new(env!("CARGO_BIN_EXE_pmacs_grammar_fuzz"))
        .args(["replay-unit", "--unit"])
        .arg(&worker)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--out")
        .arg(&out)
        .args(["--deadline-ms", "1000", "--grammar", "markdown"])
        .arg("--extra")
        .arg(format!("markdown={}", stored.display()))
        .arg("--extra")
        .arg(format!("markdown={}", stripped.display()))
        .status()
        .expect("run the replay");
    let report = std::fs::read_to_string(out.join("unit-report.md")).unwrap_or_default();
    say(&format!("regress route: exit {status:?}\n{report}"));
    assert!(
        report.contains("strip-7672.input`, time: killed at the deadline"),
        "control: the de-indented input reaches #301 through markdown: {report}"
    );
    assert!(
        report.contains("301-hang-7672.input`, time: killed at the deadline"),
        "the stored input reaches #301 through markdown as well: {report}"
    );
}

/// The worker's own parse cost, by allocator: the same worker binary
/// (`PMACS_REVIEW_UNIT`, a release build for a measurement) runs one file's
/// keystrokes with the editor's `WORKER_ENV` (`MALLOC_ARENA_MAX=1`, since
/// `d9fa165`) and without it, interleaved by round, as the editor drives
/// it under `RLIMIT_AS` (4 GiB here). Each keystroke
/// types `PMACS_REVIEW_TYPE` (default `# `) at `PMACS_REVIEW_AT` and deletes
/// it, as the comparison's probe did. The unit's own `total_us` per parse,
/// p50 and p95 for each, goes to stderr. The file is `PMACS_REVIEW_FILE`
/// (default `docs/invariants.md`), parsed as `PMACS_REVIEW_LANG`
/// (default markdown). A measurement; it fails only if a parse fails.
#[test]
fn e7i_review1_the_worker_s_parse_cost_by_allocator() {
    let file = std::env::var("PMACS_REVIEW_FILE")
        .map_or_else(|_| repo().join("docs/invariants.md"), PathBuf::from);
    let language = std::env::var("PMACS_REVIEW_LANG").unwrap_or_else(|_| "markdown".into());
    let typed = std::env::var("PMACS_REVIEW_TYPE").unwrap_or_else(|_| "# ".into());
    let rounds: usize = std::env::var("PMACS_REVIEW_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let keys: usize = std::env::var("PMACS_REVIEW_KEYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let source = std::fs::read(&file).expect("the file");
    let at: usize = std::env::var("PMACS_REVIEW_AT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(source.len() / 2);
    let dir = tempfile::tempdir().expect("tempdir");
    let mut times: HashMap<&str, Vec<f64>> = HashMap::new();
    let arms: [(&str, &[(&str, &str)]); 2] = [
        ("arena=1", pmacs_parse_unit::WORKER_ENV),
        ("arenas=default", &[]),
    ];
    for round in 0..rounds {
        let order: Vec<usize> = if round % 2 == 0 {
            vec![0, 1]
        } else {
            vec![1, 0]
        };
        for i in order {
            let (name, env) = arms[i];
            let mut worker = Worker::spawn_with(&dir.path().join(format!("{i}.err")), env, &[]);
            let mut text = source.clone();
            let id = worker.send(
                &Request::Parse(call(&language, &text, TextUpdate::Full, Vec::new())),
                &text,
            );
            assert!(
                matches!(worker.wait(id), Response::Parsed(_)),
                "{name}: cold parse"
            );
            for _ in 0..keys {
                for insert in [true, false] {
                    let p = point_at(&text, at);
                    let n = typed.len() as u32;
                    let edit = if insert {
                        text.splice(at..at, typed.bytes());
                        WireEdit {
                            start_byte: at as u32,
                            old_end_byte: at as u32,
                            new_end_byte: at as u32 + n,
                            start: p,
                            old_end: p,
                            new_end: (p.0, p.1 + n),
                        }
                    } else {
                        text.drain(at..at + typed.len());
                        WireEdit {
                            start_byte: at as u32,
                            old_end_byte: at as u32 + n,
                            new_end_byte: at as u32,
                            start: p,
                            old_end: (p.0, p.1 + n),
                            new_end: p,
                        }
                    };
                    let payload = if insert { typed.as_bytes() } else { &[][..] };
                    let id = worker.send(
                        &Request::Parse(call(&language, &text, TextUpdate::Edits, vec![edit])),
                        payload,
                    );
                    let Response::Parsed(parsed) = worker.wait(id) else {
                        panic!("{name}: a keystroke's parse installed")
                    };
                    times
                        .entry(name)
                        .or_default()
                        .push(parsed.total_us as f64 / 1000.0);
                }
            }
        }
    }
    for (name, _) in arms {
        let mut v = times.remove(name).unwrap_or_default();
        v.sort_by(f64::total_cmp);
        let q = |p: f64| v[((v.len() - 1) as f64 * p) as usize];
        say(&format!(
            "worker parse cost, {} ({} bytes, {language}), {name}: n={} p50={:.1} p95={:.1} ms",
            file.display(),
            source.len(),
            v.len(),
            q(0.5),
            q(0.95)
        ));
    }
}

/// The one interleaving the reasoning names: a read of a tree that two
/// installs evict while the read walks it, so the serve thread drops the
/// last handle (`ts_tree_delete`, tree-sitter's atomic decrements) beside
/// the third parse's `ts_tree_edit` (`ts_subtree_make_mut`'s plain
/// `ref_count == 1` read). Three parses are queued, then a whole-file read
/// of the tree before the first; the row counts the iterations in which
/// that read answered with spans after the second parse's answer (so the
/// tree was evicted under it) and before the third's (so the drop ran
/// beside a parse). Against a `ThreadSanitizer` worker the row also fails on
/// any report. `PMACS_REVIEW_ITERS` iterations (default 10) over
/// `PMACS_REVIEW_FILE` (default `src/editor.rs`) as `PMACS_REVIEW_LANG`.
#[test]
fn e7i_review1_a_read_that_outlives_two_installs() {
    let iters: usize = std::env::var("PMACS_REVIEW_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let stderr = dir.join("outlive.stderr");
    let mut worker = Worker::spawn(&stderr);
    // A language whose keystroke parses are cheap beside a whole-file read
    // (LaTeX: about a millisecond) lets the read outlive two of them.
    let file = std::env::var("PMACS_REVIEW_FILE")
        .map_or_else(|_| repo().join("src/editor.rs"), PathBuf::from);
    let language = std::env::var("PMACS_REVIEW_LANG").unwrap_or_else(|_| "rust".into());
    let language = language.as_str();
    let mut text = std::fs::read(&file).expect("the file");
    let id = worker.send(
        &Request::Parse(call(language, &text, TextUpdate::Full, Vec::new())),
        &text,
    );
    let Response::Parsed(first) = worker.wait(id) else {
        panic!("the first parse installed")
    };
    let mut newest = first.generation;
    let mut rng = Lcg(7);
    let (mut held, mut beside, mut stale) = (0usize, 0usize, 0usize);
    for _ in 0..iters {
        let mut parses = Vec::new();
        for _ in 0..3 {
            let at = rng.below(text.len());
            let p = point_at(&text, at);
            text.splice(at..at, *b"z");
            let edit = WireEdit {
                start_byte: at as u32,
                old_end_byte: at as u32,
                new_end_byte: at as u32 + 1,
                start: p,
                old_end: p,
                new_end: (p.0, p.1 + 1),
            };
            parses.push(worker.send(
                &Request::Parse(call(language, &text, TextUpdate::Edits, vec![edit])),
                b"z",
            ));
        }
        let read = worker.send(
            &Request::Spans {
                generation: newest,
                ranges: vec![(0, text.len() as u32)],
            },
            &[],
        );
        for &id in &parses {
            let Response::Parsed(p) = worker.wait(id) else {
                panic!("a queued parse installed")
            };
            newest = p.generation;
        }
        let answer = worker.wait(read);
        let at = worker.answered_at[&read];
        if matches!(answer, Response::Spans(_)) {
            if at > worker.answered_at[&parses[1]] {
                held += 1;
                if at < worker.answered_at[&parses[2]] {
                    beside += 1;
                }
            }
        } else {
            stale += 1;
        }
    }
    let log = std::fs::read_to_string(&stderr).unwrap_or_default();
    let reports = log.matches("WARNING: ThreadSanitizer").count();
    say(&format!(
        "a read outliving two installs, against {}: {iters} iterations, {held} answered after \
         their tree was evicted, {beside} of them before the next parse answered, {stale} stale; \
         {reports} sanitizer reports",
        review_unit().display()
    ));
    assert_eq!(reports, 0, "{}", &log[..log.len().min(20_000)]);
}
