//! E10n --- the parse worker's own peak (#445).
//!
//! A parse worker reported its peak resident memory as
//! `getrusage(RUSAGE_SELF).ru_maxrss`, and on Linux an `exec`'d process
//! keeps the high-water mark of the address space it replaced: a worker's
//! reading started at its spawner's peak. The editor replaces a worker at
//! its buffer's next parse once that reading passed
//! `syntax.parse-worker-recycle-mb` (256 MiB by default), so once the
//! daemon's own peak passed 256 MiB every worker it spawned was replaced
//! after its first parse, and nothing it parsed was incremental.
//!
//! The rows here grow a real daemon past 256 MiB from its `init.lua` (a
//! Lua table of 1 MiB strings, held for its life) and read what its
//! workers do from `PMACS_E7I_TRACE`, one JSON line per unit event: each
//! parse's worker, whether it was sent the whole text (`full`), its
//! durations and the peak the worker reported. The daemon's own memory is
//! read by its Lua from `/proc/self/status` (`VmHWM`, the peak of its own
//! address space, and `VmRSS`), so the daemon rows are Linux's; the
//! platform row, which asks whether a child's `getrusage` peak carries its
//! spawner's, runs on Linux and macOS alike, and CI's macOS legs answer
//! it there.

mod common;

use std::io::Write as _;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

/// `syntax.parse-worker-recycle-mb`'s default, in bytes.
#[cfg(target_os = "linux")]
const THRESHOLD: u64 = 256 << 20;

/// How much the daemon grows by before the worker a row watches starts, in
/// MiB: past [`THRESHOLD`] by itself, so the daemon's resident memory, not
/// only its peak, is past it while the worker runs.
#[cfg(target_os = "linux")]
const BALLAST_MIB: usize = 300;

const MIB: f64 = 1_048_576.0;

/// The environment variable under which this binary, run again by the
/// platform row, is that row's child.
const CHILD: &str = "PMACS_E10N_CHILD";

/// What the platform row's parent holds while it spawns its child.
const HELD: usize = 384 << 20;

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / MIB)
}

/// E10n.1, the platform question: does a process's `getrusage` peak carry
/// its spawner's? The row holds [`HELD`] bytes, every page touched, and runs
/// this test binary again as its child, which says what `getrusage` counts
/// as its peak and, where `/proc` has it, its own address space's peak
/// (`VmHWM`). On Linux the child's `getrusage` peak is at least what the
/// parent held (it inherits the parent's high-water mark at `exec`) while
/// its `VmHWM` is its own. On macOS the child's `getrusage` peak is its
/// own: the row asserts it, so CI's macOS legs answer the question (a red
/// there would say macOS inherits too, with both figures), and a real reading
/// on both ends is part of the answer ([`judge_macos_reading`]). The figures
/// go to the test's own stderr, past libtest's capture, as e7i's memory-watch
/// row does: a passing row's `eprintln!` reaches no CI log, and review 1 found
/// `e10n platform` in none of a run's six.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn e10n_a_child_s_rusage_peak_and_its_spawner_s() {
    const NAME: &str = "e10n_a_child_s_rusage_peak_and_its_spawner_s";
    if std::env::var_os(CHILD).is_some() {
        println!(
            "e10n-child rusage={} hwm={}",
            pmacs_parse_unit::rusage_peak_bytes(),
            pmacs_parse_unit::proc_status_bytes("self", "VmHWM:").unwrap_or(0)
        );
        return;
    }
    let mut held = vec![0u8; HELD];
    for (i, page) in held.chunks_mut(4096).enumerate() {
        page[0] = (i % 251) as u8 + 1;
    }
    let parent = pmacs_parse_unit::rusage_peak_bytes();
    let output = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .output()
        .expect("the child ran");
    std::hint::black_box(&held);
    drop(held);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "the child failed: {stdout}");
    // libtest prints the row's name and then, on the same line, what the
    // row prints.
    let line = stdout
        .lines()
        .find_map(|l| l.split("e10n-child ").nth(1))
        .unwrap_or_else(|| panic!("the child said nothing: {stdout}"));
    let figure = |key: &str| -> u64 {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(key))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no {key} in {line}"))
    };
    let (child, child_hwm) = (figure("rusage="), figure("hwm="));
    let _ = writeln!(
        std::io::stderr(),
        "e10n platform ({}): the parent held {} (its getrusage peak {}); the child's getrusage peak {}, its VmHWM {}",
        std::env::consts::OS,
        mib(HELD as u64),
        mib(parent),
        mib(child),
        if child_hwm > 0 {
            mib(child_hwm)
        } else {
            "not read here".to_owned()
        }
    );
    if cfg!(target_os = "linux") {
        assert!(
            parent >= HELD as u64,
            "the parent's getrusage peak reads what it held: {} against {}",
            mib(parent),
            mib(HELD as u64)
        );
        assert!(
            child >= HELD as u64,
            "on Linux a child's getrusage peak carries its spawner's: {} against the {} its parent held",
            mib(child),
            mib(HELD as u64)
        );
        assert!(
            child_hwm > 0 && child_hwm < HELD as u64 / 2,
            "on Linux a child's VmHWM is its own: {}",
            mib(child_hwm)
        );
    } else if let Err(why) = judge_macos_reading(parent, child) {
        panic!("{why}");
    }
}

