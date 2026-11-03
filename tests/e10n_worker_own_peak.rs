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

/// `key`'s figure in kB from `/proc/<who>/status`, in bytes; `None` where
/// there is none (macOS).
fn proc_status_bytes(who: &str, key: &str) -> Option<u64> {
    std::fs::read_to_string(format!("/proc/{who}/status"))
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

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
/// there would say macOS inherits too, with both figures).
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn e10n_a_child_s_rusage_peak_and_its_spawner_s() {
    const NAME: &str = "e10n_a_child_s_rusage_peak_and_its_spawner_s";
    if std::env::var_os(CHILD).is_some() {
        println!(
            "e10n-child rusage={} hwm={}",
            pmacs_parse_unit::rusage_peak_bytes(),
            proc_status_bytes("self", "VmHWM:").unwrap_or(0)
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
    eprintln!(
        "e10n platform ({}): the parent held {} (its getrusage peak {}); the child's getrusage peak {}, its VmHWM {}",
        std::env::consts::OS,
        mib(HELD as u64),
        mib(parent),
        mib(child),
        mib(child_hwm)
    );
    if cfg!(target_os = "linux") {
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
    } else {
        assert!(
            child < HELD as u64 / 2,
            "on macOS a child's getrusage peak is its own, not its spawner's: {} against the {} its parent held (its parent's peak {})",
            mib(child),
            mib(HELD as u64),
            mib(parent)
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
#[cfg(target_os = "linux")]
fn lua_prelude(report: &Path, settings: &str) -> String {
    format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         {settings}\n\
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
         local function edit(b, lang, i)\n\
           b:insert(0, string.format('// edit %d\\n', i))\n\
           if (pmacs.parse._pending_edits(b) or 0) > 0 then pmacs.parse._dispatch(b, lang) end\n\
           return settle(b)\n\
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
/// tracing its parse units, and wait up to `limit` for its report.
#[cfg(target_os = "linux")]
fn run_daemon(dir: &Path, env: &[(&str, &str)], init: &str, limit: Duration) -> Run {
    let report = dir.join("report.txt");
    let trace = dir.join("trace.jsonl");
    let trace_env = trace.display().to_string();
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
    drop(daemon);
    assert!(
        !notes.lines().any(|l| l.starts_with("error")),
        "the daemon's run failed:\n{notes}"
    );
    let trace = std::fs::read_to_string(&trace)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Run { notes, trace }
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
    eprintln!("e10n measure: load {load_before} before, {load_after} after; {edits} edits a file");
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
