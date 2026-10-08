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

/// The two places a parse can run, each with the init line that puts it
/// there: the editor's own process, which since 2.0.0 only the suite's
/// `pmacs.parse._parse_in_editor` reaches, and the buffer's worker,
/// `syntax.isolation`'s one choice. A row holding the worker's answers to
/// the editor's own runs both, the editor first.
const BOUNDARIES: [(&str, &str); 2] = [
    ("the editor", "pmacs.parse._parse_in_editor(true)"),
    ("process", "pmacs.config.set('syntax.isolation', 'process')"),
];

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
/// parse and three edits after it each started a worker that crashed, four
/// deaths in a second, told to no one. Fix round 1 backs off after a crash
/// (1 s, then 2 s) and stops parsing the buffer at its third crash in a row;
/// fix round 2 parses it again when each back-off ends, with no edit. The
/// scenario edits one keystroke every 100 ms until the third crash, each
/// judged by the back-off it met (more than 200 ms left, `held`), then once
/// after the stop, and waits 4 s, longer than any back-off. It records each
/// crash's instant and the back-off it set from the editor's own clock
/// (review 2's Low 1: with the back-off cut to 250 ms the rows still passed,
/// pinning the notice and not the interval): the back-offs are 1 s and 2 s,
/// and each crash comes at least its predecessor's back-off later, which no
/// worker started by an edit inside a back-off allows.
const CRASH_INIT: &str = "pmacs.lsp.config = {}\n\
     pmacs.config.set('syntax.isolation', 'process')\n\
     pmacs.config.set('syntax.parse-unit-path', {unit:?})\n\
     local b = pmacs.buffer.find_or_open({file:?})\n\
     pmacs.async(function()\n\
       local now = pmacs.editor.monotonic_ms\n\
       local function report() return pmacs.parse._unit_report(b) end\n\
       local t0 = now()\n\
       while now() - t0 < 30000 do\n\
         local r = report()\n\
         if r and r.deaths >= 1 then break end\n\
         pmacs.workers.sleep(20):await()\n\
       end\n\
       local crashes = {}\n\
       local function note(r)\n\
         if r.deaths > #crashes then\n\
           crashes[#crashes + 1] = { at = r.crashed_at_ms, backoff = r.backoff_ms }\n\
         end\n\
       end\n\
       note(report())\n\
       local held, n = 0, 0\n\
       while not report().stopped and now() - t0 < 30000 do\n\
         local r = report()\n\
         note(r)\n\
         if r.backoff_left_ms > 200 then held = held + 1 end\n\
         n = n + 1\n\
         b:insert(b:len(), '// ' .. n .. '\\n')\n\
         pmacs.parse._dispatch(b, 'rust')\n\
         pmacs.workers.sleep(100):await()\n\
       end\n\
       note(report())\n\
       local third = report().deaths\n\
       local function gap(i)\n\
         local a, b = crashes[i], crashes[i + 1]\n\
         return (a and b and a.at and b.at) and (b.at - a.at) or -1\n\
       end\n\
       local function backoff(i) return crashes[i] and crashes[i].backoff or -1 end\n\
       b:insert(b:len(), '// after the stop\\n')\n\
       pmacs.parse._dispatch(b, 'rust')\n\
       pmacs.workers.sleep(4000):await()\n\
       local r = report()\n\
       local said = ''\n\
       for _, x in ipairs(pmacs.buffer.list()) do\n\
         if x:name() == '*errors*' then said = x:slice(0, x:len()) end\n\
       end\n\
       local _, told = said:gsub('rust crashed %(SIGSEGV%)', '')\n\
       local function has(text) return tostring(said:find(text, 1, true) ~= nil) end\n\
       local f = assert(io.open('{report}', 'w'))\n\
       f:write(string.format('held=%d third=%d deaths=%d stopped=%s b1=%d b2=%d g12=%d g23=%d known=%s told=%d first=%s named=%s stop=%s\\n',\n\
         held, third, r.deaths, tostring(r.stopped), backoff(1), backoff(2), gap(1), gap(2),\n\
         tostring(tostring(r.last_death):find('signal 11 (SIGSEGV)', 1, true) ~= nil), told,\n\
         has('syntax: rust crashed (SIGSEGV) parsing a.rs; parsing again in 1 s, and not after 3 crashes in a row'),\n\
         has('/a.rs: signal 11 (SIGSEGV)'),\n\
         has('syntax: parsing stopped: rust crashed (SIGSEGV) 3 times in a row parsing a.rs; reopen the file')))\n\
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
/// (four deaths for the open and three edits); now several edits met a
/// back-off, and after the third crash none starts at all. Fix round 2:
/// the third crash comes with no edit after the first back-off's, each
/// back-off ending in a parse of its own.
#[test]
fn e7i_review1_the_editor_knows_its_worker_crashed() {
    let text = crash_report();
    assert!(text.contains("known=true"), "{text}");
    let field = |name: &str| -> u64 {
        text.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{name}=")))
            .and_then(|v| v.parse().ok())
            .unwrap_or(u64::MAX)
    };
    assert!(
        field("held") >= 3,
        "the back-offs were met, not stepped around: {text}"
    );
    assert!(
        field("b1") == 1000 && field("b2") == 2000,
        "the first crash backs off 1 s and the second 2 s: {text}"
    );
    assert!(
        field("g12") >= 1000 && field("g23") >= 2000,
        "no worker started inside a back-off: each crash came at least its \
         predecessor's back-off later: {text}"
    );
    assert!(
        field("g12") <= 4000 && field("g23") <= 5000,
        "each back-off ended in a parse of its own, within 3 s of its end \
         on a slow runner: {text}"
    );
    assert!(
        field("third") == 3 && text.contains("stopped=true"),
        "the buffer was parsed again after each back-off until its third \
         crash stopped it: {text}"
    );
    assert_eq!(
        field("deaths"),
        3,
        "an edit after the stop, and 4 s, started no worker: {text}"
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
/// stops the buffer's parsing. Fix round 2 (review 2's Medium 1): each
/// notice leads with the grammar, the signal and what follows, under the
/// file's short name, so a narrow echo line keeps them; the full path and
/// the worker's words come after.
#[test]
fn e7i_review1_a_crashing_worker_is_told_to_the_user() {
    let text = crash_report();
    assert!(
        text.contains("told=2 first=true named=true stop=true"),
        "the user is told that the buffer's parse worker crashed, once, and \
         that its parsing stopped, grammar, signal and what follows first: \
         {text}"
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
/// cancelled after 10 s`. Fix round 1: the death is `stalled`, told once in
/// its own words; a read's timeout is `read`, and only a parse that has not
/// returned is `time`.
#[test]
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
           while pmacs.editor.monotonic_ms() - t0 < 40000 do\n\
             local r = report()\n\
             if r and r.deaths >= 1 then break end\n\
             pmacs.workers.sleep(50):await()\n\
           end\n\
           pmacs.workers.sleep(500):await()\n\
           local said = ''\n\
           for _, x in ipairs(pmacs.buffer.list()) do\n\
             if x:name() == '*errors*' then said = x:slice(0, x:len()) end\n\
           end\n\
           local r = pmacs.parse._unit_report(b)\n\
           local f = assert(io.open('{{report}}', 'w'))\n\
           f:write(string.format('after=%d death=%s told_deadline=%s told_stall=%s\\n', pmacs.editor.monotonic_ms() - t0,\n\
             tostring(r.last_death), tostring(said:find('ran past syntax.parse-deadline-ms', 1, true) ~= nil),\n\
             tostring(said:find('returned, but parse unit stalled after its parse returned', 1, true) ~= nil)))\n\
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
    assert!(
        text.contains("death=stalled: ") && text.contains("told_stall=true"),
        "its death is a stall after the parse, told once as one: {text}"
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
    // The worker's state and CPU time from `ps`, which Linux and macOS both
    // have; the row read `/proc`, which macOS does not (fix round 1: CI's
    // macOS legs saw no process at all and failed the precondition below).
    let ps = |field: &str| {
        Command::new("ps")
            .args(["-o", field, "-p", &worker])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let running = |state: &str| !state.is_empty() && !state.starts_with('Z');
    assert!(
        running(&ps("stat=")),
        "the worker is parsing when its daemon is killed"
    );
    let _ = Command::new("kill")
        .args(["-KILL", &daemon.pid().to_string()])
        .status();
    let killed = std::time::Instant::now();
    let mut state = ps("stat=");
    while running(&state) && killed.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(20));
        state = ps("stat=");
    }
    let gone_after = killed.elapsed();
    let alive = running(&state);
    let cpu = ps("time=");
    let _ = Command::new("kill").args(["-KILL", &worker]).status();
    say(&format!(
        "after its daemon was killed: worker {worker} alive={alive} after {gone_after:?}, cpu {cpu:?}"
    ));
    drop(daemon);
    assert!(
        !alive,
        "the worker ended with its editor rather than spinning on (cpu {cpu:?})"
    );
    assert!(
        gone_after < Duration::from_secs(1),
        "the worker ended at once, not at some later bound: {gone_after:?}"
    );
}

/// Folds through the worker are the folds the editor computed in-process:
/// over a deeply nested file and over a 600 KB real one, at many positions,
/// `pmacs.fold.close` then `pmacs.fold.folds` give the same ranges in the
/// editor and under `process`, and `close_all` the same top-level set. The
/// time a whole sweep of fold commands takes in each mode goes to stderr.
/// Since 2.0.0 no setting parses in the editor; the oracle is reached by
/// the suite's own route, `pmacs.parse._parse_in_editor`.
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
        for (mode, route) in BOUNDARIES {
            let init = format!(
                "pmacs.lsp.config = {{}}\n\
                 {route}\n\
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
/// parse settles go to stderr.
///
/// Until 2.0.0 it measured the same in the editor (`syntax.isolation`
/// `none`), and that arm was #323: #301's parse in the editor's process is
/// one no deadline reaches, so the arm blocked the main thread 30.6 to
/// 100.9 s where it finished and, three times on CI, outlasted the row's
/// 120 s wait. Its subject was that hang, which the worker exists to stop;
/// with the setting gone no user reaches it, and the suite's in-editor
/// route (`pmacs.parse._parse_in_editor`) would keep the hang in CI for a
/// figure E7i review 1 already recorded. So the arm retires with the
/// setting, and the row measures the worker alone.
#[test]
fn e7i_review1_parse_now_on_a_buffer_mid_parse() {
    let openers =
        std::fs::read_to_string(repo().join("fuzz/regress/markdown/301-nested-openers-98.input"))
            .expect("#301's input");
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let file = dir.join("mid.md");
    std::fs::write(&file, "# A heading\n\nSome *emphasis*.\n").expect("mid.md");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
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
    say(&format!("_parse_now mid-parse in process: {}", text.trim()));
    drop(daemon);
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
/// both, with whether the unit survived: the editor's own time is the
/// baseline the bound is read against, reached since 2.0.0 by the suite's
/// route, `pmacs.parse._parse_in_editor`.
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
    for (mode, route) in BOUNDARIES {
        let init = format!(
            "pmacs.lsp.config = {{}}\n\
             {route}\n\
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

/// The release replays the worker it ships (2.0.0). Every check
/// `release.yml` made of the staged `pmacs-parse-unit` (present, executable,
/// `--version`, the glibc floor) passes a worker that cannot parse a byte.
/// Since 2.0.0 the build job runs `scripts/release-replay-worker` on the
/// staged archive after those checks and before the checksum, so before
/// upload. Driven here on archives staged as the release stages them, with
/// the debug worker and harness this suite builds, markdown only and from
/// this tree (no network), at a 1 s deadline: the real worker passes,
/// #296's and #301's inputs contained; a stand-in that reports the version
/// the release checks for and then ends fails, "did not answer"; and the
/// real worker crashing as it parses a markdown layer (the debug build's
/// `PMACS_PARSE_UNIT_TEST_CRASH`) fails.
#[test]
#[allow(clippy::too_many_lines)] // one release step, three archives
fn the_release_replays_the_archived_worker_and_fails_one_that_cannot_parse() {
    let workflow =
        std::fs::read_to_string(repo().join(".github/workflows/release.yml")).expect("release.yml");
    let at = |needle: &str| {
        workflow
            .find(needle)
            .unwrap_or_else(|| panic!("release.yml carries {needle:?}"))
    };
    let replay =
        at("scripts/release-replay-worker --archive \"${{ steps.stage.outputs.archive }}\"");
    assert!(
        at("\"staging/$name/pmacs-parse-unit\" --version") < replay
            && at("- name: Assert the glibc floor") < replay
            && replay < at("- name: Checksum")
            && replay < at("uses: actions/upload-artifact"),
        "the replay runs after the staged worker's checks and before the checksum and upload"
    );
    assert!(
        at("cargo build --release --bin pmacs_grammar_fuzz") < replay,
        "the harness is built from the checkout before the replay"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let real = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("target dir")
        .join("pmacs-parse-unit");
    let standin = dir.path().join("standin-worker");
    let version = format!("pmacs-parse-unit {} protocol 0", env!("CARGO_PKG_VERSION"));
    std::fs::write(
        &standin,
        format!(
            "#!/bin/sh\ncase \"${{1:-}}\" in --version) echo '{version}'; exit 0 ;; esac\nexit 0\n"
        ),
    )
    .expect("stand-in");
    let stage = |label: &str, worker: &Path| -> PathBuf {
        let name = format!("pmacs-{}-test", env!("CARGO_PKG_VERSION"));
        let staging = dir.path().join(label).join(&name);
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::copy(worker, staging.join("pmacs-parse-unit")).expect("stage the worker");
        let ok = Command::new("chmod")
            .arg("+x")
            .arg(staging.join("pmacs-parse-unit"))
            .status()
            .expect("chmod")
            .success();
        assert!(ok, "chmod");
        let archive = dir.path().join(label).join(format!("{name}.tar.gz"));
        let ok = Command::new("tar")
            .arg("-C")
            .arg(dir.path().join(label))
            .arg("-czf")
            .arg(&archive)
            .arg(&name)
            .status()
            .expect("tar")
            .success();
        assert!(ok, "tar");
        archive
    };
    let replay = |label: &str, worker: &Path, env: &[(&str, &str)]| {
        let archive = stage(label, worker);
        let out = Command::new(repo().join("scripts/release-replay-worker"))
            .current_dir(repo())
            .envs(env.iter().copied())
            .arg("--archive")
            .arg(&archive)
            .arg("--harness")
            .arg(env!("CARGO_BIN_EXE_pmacs_grammar_fuzz"))
            .arg("--out")
            .arg(dir.path().join(label).join("replay"))
            .args(["--sources", "checkout", "--grammar", "markdown", "--"])
            .args(["--max-inputs", "3", "--deadline-ms", "1000"])
            .output()
            .expect("run the release's replay");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        say(&format!("the release's replay, {label}: {:?}", out.status));
        (out.status.success(), text)
    };

    let (ok, text) = replay("real", &real, &[]);
    assert!(ok, "the real worker passes the release's replay:\n{text}");
    assert!(
        text.contains("parsed every grammar it was given, and crashed on nothing")
            && text.contains("markdown contained:"),
        "it parsed seeds and contained the regression inputs:\n{text}"
    );

    let got = Command::new("sh")
        .arg(&standin)
        .arg("--version")
        .output()
        .expect("the stand-in's version");
    assert!(
        String::from_utf8_lossy(&got.stdout)
            .starts_with(&format!("pmacs-parse-unit {} ", env!("CARGO_PKG_VERSION"))),
        "the stand-in passes the release's --version check, so only the replay can tell"
    );
    let (ok, text) = replay("standin", &standin, &[]);
    assert!(
        !ok,
        "a worker that parses nothing fails the release:\n{text}"
    );
    assert!(
        text.contains("did not answer")
            && text.contains("could not be replayed (replay-unit exit 2"),
        "it fails as a worker that did not answer:\n{text}"
    );

    let (ok, text) = replay(
        "crash",
        &real,
        &[("PMACS_PARSE_UNIT_TEST_CRASH", "markdown: ")],
    );
    assert!(
        !ok,
        "a worker that crashes as it parses fails the release:\n{text}"
    );
    assert!(
        text.contains("markdown CRASHED") && text.contains("the archived worker crashed"),
        "it fails as a crash:\n{text}"
    );
}

/// A release says what it is (2.0.0): its notes are reviewed in the tree as
/// `docs/releases/<version>.md`, `release.yml`'s preflight refuses a tag
/// whose version has none before anything is built, and the notes step
/// leads the published notes with them. So the crate's own version has its
/// notes here, and a version bump without them fails this row.
#[test]
fn the_release_s_notes_are_in_the_tree_and_lead_its_page() {
    let version = env!("CARGO_PKG_VERSION");
    let notes = std::fs::read_to_string(repo().join(format!("docs/releases/{version}.md")))
        .unwrap_or_else(|e| panic!("docs/releases/{version}.md: {e}"));
    assert!(
        notes.starts_with(&format!("# pmacs {version}\n")),
        "the notes name their release"
    );
    let workflow =
        std::fs::read_to_string(repo().join(".github/workflows/release.yml")).expect("release.yml");
    assert!(
        workflow.contains("if [ ! -s \"docs/releases/$base.md\" ]; then"),
        "the preflight refuses a tag without notes"
    );
    assert!(
        workflow.contains("\"docs/releases/${version%%-*}.md\" > notes.md"),
        "the notes step leads with them"
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
        // The refusal needs cargo to reach `pmacs-syntax`'s build script,
        // and cargo reaches it only after every other build script in the
        // worker's graph and the grammars' C: 49 crates, the guard last
        // even at `-j 1` (measured at the cache-budget PR's addendum). So
        // this check costs that stage and no more. The verdict reads only
        // the strict-aliasing flags, never `-g`, so no debug information:
        // 346 MB of target became 198 MB on the laptop. On CI it lands
        // beside a leg's whole build, and once ran that disk out (#331).
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_PROFILE_DEV_BUILD_OVERRIDE_DEBUG", "0")
        .env("CARGO_INCREMENTAL", "0")
        .output()
        .expect("cargo check from outside the checkout");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if let Err(why) =
        aliasing_guard_check_verdict(out.status.success(), &stderr, || disk_holding(dir.path()))
    {
        panic!("{why}");
    }
}

/// The aliasing-guard row's reading of its nested check. A check that ran
/// out of disk before `pmacs-syntax`'s build script ran witnessed nothing
/// about the guard, so it fails as that, with the disk's figure (#331),
/// and not as a guard that did not refuse.
fn aliasing_guard_check_verdict(
    succeeded: bool,
    stderr: &str,
    disk: impl FnOnce() -> String,
) -> Result<(), String> {
    let refused = !succeeded
        && stderr.contains("failed to run custom build command for `pmacs-syntax")
        && stderr
            .contains("refusing to build: the C compiler would not receive -fno-strict-aliasing");
    if refused {
        return Ok(());
    }
    if stderr.contains("No space left on device") {
        return Err(format!(
            "the check of the worker ran out of disk before pmacs-syntax's build script \
             could refuse it (#331), so this row witnessed nothing about the guard; the \
             disk holding its target:\n{}\n{stderr}",
            disk()
        ));
    }
    Err(format!(
        "the worker's build started outside the checkout is refused by pmacs-syntax's \
         build script, naming the flag:\n{stderr}"
    ))
}

/// `df` of the filesystem holding `path`, in POSIX form so it reads the
/// same on Linux and macOS, or why it could not be read.
fn disk_holding(path: &std::path::Path) -> String {
    match Command::new("df").arg("-Pk").arg(path).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(e) => format!("df could not run: {e}"),
    }
}

/// #331's own output (`CI` 37648231070, attempt 5): the check died on the
/// disk before the guard ran, and the row now says so, with the figure,
/// rather than reading it as a guard that did not refuse; a refusal still
/// passes, and anything else still fails as the guard's absence.
#[test]
fn e7i_review1_the_aliasing_guard_row_names_a_disk_that_ran_out() {
    let ran_out = "   Compiling libc v0.2.186\n\
        error: linking with `cc` failed: exit status: 1\n  \
        = note: collect2: fatal error: ld terminated with signal 7 [Bus error], core dumped\n\
        error: could not compile `libc` (build script) due to 1 previous error\n\
        warning: tree-sitter-python@0.25.0: src/parser.c:129739:1: fatal error: error \
        writing to /tmp/ccnh7jfG.s: No space left on device\n";
    let why = aliasing_guard_check_verdict(false, ran_out, || "a df figure".to_owned())
        .expect_err("a check that ran out of disk is not a refusal");
    assert!(
        why.contains("ran out of disk before pmacs-syntax's build script")
            && why.contains("(#331)")
            && why.contains("a df figure"),
        "{why}"
    );

    let refusal = "error: failed to run custom build command for `pmacs-syntax v1.1.0 (…)`\n  \
        pmacs: refusing to build: the C compiler would not receive -fno-strict-aliasing; …\n";
    assert_eq!(
        aliasing_guard_check_verdict(false, refusal, || unreachable!("no df for a refusal")),
        Ok(())
    );

    let built = aliasing_guard_check_verdict(true, "    Finished `dev` profile\n", || {
        unreachable!("no df when the disk did not run out")
    })
    .expect_err("a check that succeeds is the guard's absence");
    assert!(
        built.contains("is refused by pmacs-syntax's build script, naming the flag"),
        "{built}"
    );
}

/// `fuzz/regress/markdown/301-hang-7672.input` began with 33 spaces at
/// `fa176de`, so markdown read its line as an indented code block and never
/// injected `markdown_inline`: the replay "answered" it and proved nothing
/// about #301. Fix round 1 stores it without them, and the replay calls a
/// parse the worker cancels at its own deadline contained by time, as it
/// calls one killed past the grace. Replayed through the worker as markdown
/// at a 1 s deadline beside a control, the bytes as `fa176de` stored them
/// (the 33 spaces restored), the stored input is contained by time and the
/// control answered. Until fix round 2 the control was the stored bytes
/// with their leading spaces stripped, which since fix round 1 are none, so
/// it was the subject byte for byte and could not fail apart from it
/// (review 2's Low 4); the count line now says one of the two is answered.
#[test]
fn e7i_review1_the_regress_hang_reaches_its_defect_through_markdown() {
    let stored = repo().join("fuzz/regress/markdown/301-hang-7672.input");
    let dir = tempfile::tempdir().expect("tempdir");
    let bytes = std::fs::read(&stored).expect("stored input");
    assert_ne!(
        bytes.first(),
        Some(&b' '),
        "the stored input begins at its first image opener, not in an indent"
    );
    let as_found = dir.path().join("as-found-7672.input");
    let mut indented = vec![b' '; 33];
    indented.extend_from_slice(&bytes);
    std::fs::write(&as_found, &indented).expect("as found");
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
        .arg(format!("markdown={}", as_found.display()))
        .status()
        .expect("run the replay");
    let report = std::fs::read_to_string(out.join("unit-report.md")).unwrap_or_default();
    say(&format!("regress route: exit {status:?}\n{report}"));
    assert!(
        report.contains("| markdown | 2 | 1 | 1 | 0 |") && !report.contains("as-found-7672.input`"),
        "control: the bytes as found, an indented code block, are answered: {report}"
    );
    assert!(
        report.contains("301-hang-7672.input`, time: "),
        "the stored input reaches #301 through markdown: {report}"
    );
}

/// Low 2: the two settings whose bound is reactive where the kernel cannot
/// hold it (macOS, a Linux without a delegated cgroup) read, at `fa176de`,
/// as preventive everywhere: `syntax.parse-memory-total-mb`'s "a parse
/// that would pass it is stopped" and `syntax.isolation`'s "a parse past its
/// memory or time limit is stopped". Fix round 1: each says where it acts
/// after the memory exists and points to `docs/divergences.md`, as the user
/// reads it (`pmacs.config.describe`, what the settings help renders).
#[test]
fn e7i_review1_the_parse_limit_settings_say_where_they_are_reactive() {
    let state = pmacs::editor::EditorState::new_with_roots(&common::iso::roots());
    for name in ["syntax.isolation", "syntax.parse-memory-total-mb"] {
        let description: String = state
            .lua_host
            .lua()
            .load(format!(
                "return pmacs.config.describe({name:?}).description"
            ))
            .eval()
            .expect("the setting is described");
        assert!(
            description.contains("macOS") && description.contains("docs/divergences.md"),
            "{name} says where its bound is reactive: {description}"
        );
    }
}

/// 2.0.0 removed `syntax.isolation`'s `none`, the switch back to parsing in
/// the editor where no deadline or memory limit reaches the parse. A user's
/// `none` is refused at the setting, naming the one choice left, and the
/// setting's own description says what became of it, as the settings help
/// renders it. The suite's route to the editor's parse,
/// `pmacs.parse._parse_in_editor`, is no setting.
#[test]
fn syntax_isolation_refuses_none_and_names_its_one_choice() {
    let state = pmacs::editor::EditorState::new_with_roots(&common::iso::roots());
    let (ok, err, choices, value, description, settings): (
        bool,
        String,
        String,
        String,
        String,
        String,
    ) = state
        .lua_host
        .lua()
        .load(
            "local ok, err = pcall(pmacs.config.set, 'syntax.isolation', 'none')\n\
             local d = pmacs.config.describe('syntax.isolation')\n\
             local names = {}\n\
             for _, s in ipairs(pmacs.config.list()) do\n\
               if s.name:find('isolation') or s.name:find('in.editor') then\n\
                 names[#names + 1] = s.name\n\
               end\n\
             end\n\
             return ok, tostring(err), table.concat(d.choices, ','),\n\
               pmacs.config.get('syntax.isolation'), d.description, table.concat(names, ',')",
        )
        .eval()
        .expect("the setting is described");
    assert!(!ok, "syntax.isolation = none is refused");
    assert!(
        err.contains(r#"config "syntax.isolation" value "none" is not one of: ["process"]"#),
        "the refusal names the one choice: {err}"
    );
    assert_eq!(choices, "process", "process is the one choice");
    assert_eq!(value, "process", "and the setting still reads it");
    assert!(
        description.contains("the only choice since 2.0.0")
            && description.contains("none, which parsed in the editor")
            && description.contains("was removed"),
        "the setting says what became of none: {description}"
    );
    assert_eq!(
        settings, "syntax.isolation",
        "no other setting reaches the boundary, the suite's in-editor route among them"
    );
}

/// `fuzz/regress/markdown/301-hang-596.input` hangs only under the edit
/// sequence the fuzzer's seed made (`--edits 8 --seed 15703056251634817165`);
/// its bytes alone parse at once by any route, so at `fa176de` the replay
/// answered it and proved nothing about #301. Fix round 1 records the
/// sequence beside it (`301-hang-596.edits`) and `replay-unit` applies it.
/// Replayed as every arm replays `fuzz/regress/`, through markdown's route at
/// a 1 s deadline, the stored input is stopped at the deadline. The control,
/// the same bytes without their edits, replays alone and is answered: until
/// fix round 2 it was judged by its absence from the report, with no count,
/// so a copy never replayed passed (review 2's Low 4).
#[test]
fn e7i_review1_the_regress_edit_sequence_reaches_its_defect() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bare = dir.path().join("596-without-edits.input");
    std::fs::copy(
        repo().join("fuzz/regress/markdown/301-hang-596.input"),
        &bare,
    )
    .expect("the bare copy");
    let corpus = dir.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus");
    let worker = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("target dir")
        .join("pmacs-parse-unit");
    let replay = |out: &Path, inputs: &[&str]| -> String {
        let status = Command::new(env!("CARGO_BIN_EXE_pmacs_grammar_fuzz"))
            .args(["replay-unit", "--unit"])
            .arg(&worker)
            .arg("--corpus")
            .arg(&corpus)
            .arg("--out")
            .arg(out)
            .args(["--deadline-ms", "1000", "--grammar", "markdown"])
            .args(inputs)
            .status()
            .expect("run the replay");
        let report = std::fs::read_to_string(out.join("unit-report.md")).unwrap_or_default();
        say(&format!("regress edit sequence: exit {status:?}\n{report}"));
        report
    };
    let control = replay(
        &dir.path().join("out-control"),
        &["--extra", &format!("markdown={}", bare.display())],
    );
    assert!(
        control.contains("| markdown | 1 | 1 | 0 | 0 |"),
        "control: the bytes alone, one input, are answered: {control}"
    );
    let regress = repo().join("fuzz/regress");
    let report = replay(
        &dir.path().join("out"),
        &["--findings", &regress.display().to_string()],
    );
    assert!(
        report.contains("301-hang-596.input`, time: "),
        "the stored input, with its recorded edits, reaches #301 through markdown: {report}"
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