/// The macOS half of the platform row: that a child's `getrusage` peak is its
/// own needs a reading on both ends, or a platform reading 0 ("0 where
/// unread", [`pmacs_parse_unit::rusage_peak_bytes`]) would pass as an answer.
/// It takes the figures as arguments so that a run on Linux can falsify it,
/// the branch that calls it running only on CI's macOS legs.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn judge_macos_reading(parent: u64, child: u64) -> Result<(), String> {
    let held = HELD as u64;
    if parent < held {
        return Err(format!(
            "the parent's getrusage peak reads what it held: {} against {}, so this platform's reading says nothing of its child's",
            mib(parent),
            mib(held)
        ));
    }
    if child == 0 {
        return Err(
            "the child's getrusage peak was not read: 0 bytes says nothing of what it carries"
                .to_owned(),
        );
    }
    if child >= held / 2 {
        return Err(format!(
            "on macOS a child's getrusage peak is its own, not its spawner's: {} against the {} its parent held (its parent's peak {})",
            mib(child),
            mib(held),
            mib(parent)
        ));
    }
    Ok(())
}

/// Review 1's Low 2: the macOS half has a positive control. Each reading it
/// has to refuse, and the shape it has to accept, on any platform that builds
/// the row.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn e10n_the_macos_half_refuses_a_reading_of_nothing() {
    let held = HELD as u64;
    let own = 8 << 20;
    assert!(judge_macos_reading(held + (4 << 20), own).is_ok());
    for (parent, child, why) in [
        (held + (4 << 20), 0, "not read"),
        (0, own, "reads what it held"),
        (0, 0, "reads what it held"),
        (held + (4 << 20), held, "its own, not its spawner's"),
    ] {
        let verdict = judge_macos_reading(parent, child);
        assert!(
            verdict.as_ref().is_err_and(|e| e.contains(why)),
            "parent {parent}, child {child}: {verdict:?}, wanted {why:?}"
        );
    }
}

/// A small Rust file, `lines` lines long.
#[cfg(target_os = "linux")]
fn rust_source(lines: usize) -> String {
    use std::fmt::Write as _;
    let mut text = String::from("fn main() {\n");
    for i in 0..lines {
        let _ = writeln!(text, "    let value_{i} = {i} * 2 + 1;");
    }
    text.push_str("}\n");
    text
}

/// The Lua a grown daemon's rows share: `note` and `finish` write the
/// report (atomically, by rename), `status` reads the daemon's own memory,
/// `settle` waits until a buffer's parse has installed the buffer's text in
/// its unit and stayed so for three polls (a follow-up dispatch would make
/// the unit busy again), `open` and `edit` parse as a user's buffer is
/// parsed (an explicit dispatch only where `syntax.lua`'s hooks made none),
/// and `grow` takes the daemon past [`BALLAST_MIB`] MiB, held for its life.
/// `PMACS_E10N_UNIT_PATH`, set by hand, names another worker binary through
/// `syntax.parse-unit-path`: E10n.3's "before" runs the base's worker under
/// the same editor and the same rows.
#[cfg(target_os = "linux")]
fn lua_prelude(report: &Path, settings: &str) -> String {
    let unit = std::env::var("PMACS_E10N_UNIT_PATH")
        .map(|path| format!("pmacs.config.set('syntax.parse-unit-path', {path:?})"))
        .unwrap_or_default();
    format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         {settings}\n\
         {unit}\n\
         local REPORT = {report:?}\n\
         local out = {{}}\n\
         local function note(s) out[#out + 1] = s end\n\
         local function finish(s)\n\
           note(s)\n\
           local f = assert(io.open(REPORT .. '.part', 'w'))\n\
           f:write(table.concat(out, '\\n') .. '\\n')\n\
           f:close()\n\
           assert(os.rename(REPORT .. '.part', REPORT))\n\
         end\n\
         local function status(key, who)\n\
           local f = io.open('/proc/' .. (who or 'self') .. '/status')\n\
           if not f then return -1 end\n\
           local s = f:read('*a'); f:close()\n\
           local kb = s:match(key .. ':%s*(%d+) kB')\n\
           return kb and tonumber(kb) * 1024 or -1\n\
         end\n\
         local function mem(label)\n\
           note(string.format('mem %s hwm=%d rss=%d', label, status('VmHWM'), status('VmRSS')))\n\
         end\n\
         local function settled(b)\n\
           local r = pmacs.parse._unit_report(b)\n\
           local t = pmacs.parse.tree(b)\n\
           return r ~= nil and not r.busy and r.unit ~= '' and t ~= nil and t:source_len() == b:len()\n\
         end\n\
         local function settle(b)\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           local quiet = 0\n\
           while pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
             pmacs.workers.sleep(20):await()\n\
             if settled(b) then quiet = quiet + 1 else quiet = 0 end\n\
             if quiet >= 3 then return true end\n\
           end\n\
           return false\n\
         end\n\
         local function open(path, lang)\n\
           local b = pmacs.buffer.find_or_open(path)\n\
           if not pmacs.parse._has_view(b) then pmacs.parse._dispatch(b, lang) end\n\
           return b\n\
         end\n\
         local function kick(b, lang)\n\
           if (pmacs.parse._pending_edits(b) or 0) > 0 then pmacs.parse._dispatch(b, lang) end\n\
           return settle(b)\n\
         end\n\
         local function edit(b, lang, i)\n\
           b:insert(0, string.format('// edit %d\\n', i))\n\
           return kick(b, lang)\n\
         end\n\
         local function unit(b) return pmacs.parse._unit_report(b).unit end\n\
         local function grow()\n\
           local ballast = {{}}\n\
           for i = 1, {BALLAST_MIB} do\n\
             ballast[i] = string.rep(string.char(65 + i % 26), 1048576 - 16) .. string.format('%016d', i)\n\
           end\n\
           _G.e10n_ballast = ballast\n\
         end\n",
        report = report.display().to_string(),
    )
}

/// One grown daemon's run: its Lua's notes, its trace, and the files it
/// parsed. The daemon is gone when this is returned.
#[cfg(target_os = "linux")]
struct Run {
    notes: String,
    trace: Vec<serde_json::Value>,
}

#[cfg(target_os = "linux")]
impl Run {
    /// A `mem <label>` note's figure (`hwm` or `rss`), bytes.
    fn mem(&self, label: &str, key: &str) -> u64 {
        let line = self
            .notes
            .lines()
            .find(|l| l.starts_with(&format!("mem {label} ")))
            .unwrap_or_else(|| panic!("no mem {label} note in\n{}", self.notes));
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{key}=")))
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or_else(|| panic!("no {key} in {line}"))
    }

    /// The worker pids a buffer's notes name, in parse order
    /// (`parsed <buffer> <i> pid N`).
    fn units(&self, buffer: &str) -> Vec<u32> {
        self.notes
            .lines()
            .filter_map(|l| l.strip_prefix(&format!("parsed {buffer} ")))
            .filter_map(|rest| rest.split("pid ").nth(1)?.trim().parse().ok())
            .collect()
    }

    /// The trace's events of `kind`.
    fn events(&self, kind: &str) -> impl Iterator<Item = &serde_json::Value> {
        self.trace
            .iter()
            .filter(move |e| e["event"].as_str() == Some(kind))
    }

    /// The trace's `parsed` events for the buffer whose workers are `pids`,
    /// in order.
    fn parses(&self, pids: &[u32]) -> Vec<&serde_json::Value> {
        self.events("parsed")
            .filter(|e| pids.contains(&unit_pid(e)))
            .collect()
    }

    /// The raw buffer id the trace gives the buffer whose workers are
    /// `pids`.
    fn buffer_id(&self, pids: &[u32]) -> u64 {
        self.parses(pids)
            .first()
            .and_then(|e| e["buffer"].as_u64())
            .unwrap_or_else(|| panic!("no parse traced for {pids:?}"))
    }

    /// The trace's `recycle` events for the buffer whose workers are `pids`.
    fn recycles(&self, pids: &[u32]) -> usize {
        let id = self.buffer_id(pids);
        self.events("recycle")
            .filter(|e| e["buffer"].as_u64() == Some(id))
            .count()
    }

    /// The figure `key` carries in the first note starting with `prefix`.
    fn note_figure(&self, prefix: &str, key: &str) -> u64 {
        self.notes
            .lines()
            .find(|l| l.starts_with(prefix))
            .and_then(|l| l.split_whitespace().find_map(|w| w.strip_prefix(key)))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no {key} in a {prefix} note:\n{}", self.notes))
    }

    /// How long starting `pid` took, if the trace records its spawn.
    fn spawn_us(&self, pid: u32) -> Option<u64> {
        self.events("spawn")
            .find(|e| unit_pid(e) == pid)
            .and_then(|e| e["spawn_us"].as_u64())
    }
}

/// `"unit":"pid N"`'s N.
#[cfg(target_os = "linux")]
fn unit_pid(event: &serde_json::Value) -> u32 {
    event["unit"]
        .as_str()
        .and_then(|u| u.strip_prefix("pid "))
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
}

/// Spawn a daemon with `env` and `init` (whose Lua calls `finish` once),
/// tracing its parse units to `dir`'s `trace.jsonl`, and wait up to
/// `limit` for its report; the daemon, alive, and the report.
#[cfg(target_os = "linux")]
fn spawn_and_wait(
    dir: &Path,
    env: &[(&str, &str)],
    init: &str,
    limit: Duration,
) -> (common::daemon::TestDaemon, String) {
    let report = dir.join("report.txt");
    let trace_env = dir.join("trace.jsonl").display().to_string();
    let mut env: Vec<(&str, &str)> = env.to_vec();
    env.push(("PMACS_E7I_TRACE", &trace_env));
    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(&env, init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let started = Instant::now();
    let notes = loop {
        if let Ok(text) = std::fs::read_to_string(&report) {
            break text;
        }
        assert!(
            started.elapsed() < limit,
            "the daemon's init did not finish within {limit:?}; its stderr:\n{}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        !notes.lines().any(|l| l.starts_with("error")),
        "the daemon's run failed:\n{notes}"
    );
    (daemon, notes)
}

/// The unit events `dir`'s daemon traced so far.
#[cfg(target_os = "linux")]
fn trace_events(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("trace.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// [`spawn_and_wait`], the daemon then gone: its notes and its trace.
#[cfg(target_os = "linux")]
fn run_daemon(dir: &Path, env: &[(&str, &str)], init: &str, limit: Duration) -> Run {
    let (daemon, notes) = spawn_and_wait(dir, env, init, limit);
    drop(daemon);
    Run {
        notes,
        trace: trace_events(dir),
    }
}

/// E10n.1's daemon: an early Rust file parsed before the daemon grows, then
/// the daemon grown past [`THRESHOLD`], then a Rust file opened after the
/// growth, each then edited `edits` times, every edit parsed and settled
/// before the next.
#[cfg(target_os = "linux")]
fn grown_run(edits: usize) -> Run {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let early = dir.join("early.rs");
    let after = dir.join("after.rs");
    std::fs::write(&early, rust_source(60)).expect("early.rs");
    std::fs::write(&after, rust_source(60)).expect("after.rs");
    let init = format!(
        "{prelude}\
         pmacs.async(function()\n\
           mem('start')\n\
           local early = open({early:?}, 'rust')\n\
           if not settle(early) then return finish('error the early file did not settle') end\n\
           note('parsed early 0 ' .. unit(early))\n\
           mem('early')\n\
           grow()\n\
           mem('grown')\n\
           local after = open({after:?}, 'rust')\n\
           if not settle(after) then return finish('error the late file did not settle') end\n\
           note('parsed after 0 ' .. unit(after))\n\
           for i = 1, {edits} do\n\
             if not edit(after, 'rust', i) then return finish('error the late file, edit ' .. i) end\n\
             note('parsed after ' .. i .. ' ' .. unit(after))\n\
           end\n\
           for i = 1, {edits} do\n\
             if not edit(early, 'rust', i) then return finish('error the early file, edit ' .. i) end\n\
             note('parsed early ' .. i .. ' ' .. unit(early))\n\
           end\n\
           mem('end')\n\
           finish('done')\n\
         end)\n",
        prelude = lua_prelude(&dir.join("report.txt"), ""),
        early = early.display().to_string(),
        after = after.display().to_string(),
    );
    run_daemon(&dir, &[], &init, Duration::from_mins(3))
}

/// The positive precondition every grown row asserts: the daemon's own
/// peak and its resident memory, read by its Lua after the growth and
/// before the late file's worker started, are both past [`THRESHOLD`].
#[cfg(target_os = "linux")]
fn assert_grown(run: &Run) {
    let (hwm, rss) = (run.mem("grown", "hwm"), run.mem("grown", "rss"));
    assert!(
        hwm > THRESHOLD && rss > THRESHOLD,
        "the daemon grew past {} before the late worker started: VmHWM {}, VmRSS {}",
        mib(THRESHOLD),
        mib(hwm),
        mib(rss)
    );
}

#[cfg(target_os = "linux")]
fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|l| {
            let mut f = l.split_whitespace();
            Some(format!("{} {} {}", f.next()?, f.next()?, f.next()?))
        })
        .unwrap_or_else(|| "unknown".into())
}

/// One buffer's parses as the record prints them: each parse's worker,
/// whether it was sent the whole text, its durations (the editor's round
/// trip, the unit's own, the root parse's) and the peak its worker
/// reported; a parse that started its worker carries the spawn's time.
#[cfg(target_os = "linux")]
fn print_parses(run: &Run, label: &str, buffer: &str) {
    let pids = run.units(buffer);
    let parses = run.parses(&pids);
    eprintln!(
        "e10n {label}: {} parses, {} workers, {} recycled",
        parses.len(),
        {
            let mut distinct = pids.clone();
            distinct.dedup();
            distinct.len()
        },
        run.recycles(&pids)
    );
    let mut seen = Vec::new();
    for (i, parse) in parses.iter().enumerate() {
        let pid = unit_pid(parse);
        let spawn = if seen.contains(&pid) {
            String::new()
        } else {
            seen.push(pid);
            run.spawn_us(pid)
                .map(|us| format!(" spawn {:.2} ms", us as f64 / 1000.0))
                .unwrap_or_default()
        };
        eprintln!(
            "  parse {i}: pid {pid} full={} call {:.2} ms unit {:.2} ms root {:.2} ms peak {}{spawn}",
            parse["full"],
            parse["call_us"].as_u64().unwrap_or(0) as f64 / 1000.0,
            parse["unit_us"].as_u64().unwrap_or(0) as f64 / 1000.0,
            parse["root_us"].as_u64().unwrap_or(0) as f64 / 1000.0,
            mib(parse["peak"].as_u64().unwrap_or(0)),
        );
    }
}

/// E10n.1, measured: a daemon grown past 256 MiB spawns a worker for a
/// small file, and the record shows what each parse of it and of a file
/// opened before the growth did, over eight edits each. Asserts only the
/// precondition and that every parse settled; the rest is the record
/// (`--ignored --nocapture`; `PMACS_E10N_EDITS` changes the run's length).
#[cfg(target_os = "linux")]
#[test]
#[ignore = "a measurement: run by hand with --ignored --nocapture"]
fn e10n_measure_a_daemon_past_256_mib_and_its_workers() {
    let edits = std::env::var("PMACS_E10N_EDITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let load_before = loadavg();
    let run = grown_run(edits);
    let load_after = loadavg();
    assert_grown(&run);
    eprintln!(
        "e10n measure: load {load_before} before, {load_after} after; {edits} edits a file; {}",
        worker_in_use()
    );
    for label in ["start", "early", "grown", "end"] {
        eprintln!(
            "e10n daemon {label}: VmHWM {}, VmRSS {}",
            mib(run.mem(label, "hwm")),
            mib(run.mem(label, "rss"))
        );
    }
    print_parses(&run, "the file opened before the growth", "early");
    print_parses(&run, "the file opened after the growth", "after");
    assert_eq!(run.units("after").len(), edits + 1, "{}", run.notes);
    assert_eq!(run.units("early").len(), edits + 1, "{}", run.notes);
}

/// E10n.2 (i): in a daemon grown past 256 MiB (asserted), a worker started
/// after the growth parses a small Rust file, reports a peak under the
/// threshold, and is the same worker at each next parse, which is
/// incremental; the worker started before the growth likewise. At the base
/// the late worker reported the daemon's peak (about 369 MiB) and was
/// replaced at every parse, each one full.
#[cfg(target_os = "linux")]
#[test]
fn e10n_a_worker_started_after_the_daemon_grew_reports_its_own_peak_and_is_kept() {
    let run = grown_run(2);
    assert_grown(&run);
    for buffer in ["after", "early"] {
        let pids = run.units(buffer);
        let parses = run.parses(&pids);
        // At least the three its notes name; a follow-up dispatch under load
        // would add one, in the same worker and as incremental.
        assert!(
            parses.len() >= 3,
            "{buffer}: its three parses traced: {:?}",
            run.trace
        );
        let peak = parses[0]["peak"].as_u64().expect("a peak");
        assert!(
            peak < THRESHOLD,
            "{buffer}'s worker reports its own peak, under the {} threshold: it reported {} (the daemon's VmHWM {})",
            mib(THRESHOLD),
            mib(peak),
            mib(run.mem("grown", "hwm"))
        );
        assert!(
            pids.iter().all(|&p| p == pids[0]),
            "{buffer}'s worker is kept across its parses: {pids:?}"
        );
        assert_eq!(
            run.recycles(&pids),
            0,
            "{buffer}'s worker was never recycled"
        );
        for (i, parse) in parses.iter().enumerate().skip(1) {
            assert_eq!(
                parse["full"].as_bool(),
                Some(false),
                "{buffer}'s parse {i} is incremental: {parse}"
            );
        }
    }
}

/// #296's paragraph as the issue generates it, `lines` lines long.
#[cfg(target_os = "linux")]
fn underscores(lines: usize) -> String {
    let line = format!("{}a `_`_", "_".repeat(582));
    vec![line; lines].join("\n") + "\n"
}

/// The allowance the memory-watch row gives its worker, MiB.
#[cfg(target_os = "linux")]
const ALLOWANCE_MIB: u64 = 64;

/// E10n.2 (ii), its independence: a worker started by a daemon grown past
/// 256 MiB, with the memory watch forced (`PMACS_PARSE_UNIT_MEMORY=watch`, so
/// no `RLIMIT_AS`) and a 64 MiB allowance, is stopped by its own watch at a
/// limit that does not carry the daemon's peak. The worker first parses a
/// small markdown file, after which the daemon's Lua reads the worker's peak
/// (`VmHWM`, never below what it held when its watch started); then #296's
/// paragraph is appended, whose parse grows without bound. The deadline is
/// two minutes and the row waits one, so no deadline can supply the death;
/// the death record names the watch and the limit it applied. At the base the
/// limit was the daemon's peak plus the allowance.
///
/// The row reads no start: "the limit less the allowance" below is derived
/// from the limit, so these bounds hold for a limit with no start in it
/// (review 1, Medium 1). The equality the gate row asks for is R1-P10's, in
/// `pmacs-parse-unit/tests/worker.rs`, against a reading of the start, and
/// R1-P5's below, through a daemon.
#[cfg(target_os = "linux")]
#[test]
fn e10n_the_memory_watch_holds_a_worker_to_its_own_start_plus_its_allowance() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let victim = dir.join("victim.md");
    std::fs::write(&victim, "# A note\n\nSome text, and *emphasis*.\n").expect("victim");
    let paragraph = dir.join("paragraph.md");
    std::fs::write(&paragraph, format!("\n{}", underscores(28))).expect("paragraph");
    let settings = format!(
        "pmacs.config.set('syntax.parse-memory-limit-mb', {ALLOWANCE_MIB})\n\
         pmacs.config.set('syntax.parse-deadline-ms', 120000)\n\
         pmacs.config.set('syntax.parse-worker-recycle-mb', 0)"
    );
    let init = format!(
        "{prelude}\
         pmacs.async(function()\n\
           grow()\n\
           mem('grown')\n\
           local b = open({victim:?}, 'markdown')\n\
           if not settle(b) then return finish('error the note did not settle') end\n\
           local pid = unit(b):match('%d+')\n\
           note(string.format('worker pid=%s hwm=%d', pid, status('VmHWM', pid)))\n\
           local f = assert(io.open({paragraph:?}))\n\
           local text = f:read('*a'); f:close()\n\
           b:insert(b:len(), text)\n\
           if (pmacs.parse._pending_edits(b) or 0) > 0 then pmacs.parse._dispatch(b, 'markdown') end\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
             pmacs.workers.sleep(20):await()\n\
             local r = pmacs.parse._unit_report(b)\n\
             if r.deaths > 0 then\n\
               note(string.format('stopped after_ms=%d', pmacs.editor.monotonic_ms() - t0))\n\
               return finish('death ' .. tostring(r.last_death))\n\
             end\n\
           end\n\
           finish('error the worker parsing the paragraph was not stopped within 60 s')\n\
         end)\n",
        prelude = lua_prelude(&dir.join("report.txt"), &settings),
        victim = victim.display().to_string(),
        paragraph = paragraph.display().to_string(),
    );
    let run = run_daemon(
        &dir,
        &[("PMACS_PARSE_UNIT_MEMORY", "watch")],
        &init,
        Duration::from_mins(2),
    );
    let hwm = run.mem("grown", "hwm");
    let rss = run.mem("grown", "rss");
    assert!(
        hwm > THRESHOLD && rss > THRESHOLD,
        "the daemon grew past {} before the worker started: VmHWM {}, VmRSS {}",
        mib(THRESHOLD),
        mib(hwm),
        mib(rss)
    );
    let worker_hwm: u64 = run
        .notes
        .lines()
        .find_map(|l| l.strip_prefix("worker "))
        .and_then(|l| l.split("hwm=").nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no worker note in\n{}", run.notes));
    let death = run
        .notes
        .lines()
        .find_map(|l| l.strip_prefix("death "))
        .unwrap_or_else(|| panic!("no death note in\n{}", run.notes));
    eprintln!(
        "e10n watch: the daemon's VmHWM {}, the worker's after its first parse {}; {death}",
        mib(hwm),
        mib(worker_hwm)
    );
    assert!(
        death.starts_with("memory:") && death.contains("(by watch, "),
        "the worker was stopped by its own memory watch: {death}"
    );
    let limit: u64 = death
        .split("a limit of ")
        .nth(1)
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("the death names the watch's limit: {death}"));
    let less_allowance = limit.saturating_sub(ALLOWANCE_MIB << 20);
    assert!(
        less_allowance < hwm,
        "the watch's limit does not carry the daemon's peak: less the {ALLOWANCE_MIB} MiB allowance it is {} against the daemon's VmHWM {}",
        mib(less_allowance),
        mib(hwm)
    );
    assert!(
        less_allowance > 0 && less_allowance <= worker_hwm,
        "the watch's limit less its allowance is at most the worker's VmHWM after its first parse ({}): {}",
        mib(worker_hwm),
        mib(less_allowance)
    );
}

/// One forced-watch daemon stopped on #296's paragraph, grown first or not:
/// the limit the death names, the worker's `VmHWM` after its first parse, and
/// the daemon's own `VmHWM`.
#[cfg(target_os = "linux")]
fn watch_limit(grown: bool, allowance: u64) -> (u64, u64, u64) {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let victim = dir.join("victim.md");
    std::fs::write(&victim, "# A note\n\nSome text, and *emphasis*.\n").expect("victim");
    let paragraph = dir.join("paragraph.md");
    std::fs::write(&paragraph, format!("\n{}", underscores(28))).expect("paragraph");
    let settings = format!(
        "pmacs.config.set('syntax.parse-memory-limit-mb', {allowance})\n\
         pmacs.config.set('syntax.parse-deadline-ms', 120000)\n\
         pmacs.config.set('syntax.parse-worker-recycle-mb', 0)"
    );
    let growth = if grown { "grow()" } else { "" };
    let init = format!(
        "{prelude}\
         pmacs.async(function()\n\
           {growth}\n\
           mem('grown')\n\
           local b = open({victim:?}, 'markdown')\n\
           if not settle(b) then return finish('error the note did not settle') end\n\
           local pid = unit(b):match('%d+')\n\
           note(string.format('worker hwm=%d', status('VmHWM', pid)))\n\
           local f = assert(io.open({paragraph:?}))\n\
           local text = f:read('*a'); f:close()\n\
           b:insert(b:len(), text)\n\
           if (pmacs.parse._pending_edits(b) or 0) > 0 then pmacs.parse._dispatch(b, 'markdown') end\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
             pmacs.workers.sleep(20):await()\n\
             local r = pmacs.parse._unit_report(b)\n\
             if r.deaths > 0 then return finish('death ' .. tostring(r.last_death)) end\n\
           end\n\
           finish('error the worker was not stopped within 60 s')\n\
         end)\n",
        prelude = lua_prelude(&dir.join("report.txt"), &settings),
        victim = victim.display().to_string(),
        paragraph = paragraph.display().to_string(),
    );
    let run = run_daemon(
        &dir,
        &[("PMACS_PARSE_UNIT_MEMORY", "watch")],
        &init,
        Duration::from_mins(2),
    );
    let death = run
        .notes
        .lines()
        .find_map(|l| l.strip_prefix("death "))
        .unwrap_or_else(|| panic!("no death in\n{}", run.notes));
    assert!(death.contains("(by watch, "), "{death}");
    let limit: u64 = death
        .split("a limit of ")
        .nth(1)
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("the limit: {death}"));
    (
        limit,
        run.note_figure("worker ", "hwm="),
        run.mem("grown", "hwm"),
    )
}

/// R1-P5 (review 1, Medium 1), E10n.2 (ii) through a daemon: the same worker
/// under a small parent and a grown one is stopped at the same limit, and the
/// limit less the allowance is memory the worker really held at its start,
/// a figure R1-P10 reads directly at the worker's own seam. A limit with no
/// start in it fails the lower bound here; the row above has none.
#[cfg(target_os = "linux")]
#[test]
fn r1_p5_the_watch_s_limit_does_not_move_with_the_parent() {
    const ALLOWANCE: u64 = 64;
    let (small_limit, small_hwm, small_parent) = watch_limit(false, ALLOWANCE);
    let (grown_limit, grown_hwm, grown_parent) = watch_limit(true, ALLOWANCE);
    eprintln!(
        "r1-p5: parent {} -> limit {} (worker's VmHWM {}); parent {} -> limit {} (worker's VmHWM {})",
        mib(small_parent),
        mib(small_limit),
        mib(small_hwm),
        mib(grown_parent),
        mib(grown_limit),
        mib(grown_hwm)
    );
    assert!(
        grown_parent > THRESHOLD && small_parent < THRESHOLD,
        "one parent small, one grown: {} and {}",
        mib(small_parent),
        mib(grown_parent)
    );
    assert!(
        small_limit.abs_diff(grown_limit) < 8 << 20,
        "the limit moved with the parent: {} against {}",
        mib(small_limit),
        mib(grown_limit)
    );
    for (limit, hwm) in [(small_limit, small_hwm), (grown_limit, grown_hwm)] {
        let start = limit.saturating_sub(ALLOWANCE << 20);
        assert!(
            start >= 1 << 20 && start <= hwm,
            "the limit less the allowance is the worker's own memory at its start: {} (its VmHWM {})",
            mib(start),
            mib(hwm)
        );
    }
}

/// The threshold the recycle row sets, MiB: above a small file's worker's
/// own peak (about 10 MiB here) and below the large file's.
#[cfg(target_os = "linux")]
const RECYCLE_MIB: u64 = 32;

/// E10n.2, a worker whose own parse passes the threshold is still recycled:
/// in a daemon grown past 256 MiB (asserted) with
/// `syntax.parse-worker-recycle-mb` at 32, a large Rust file's worker passes
/// it in its first parse and is replaced at the next (a new worker, a
/// `recycle`, the whole text sent), while a small file's worker in the same
/// daemon stays under it and is kept. The large half is a guard, holding at
/// the base too, where every worker in the grown daemon passed it; the small
/// half failed there, as (i) does.
#[cfg(target_os = "linux")]
#[test]
fn e10n_a_worker_whose_own_parse_passes_the_threshold_is_still_recycled() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let small = dir.join("small.rs");
    let large = dir.join("large.rs");
    std::fs::write(&small, rust_source(60)).expect("small.rs");
    std::fs::write(&large, rust_source(20_000)).expect("large.rs");
    let init = format!(
        "{prelude}\
         pmacs.async(function()\n\
           grow()\n\
           mem('grown')\n\
           for _, name in ipairs({{'small', 'large'}}) do\n\
             local b = open(name == 'small' and {small:?} or {large:?}, 'rust')\n\
             if not settle(b) then return finish('error ' .. name .. ' did not settle') end\n\
             note('parsed ' .. name .. ' 0 ' .. unit(b))\n\
             for i = 1, 2 do\n\
               if not edit(b, 'rust', i) then return finish('error ' .. name .. ', edit ' .. i) end\n\
               note('parsed ' .. name .. ' ' .. i .. ' ' .. unit(b))\n\
             end\n\
           end\n\
           finish('done')\n\
         end)\n",
        prelude = lua_prelude(
            &dir.join("report.txt"),
            &format!("pmacs.config.set('syntax.parse-worker-recycle-mb', {RECYCLE_MIB})")
        ),
        small = small.display().to_string(),
        large = large.display().to_string(),
    );
    let run = run_daemon(&dir, &[], &init, Duration::from_mins(3));
    assert_grown(&run);
    let threshold = RECYCLE_MIB << 20;
    print_parses(&run, "the small file", "small");
    print_parses(&run, "the large file", "large");
    let (small, large) = (run.units("small"), run.units("large"));
    let large_parses = run.parses(&large);
    let large_peak = large_parses[0]["peak"].as_u64().expect("a peak");
    assert!(
        large_peak > threshold,
        "the large file's first parse took its worker past {}: {}",
        mib(threshold),
        mib(large_peak)
    );
    assert_ne!(
        large[0], large[1],
        "the large file's worker was replaced: {large:?}"
    );
    assert!(run.recycles(&large) >= 1, "the trace records the recycle");
    assert_eq!(
        large_parses[1]["full"].as_bool(),
        Some(true),
        "the fresh worker was sent the whole text: {}",
        large_parses[1]
    );
    let small_parses = run.parses(&small);
    let small_peak = small_parses[0]["peak"].as_u64().expect("a peak");
    assert!(
        small_peak < threshold,
        "the small file's worker reports its own peak, under {}: {}",
        mib(threshold),
        mib(small_peak)
    );
    assert!(
        small.iter().all(|&p| p == small[0]) && run.recycles(&small) == 0,
        "the small file's worker is kept: {small:?}"
    );
}

/// R1-P8 (review 1, Low 1), E10n.2's "peak": the worker's answer is its
/// `VmHWM`, not what it holds now. A large parse with the recycle off, the
/// buffer cut down, more parses: the report after them still carries the large
/// parse's high-water mark though the worker has released memory. With what it
/// holds now in the report, a worker that came back down before its report
/// would be kept past the threshold, and nothing else here tells the two apart.
#[cfg(target_os = "linux")]
#[test]
fn r1_p8_the_report_after_a_cut_is_the_peak_not_the_resident() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let large = dir.join("large.rs");
    // One 64 MiB comment: a large text and a small tree, so what the cut
    // releases is the text, in allocations large enough to go back to the
    // kernel. A file of many small nodes keeps its resident memory after the
    // cut (240.9 MiB against a VmHWM of 243.1, measured) and cannot tell the
    // two readings apart.
    let mut text = format!("// {}\n", "a".repeat(64 << 20));
    text.push_str(&rust_source(4));
    std::fs::write(&large, text).expect("large.rs");
    let init = format!(
        "{prelude}\
         pmacs.async(function()\n\
           local b = open({large:?}, 'rust')\n\
           if not settle(b) then return finish('error the large file did not settle') end\n\
           local pid = unit(b):match('%d+')\n\
           note(string.format('large hwm=%d rss=%d', status('VmHWM', pid), status('VmRSS', pid)))\n\
           b:delete(40, b:len())\n\
           if not kick(b, 'rust') then return finish('error the cut did not settle') end\n\
           for k = 1, 6 do\n\
             if not edit(b, 'rust', k) then return finish('error edit ' .. k) end\n\
           end\n\
           note(string.format('cut hwm=%d rss=%d', status('VmHWM', pid), status('VmRSS', pid)))\n\
           finish('done')\n\
         end)\n",
        prelude = lua_prelude(
            &dir.join("report.txt"),
            "pmacs.config.set('syntax.parse-worker-recycle-mb', 0)"
        ),
        large = large.display().to_string(),
    );
    let run = run_daemon(&dir, &[], &init, Duration::from_mins(3));
    let large_hwm = run.note_figure("large ", "hwm=");
    let cut_rss = run.note_figure("cut ", "rss=");
    assert!(
        cut_rss < large_hwm,
        "the worker released memory after the cut: VmRSS {} against the large parse's VmHWM {}",
        mib(cut_rss),
        mib(large_hwm)
    );
    let reported = run.events("parsed").last().expect("a parse")["peak"]
        .as_u64()
        .expect("a peak");
    eprintln!(
        "r1-p8: after the large parse VmHWM {}; after the cut VmRSS {}; the cut's parse reported {}",
        mib(large_hwm),
        mib(cut_rss),
        mib(reported)
    );
    assert!(
        reported >= large_hwm,
        "the report is the high-water mark: {} against {}",
        mib(reported),
        mib(large_hwm)
    );
}

/// Which worker a measurement ran: the one beside this build's `pmacs`, or
/// the binary `PMACS_E10N_UNIT_PATH` names.
#[cfg(target_os = "linux")]
fn worker_in_use() -> String {
    std::env::var("PMACS_E10N_UNIT_PATH").map_or_else(
        |_| "the worker beside this build's pmacs".to_owned(),
        |path| format!("the worker at {path}"),
    )
}

/// The `q`th percentile of `values`, nearest rank.
#[cfg(all(target_os = "linux", feature = "crdt"))]
fn percentile(values: &[u64], q: usize) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted
        .get((sorted.len() * q / 100).min(sorted.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0)
}

/// E10n.3, typing: keys typed through the GPU's route into a small Rust
/// file in a daemon grown past 256 MiB, each timed from the frontend's
/// `apply_keyboard` to the end of the first frame painted with it
/// (`pmacs-gpu --headless-probe`'s `keypaint`, E10k.1's instrument:
/// `kxv` and `.,;` at the end of the last line, each key and each burst
/// deleted again), and, from the daemon's trace over the same keys, what
/// each parse they caused did: its worker, whether it was sent the whole
/// text, its round trip. The three are reported apart, since faster or
/// rarer background parsing alone does not make a key reach the screen
/// sooner. `PMACS_E10N_SAMPLES` (eight by default) sets the samples.
#[cfg(all(target_os = "linux", feature = "crdt"))]
#[test]
#[ignore = "a measurement: run by hand with --ignored --nocapture"]
fn e10n_measure_typing_in_a_daemon_past_256_mib_gpu() {
    let Some(binary) = common::gpu_probe::gpu_probe_binary() else {
        return;
    };
    let samples = std::env::var("PMACS_E10N_SAMPLES").unwrap_or_else(|_| "8".into());
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let file = dir.join("typed.rs");
    std::fs::write(&file, rust_source(60)).expect("typed.rs");
    let init = format!(
        "{prelude}\
         pmacs.async(function()\n\
           grow()\n\
           mem('grown')\n\
           local b = open({file:?}, 'rust')\n\
           if not settle(b) then return finish('error the file did not settle') end\n\
           for _, x in ipairs(pmacs.buffer.list()) do\n\
             if x:name() == '*scratch*' then pcall(pmacs.buffer.kill, x) end\n\
           end\n\
           note('parsed typed 0 ' .. unit(b))\n\
           finish('ready')\n\
         end)\n",
        prelude = lua_prelude(&dir.join("report.txt"), ""),
        file = file.display().to_string(),
    );
    let (daemon, notes) = spawn_and_wait(&dir, &[], &init, Duration::from_mins(2));
    let ready = trace_events(&dir).len();
    let report = dir.join("keypaint.txt");
    let load_before = loadavg();
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_ACTION", "keypaint")
        .env("PMACS_GPU_PROBE_TYPE_TEXT", "kxv|.,;")
        .env("PMACS_GPU_PROBE_SAMPLES", &samples)
        .env("PMACS_GPU_PROBE_GAP_MS", "150")
        .env("PMACS_GPU_PROBE_SAMPLE_MS", "120000")
        .output()
        .expect("run the keypaint probe");
    let load_after = loadavg();
    drop(daemon);
    let run = Run {
        notes,
        trace: trace_events(&dir),
    };
    assert_grown(&run);
    let Some(facts) = common::gpu_probe::probe_facts(&output, &report, "e10n typing") else {
        return;
    };
    assert!(
        !facts.contains_key("failure"),
        "the probe failed: {:?}",
        facts.get("failure")
    );
    eprintln!(
        "e10n typing: load {load_before} before, {load_after} after; {samples} samples; {}",
        worker_in_use()
    );
    print_keys(&facts);
    print_typed_parses(&run.trace[ready..]);
}

/// The keypaint probe's keys and bursts of each kind, at the median and
/// the 95th percentile, with how many keys were painted optimistically.
#[cfg(all(target_os = "linux", feature = "crdt"))]
fn print_keys(facts: &std::collections::HashMap<String, String>) {
    let count = |key: &str| -> usize {
        common::gpu_probe::fact(facts, key)
            .parse()
            .unwrap_or_else(|_| panic!("{key}"))
    };
    for kind in ["kxv", ".,;"] {
        let (mut keys, mut optimistic) = (Vec::new(), 0);
        for n in 0..count("keys") {
            let line = common::gpu_probe::fact(facts, &format!("key.{n}"));
            let parts: Vec<&str> = line.split('|').collect();
            assert_eq!(parts.len(), 6, "key.{n}: {line}");
            if parts[0] == kind {
                keys.push(parts[2].parse::<u64>().expect("microseconds"));
                optimistic += usize::from(parts[5] == "true");
            }
        }
        let bursts: Vec<u64> = (0..count("bursts"))
            .map(|n| common::gpu_probe::fact(facts, &format!("burst.{n}")))
            .filter(|line| line.starts_with(&format!("{kind}|")))
            .map(|line| {
                line.split('|')
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .expect("burst")
            })
            .collect();
        eprintln!(
            "  {kind}: {} keys ({optimistic} optimistic) p50 {:.2} ms p95 {:.2} ms; {} bursts p50 {:.2} ms",
            keys.len(),
            percentile(&keys, 50) as f64 / 1000.0,
            percentile(&keys, 95) as f64 / 1000.0,
            bursts.len(),
            percentile(&bursts, 50) as f64 / 1000.0,
        );
    }
}

/// What the parses in `events` did: how many, in how many workers, how
/// many recycled and sent the whole text, and their round trips.
#[cfg(all(target_os = "linux", feature = "crdt"))]
fn print_typed_parses(events: &[serde_json::Value]) {
    let typed: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e["event"].as_str() == Some("parsed"))
        .collect();
    let mut workers: Vec<u32> = typed.iter().map(|e| unit_pid(e)).collect();
    workers.dedup();
    let calls: Vec<u64> = typed
        .iter()
        .map(|e| e["call_us"].as_u64().unwrap_or(0))
        .collect();
    eprintln!(
        "  the parses the keys caused: {} parses, {} workers, {} recycled, {} full; round trip p50 {:.2} ms p95 {:.2} ms",
        typed.len(),
        workers.len(),
        events
            .iter()
            .filter(|e| e["event"].as_str() == Some("recycle"))
            .count(),
        typed
            .iter()
            .filter(|e| e["full"].as_bool() == Some(true))
            .count(),
        percentile(&calls, 50) as f64 / 1000.0,
        percentile(&calls, 95) as f64 / 1000.0,
    );
}
