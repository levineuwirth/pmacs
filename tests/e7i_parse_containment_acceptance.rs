//! E7i's witnesses: a parse behind the process boundary, through the real
//! daemon (`syntax.lua`'s dispatch, the async runtime, the worker, the
//! settle and install) or a real in-process editor, observed by Lua the
//! daemon runs, which writes what `pmacs.parse._unit_report` and the tree's
//! readers say to a file the test reads. `cargo build --workspace` builds
//! `pmacs-parse-unit` beside `pmacs`, where the editor finds it, and beside
//! cargo's `deps/`, where a test binary's editor does.
//!
//! #301's nested image openers (`fuzz/regress/markdown/301-nested-openers-98.input`)
//! never return from `ts_parser__condense_stack` inside a deadline, and
//! #296's underscore paragraph grows inside `ts_parser__accept`; neither
//! reaches tree-sitter's progress callback, so in-process nothing stops
//! them. In a worker the time and memory limits stop it. The comparison's
//! wasm rows went with the wasm candidate (E7i.3); its measurements stay in
//! the record.

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
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
    daemon_with_env(mode, victim, settings, &[])
}

/// [`daemon`] with environment variables for the daemon.
fn daemon_with_env(
    mode: &str,
    victim: &str,
    settings: &str,
    env: &[(&str, &str)],
) -> (TestDaemon, PathBuf, PathBuf) {
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
    let daemon = TestDaemon::spawn_with_env_and_init(env, &init);
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

/// #301 behind `mode`: stopped at the deadline (300 ms here), the daemon
/// alive, and the other buffer's parse installed from its own unit.
fn stops_301_at_the_deadline(mode: &str, limit: Duration) {
    let victim =
        std::fs::read_to_string(repo().join("fuzz/regress/markdown/301-nested-openers-98.input"))
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

/// On Linux `RLIMIT_AS` stops it; on macOS, which refuses that limit, the
/// worker's memory watch does (condition 3).
#[test]
fn e7i_a_process_unit_stops_296_at_its_memory_limit_and_says_so_once() {
    stops_296_at_its_memory_limit("process", Duration::from_secs(25));
}

/// The previous parse survives its unit's discard (E7i's third acceptance
/// item), shown under `mode`: a clean markdown file of 12 KB parses, so
/// its spans over the first 4 KB come back with the parse (no renderer has
/// shown it, and that is the default interest); then #301's nested openers
/// are appended and the next parse is stopped at the deadline, discarding
/// the unit. The installed parse is still the clean one, and a range near
/// its end, which the editor never fetched, is answered: the editor
/// rebuilds the previous tree from the previous text in a fresh unit.
fn previous_parse_answers_after_its_unit_is_discarded(mode: &str, limit: Duration) {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let mut good = String::new();
    while good.len() < 12_000 {
        good.push_str(
            "## A heading\n\nSome *emphasis*, a `code span` and a [link](https://example.org).\n\n",
        );
    }
    let good_len = good.len();
    let path = dir.join("good.md");
    std::fs::write(&path, &good).expect("good.md");
    let openers =
        std::fs::read_to_string(repo().join("fuzz/regress/markdown/301-nested-openers-98.input"))
            .expect("#301's input");
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', {mode:?})\n\
         pmacs.config.set('syntax.parse-deadline-ms', 300)\n\
         local b = pmacs.buffer.find_or_open({path:?})\n\
         local function write(text)\n\
           local f = assert(io.open({report:?}, 'w')); f:write(text); f:close()\n\
         end\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           local function wait(ok)\n\
             while not ok() do\n\
               if pmacs.editor.monotonic_ms() - t0 > 120000 then return false end\n\
               pmacs.workers.sleep(50):await()\n\
             end\n\
             return true\n\
           end\n\
           local function len() local t = pmacs.parse.tree(b); return t and t:source_len() end\n\
           local function deaths() local r = pmacs.parse._unit_report(b); return r and not r.busy and r.deaths or 0 end\n\
           if not wait(function() return len() == {good_len} end) then write('clean parse never installed') return end\n\
           local head = pmacs.parse._isolated_spans(b, 0, 100)\n\
           b:insert(b:len(), {openers:?})\n\
           pmacs.parse._dispatch(b, 'markdown')\n\
           if not wait(function() return deaths() >= 1 end) then write('no unit was discarded') return end\n\
           local tail = pmacs.parse._isolated_spans(b, {good_len} - 200, {good_len})\n\
           local r = pmacs.parse._unit_report(b)\n\
           write(string.format('head=%s tail=%s installed_len=%s deaths=%d reestablished=%d death=%s\\n',\n\
             tostring(head), tostring(tail), tostring(len()), r.deaths, r.reestablished, tostring(r.last_death)))\n\
         end)\n",
        path = path.display().to_string(),
        report = report.display().to_string(),
    );
    let mut daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, limit, |t| {
        t.ends_with('\n') || t.starts_with("clean") || t.starts_with("no unit")
    });
    let value = |key: &str| {
        text.split_whitespace()
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("no {key} in {text}"))
            .to_owned()
    };
    assert!(
        value("head").parse::<usize>().is_ok_and(|n| n > 0),
        "spans came back with the clean parse: {text}"
    );
    assert_eq!(
        value("installed_len"),
        good_len.to_string(),
        "the clean parse stays installed: {text}"
    );
    assert!(
        text.contains("death=time:"),
        "the pathological parse's unit was stopped at the deadline: {text}"
    );
    assert_eq!(
        value("reestablished"),
        "1",
        "the previous tree was rebuilt in a fresh unit: {text}"
    );
    assert!(
        value("tail").parse::<usize>().is_ok_and(|n| n > 0),
        "a range never fetched is answered from the previous text: {text}"
    );
    assert!(daemon.is_alive(), "the daemon outlived the stopped parse");
}

#[test]
fn e7i_a_process_unit_s_previous_parse_answers_after_its_discard() {
    previous_parse_answers_after_its_unit_is_discarded("process", Duration::from_secs(30));
}

/// Condition 2 of the owner's ruling: whether this machine lets the
/// editor hold its workers to a total with a cgroup. The editor's own
/// attempt, through a daemon in process mode, goes to this test's stderr
/// through `io::stderr` directly, which libtest does not capture, so CI's
/// log carries it on a passing run. Where the gate arms
/// `PMACS_REQUIRE_CGROUP` (this session's cgroup subtree delegated, with
/// the memory controller), the cgroup must be created and the worker moved
/// into it.
#[test]
fn e7i_the_workers_total_says_how_this_machine_holds_it() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let typed = dir.join("typed.rs");
    std::fs::write(&typed, "fn main() {\n    let x = 1;\n}\n").expect("typed");
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         local typed = pmacs.buffer.find_or_open({typed:?})\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
             pmacs.workers.sleep(50):await()\n\
             local r = pmacs.parse._unit_report(typed)\n\
             if r and not r.busy and r.unit ~= '' and pmacs.parse.tree(typed) then\n\
               local f = assert(io.open({report:?}, 'w'))\n\
               f:write(pmacs.parse._isolation_report() or 'nil')\n\
               f:close()\n\
               return\n\
             end\n\
           end\n\
         end)\n",
        typed = typed.display().to_string(),
        report = report.display().to_string(),
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, Duration::from_mins(1), |t| !t.is_empty());
    let _ = writeln!(std::io::stderr(), "e7i cgroup capability: {text}");
    assert!(
        text.starts_with("cgroup "),
        "the editor reports its cgroup attempt once a worker has started: {text}"
    );
    if std::env::var_os("PMACS_REQUIRE_CGROUP").is_some() {
        assert!(
            text.starts_with("cgroup created ")
                && text.contains("refused 0")
                && !text.contains("adopted 0,"),
            "PMACS_REQUIRE_CGROUP is armed, so the workers' cgroup is created and the worker in it: {text}"
        );
    }
}

/// A daemon in process mode whose init.lua opens `file` (`name`, its text
/// `text`), waits until the buffer's parse settles from its unit, runs
/// `probe` (Lua with `b` bound to the buffer, returning a string) and
/// writes `mode=<unit mode> <probe's string>` to the report.
fn probe_in_unit(name: &str, text: &str, probe: &str) -> String {
    probe_in_unit_with(name, text, probe, "")
}

/// [`probe_in_unit`] with `settings` (Lua) run before the file opens.
fn probe_in_unit_with(name: &str, text: &str, probe: &str, settings: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let file = dir.join(name);
    std::fs::write(&file, text).expect("file");
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         {settings}\n\
         local b = pmacs.buffer.find_or_open({file:?})\n\
         local function probe()\n{probe}\nend\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
             pmacs.workers.sleep(50):await()\n\
             local r = pmacs.parse._unit_report(b)\n\
             if r and not r.busy and r.unit ~= '' and pmacs.parse.tree(b) then\n\
               local ok, said = pcall(probe)\n\
               local f = assert(io.open({report:?}, 'w'))\n\
               f:write('mode=' .. r.mode .. ' ' .. (ok and tostring(said) or ('error ' .. tostring(said))))\n\
               f:close()\n\
               return\n\
             end\n\
           end\n\
         end)\n",
        file = file.display().to_string(),
        report = report.display().to_string(),
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    wait_report(&report, &log, Duration::from_mins(1), |t| !t.is_empty())
}

/// Folds read the tree where it lives (E7i): with the buffer's parse in a
/// worker, `fold.close` inside a function's body folds that body, and
/// `fold.close_all` folds both top-level functions, each range the
/// structural source's (head line kept, closing brace kept).
#[test]
fn e7i_folds_read_their_tree_from_the_buffer_s_unit() {
    let text = "fn one() {\n    let a = 1;\n    let b = 2;\n}\n\nfn two() {\n    let c = 3;\n    let d = 4;\n}\n";
    let inside = text.find("let a").expect("let a");
    let probe = format!(
        "local function list()\n\
           local out = {{}}\n\
           for _, r in ipairs(pmacs.fold.folds(b)) do out[#out + 1] = r.start .. '-' .. r['end'] end\n\
           return table.concat(out, ',')\n\
         end\n\
         local closed = pmacs.fold.close(b, {inside})\n\
         local one = list()\n\
         pmacs.fold.open_all(b)\n\
         local n = pmacs.fold.close_all(b)\n\
         return string.format('close=%s folds=%s all=%d folds=%s', tostring(closed), one, n, list())"
    );
    let text_of = |needle: &str| text.find(needle).expect("needle");
    let one = format!(
        "{}-{}",
        text_of("fn one() {") + 10,
        text_of("\n}\n\nfn two")
    );
    let two = format!(
        "{}-{}",
        text_of("fn two() {") + 10,
        text.rfind("\n}\n").expect("last brace")
    );
    let report = probe_in_unit("folds.rs", text, &probe);
    assert_eq!(
        report,
        format!("mode=process close=true folds={one} all=2 folds={one},{two}"),
        "folds come from the unit's tree"
    );
}

/// Lua's node API reads a tree that lives in a worker (E7i), batched: the
/// `pmacs-mcp-ai` fixture's enclosing-function walk finds the function,
/// with its text, its named children, its parent and the tree's
/// s-expression, none of which the editor holds a tree for.
#[test]
fn e7i_lua_s_node_api_walks_a_tree_in_the_buffer_s_unit() {
    let text = "fn one() {\n    let a = 1;\n}\n\nfn two() {}\n";
    let inside = text.find("let a").expect("let a");
    let probe = format!(
        "local t = pmacs.parse.tree(b)\n\
         local root = t:root()\n\
         local function find(node, pos)\n\
           local sb, eb = node:start_byte(), node:end_byte()\n\
           if pos < sb or pos > eb then return nil end\n\
           for _, c in ipairs(node:children()) do\n\
             local f = find(c, pos)\n\
             if f then return f end\n\
           end\n\
           if node:type() == 'function_item' then return node end\n\
         end\n\
         local f = find(root, {inside})\n\
         local names = {{}}\n\
         for _, c in ipairs(f:named_children()) do names[#names + 1] = c:type() end\n\
         local sexp = t:sexp()\n\
         return string.format('root=%s n=%d fn=%s text=%q named=%s parent=%s row=%d sexp=%s current=%s',\n\
           root:type(), root:child_count(), f:type(), f:text(), table.concat(names, ','),\n\
           f:parent():type(), f:end_position().row, sexp:sub(1, 13), tostring(t:is_current()))"
    );
    let report = probe_in_unit("walk.rs", text, &probe);
    assert_eq!(
        report,
        "mode=process root=source_file n=2 fn=function_item \
         text=\"fn one() {\\\n    let a = 1;\\\n}\" named=identifier,parameters,block \
         parent=source_file row=2 sexp=(source_file  current=true",
        "the walk read the unit's tree"
    );
}

/// A tree held across reparses (E7i): its unit keeps the two most recent
/// trees, so after one reparse the held tree still answers reads it had
/// not made (the editor may still be showing it while the newer one
/// settles), and after two it answers nil; `is_current` is false from the
/// first, and the buffer's new tree answers throughout.
#[test]
fn e7i_a_held_tree_answers_for_one_reparse_and_then_nil() {
    let probe = "local t = pmacs.parse.tree(b)\n\
         local before = t:root():type()\n\
         local function reparse(n)\n\
           local len = pmacs.parse.tree(b):source_len()\n\
           b:insert(0, '// x\\n')\n\
           pmacs.parse._dispatch(b, 'rust')\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 20000 do\n\
             pmacs.workers.sleep(50):await()\n\
             local now = pmacs.parse.tree(b)\n\
             if now and now:source_len() ~= len then return end\n\
           end\n\
         end\n\
         reparse()\n\
         local once = t:root():type()\n\
         local current = t:is_current()\n\
         reparse()\n\
         return string.format('before=%s once=%s current=%s twice=%s new=%s', before, tostring(once),\n\
           tostring(current), tostring(t:root():type()), tostring(pmacs.parse.tree(b):root():type()))";
    let report = probe_in_unit("held.rs", "fn main() {}\n", probe);
    assert_eq!(
        report,
        "mode=process before=source_file once=source_file current=false twice=nil new=source_file",
        "a held tree in a unit answers while the unit keeps it, then stops"
    );
}

/// `pmacs.parse._parse_now` parses where a dispatch would (E7i): under
/// process isolation, #296's paragraph is stopped at a 64 MiB unit and the
/// synchronous call returns that error, where in-process it would have
/// grown and returned a tree; a clean buffer's `_parse_now` comes back
/// from its unit with its tree.
#[test]
fn e7i_parse_now_runs_in_the_unit_and_is_held_to_its_limit() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let victim = dir.join("victim.md");
    std::fs::write(&victim, underscores(14)).expect("victim");
    let clean = dir.join("clean.rs");
    std::fs::write(&clean, "fn main() {}\n").expect("clean");
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         pmacs.config.set('syntax.parse-memory-limit-mb', 64)\n\
         local v = pmacs.buffer.find_or_open({victim:?})\n\
         local c = pmacs.buffer.find_or_open({clean:?})\n\
         local ok, err = pcall(pmacs.parse._parse_now, v, 'markdown')\n\
         local t = pmacs.parse._parse_now(c, 'rust')\n\
         local r = pmacs.parse._unit_report(c)\n\
         local f = assert(io.open({report:?}, 'w'))\n\
         f:write(string.format('ok=%s limit=%s clean=%s unit=%s\\n', tostring(ok),\n\
           tostring(tostring(err):find('parse stopped at its memory limit', 1, true) ~= nil),\n\
           tostring(t:root():type()), tostring(r ~= nil and r.mode == 'process' and r.unit ~= '')))\n\
         f:close()\n",
        victim = victim.display().to_string(),
        clean = clean.display().to_string(),
        report = report.display().to_string(),
    );
    let mut daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, Duration::from_mins(1), |t| t.ends_with('\n'));
    assert_eq!(
        text, "ok=false limit=true clean=source_file unit=true\n",
        "the synchronous parse ran in the unit, held to its limit"
    );
    assert!(daemon.is_alive(), "the daemon outlived the stopped parse");
}

/// The TUI grid reads a tree in a parse unit for what it shows (E7i), not
/// the whole file: a real `paint_frame` of an in-process editor whose
/// parses run in a worker styles the first row's `fn`, and the unit is
/// asked for no spans beyond what came back with the parse, where a
/// whole-file request would have fetched every byte of the file.
#[test]
fn e7i_the_grid_reads_what_it_shows_from_the_buffer_s_unit() {
    use std::fmt::Write as _;

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
    let lua = state.lua_host.lua();
    lua.load(format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         pmacs.theme.merge {{ keyword = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}\n\
         pmacs.buffer.find_or_open({:?})",
        path.display().to_string()
    ))
    .exec()
    .expect("open big.rs in process mode");
    let settled = "(function() local b = pmacs.window.buffer() \
                   local r = pmacs.parse._unit_report(b) \
                   return r ~= nil and not r.busy and r.unit ~= '' and pmacs.parse.tree(b) ~= nil end)()";
    let deadline = Instant::now() + Duration::from_mins(1);
    loop {
        state.tick_processes();
        state.tick_async();
        let done: bool = state
            .lua_host
            .lua()
            .load(format!("return ({settled}) == true"))
            .eval()
            .unwrap_or(false);
        if done {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the parse never settled from its unit"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let (rows, cols) = (24u32, 80u32);
    let size = CellSize::new(rows, cols);
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(&state, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    let marked = cells[..cols as usize]
        .iter()
        .filter(|c| c.style.fg == MARK)
        .count();
    let fetched: u64 = state
        .lua_host
        .lua()
        .load("return pmacs.parse._unit_report(pmacs.window.buffer()).fetched")
        .eval()
        .expect("fetched");
    assert_eq!(marked, 2, "row 0's `fn` is styled from the unit's spans");
    assert_eq!(
        fetched,
        0,
        "the grid's rows came back with the parse; nothing more was fetched (the file is {} bytes)",
        text.len()
    );
}

/// Reads never wait on a parse (E7i, the consumer scoping): while the
/// buffer's unit runs #301's openers to a 4 s deadline, a read of the tree
/// the editor shows is answered at once from the unit's installed tree,
/// on the unit's other thread.
#[test]
fn e7i_a_read_is_answered_while_the_unit_parses() {
    let openers =
        std::fs::read_to_string(repo().join("fuzz/regress/markdown/301-nested-openers-98.input"))
            .expect("#301's input");
    let probe = format!(
        "pmacs.config.set('syntax.parse-deadline-ms', 4000)\n\
         local t = pmacs.parse.tree(b)\n\
         b:insert(b:len(), {openers:?})\n\
         pmacs.parse._dispatch(b, 'markdown')\n\
         pmacs.workers.sleep(300):await()\n\
         local busy = pmacs.parse._unit_report(b).busy\n\
         local t0 = pmacs.editor.monotonic_ms()\n\
         local kind = t:root():type()\n\
         local took = pmacs.editor.monotonic_ms() - t0\n\
         local still = pmacs.parse._unit_report(b).busy\n\
         return string.format('busy=%s kind=%s quick=%s still=%s', tostring(busy), tostring(kind),\n\
           tostring(took < 500), tostring(still))"
    );
    let report = probe_in_unit("reads.md", "# A heading\n\nSome *emphasis*.\n", &probe);
    assert_eq!(
        report, "mode=process busy=true kind=document quick=true still=true",
        "the read came back while the parse ran"
    );
}

/// Condition 3: where `RLIMIT_AS` is not used (macOS refuses it; here the
/// measurement hook forces it), the worker's memory watch stops #296 at
/// its 64 MiB allowance, after the fact. How far past the allowance the
/// worker's peak was goes to this test's stderr for the record, and it must
/// stay under the bound below, or the watch is too slow for #296's growth.
#[test]
fn e7i_the_memory_watch_stops_296_and_says_by_how_much() {
    use std::io::Write as _;
    let (mut daemon, _dir, report) = daemon_with_env(
        "process",
        &underscores(28),
        "pmacs.config.set('syntax.parse-memory-limit-mb', 64)\n\
         pmacs.config.set('syntax.parse-deadline-ms', 20000)",
        &[("PMACS_PARSE_UNIT_MEMORY", "watch")],
    );
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, Duration::from_secs(25), victim_died);
    let line = text
        .lines()
        .find(|l| l.starts_with("victim"))
        .expect("victim row");
    let _ = writeln!(std::io::stderr(), "e7i memory watch: {line}");
    let over: u64 = line
        .split("(by watch, ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("#296's worker was stopped by its memory watch: {line}"));
    assert!(
        over < 256 << 20,
        "the watch stopped #296 within 256 MiB of its allowance, not {over} bytes past it"
    );
    assert!(daemon.is_alive(), "the daemon outlived the stopped parse");
}

/// Three #296 buffers parsing at once under a 256 MiB total, each worker
/// allowed 2 GiB: what stops them is the total, held by the cgroup's OOM
/// killer where this session's subtree is delegated (`PMACS_REQUIRE_CGROUP`
/// arms that expectation) and by the editor's watchdog where it is not,
/// or where `force_watchdog` makes it so. The enforcer and the watchdog's
/// overshoot go to this test's stderr; the daemon lives.
fn the_total_holds(force_watchdog: bool) -> String {
    use std::fmt::Write as _;
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let mut opens = String::new();
    for i in 0..3 {
        let path = dir.join(format!("v{i}.md"));
        std::fs::write(&path, underscores(28)).expect("victim");
        let _ = writeln!(
            opens,
            "victims[#victims + 1] = pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        );
    }
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         pmacs.config.set('syntax.parse-memory-limit-mb', 2048)\n\
         pmacs.config.set('syntax.parse-memory-total-mb', 256)\n\
         pmacs.config.set('syntax.parse-deadline-ms', 20000)\n\
         local victims = {{}}\n\
         {opens}\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 60000 do\n\
             pmacs.workers.sleep(50):await()\n\
             local deaths, busy, lines = 0, false, {{}}\n\
             for _, v in ipairs(victims) do\n\
               local r = pmacs.parse._unit_report(v)\n\
               if r then\n\
                 deaths = deaths + r.deaths\n\
                 busy = busy or r.busy\n\
                 lines[#lines + 1] = tostring(r.last_death)\n\
               end\n\
             end\n\
             if deaths >= 1 and not busy then\n\
               local f = assert(io.open({report:?}, 'w'))\n\
               f:write(table.concat(lines, '\\n') .. '\\nreport ' .. tostring(pmacs.parse._isolation_report()) .. '\\n')\n\
               f:close()\n\
               return\n\
             end\n\
           end\n\
         end)\n",
        report = report.display().to_string(),
    );
    let env: &[(&str, &str)] = if force_watchdog {
        &[("PMACS_PARSE_UNIT_CGROUP", "off")]
    } else {
        &[]
    };
    let mut daemon = TestDaemon::spawn_with_env_and_init(env, &init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, Duration::from_mins(1), |t| {
        t.contains("\nreport ")
    });
    let _ = std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("e7i total: {}\n", text.replace('\n', " | ")).as_bytes(),
    );
    assert!(
        text.contains("total: parse stopped at its memory limit"),
        "a worker was stopped by the total: {text}"
    );
    assert!(daemon.is_alive(), "the daemon outlived the stopped parses");
    text
}

#[test]
fn e7i_the_total_holds_three_296_buffers_by_whatever_this_machine_grants() {
    let text = the_total_holds(false);
    if std::env::var_os("PMACS_REQUIRE_CGROUP").is_some() {
        assert!(
            text.contains("(by cgroup)") && !text.contains("(by watchdog)"),
            "PMACS_REQUIRE_CGROUP is armed, so the kernel holds the total: {text}"
        );
    } else {
        assert!(
            text.contains("(by cgroup)") || text.contains("(by watchdog)"),
            "the total is held by the cgroup or the watchdog: {text}"
        );
    }
}

/// The watchdog alone, as macOS and an undelegated Linux get it (the hook
/// keeps the editor from making a cgroup here).
#[test]
fn e7i_the_watchdog_holds_the_total_where_no_cgroup_does() {
    let text = the_total_holds(true);
    assert!(
        text.contains("(by watchdog)")
            && text.contains("the watchdog holds the total")
            && !text.contains("(by cgroup)"),
        "the watchdog held the total: {text}"
    );
}

/// E7i.4: a worker whose peak passed `syntax.parse-worker-recycle-mb` in a
/// parse is replaced at the buffer's next parse. With the threshold at
/// 1 MiB every parse passes it: the first parse after setting it stays in
/// the worker it found, the next runs in a fresh one, while the replaced
/// worker still answers reads of the tree it held, and one parse later
/// that worker is gone.
#[test]
fn e7i_a_worker_past_the_recycle_threshold_is_replaced_at_the_next_parse() {
    let probe = "pmacs.config.set('syntax.parse-worker-recycle-mb', 1)\n\
         local function unit() return pmacs.parse._unit_report(b).unit end\n\
         local function alive(u)\n\
           local p = io.popen('kill -0 ' .. u:match('%d+') .. ' 2>/dev/null; echo $?')\n\
           local code = p:read('*l'); p:close(); return code == '0'\n\
         end\n\
         local function reparse()\n\
           local len = pmacs.parse.tree(b):source_len()\n\
           b:insert(0, '// x\\n')\n\
           pmacs.parse._dispatch(b, 'rust')\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < 20000 do\n\
             pmacs.workers.sleep(50):await()\n\
             local now = pmacs.parse.tree(b)\n\
             if now and now:source_len() ~= len and not pmacs.parse._unit_report(b).busy then return end\n\
           end\n\
         end\n\
         local u0 = unit()\n\
         reparse()\n\
         local u1, t1 = unit(), pmacs.parse.tree(b)\n\
         reparse()\n\
         local u2 = unit()\n\
         local old = t1:root():type()\n\
         reparse()\n\
         local u3 = unit()\n\
         return string.format('same=%s new=%s newer=%s old=%s recycled=%d gone=%s',\n\
           tostring(u1 == u0), tostring(u2 ~= u1), tostring(u3 ~= u2), tostring(old),\n\
           pmacs.parse._unit_report(b).recycled, tostring(not alive(u0)))";
    let report = probe_in_unit("recycle.rs", "fn main() {}\n", probe);
    assert_eq!(
        report, "mode=process same=true new=true newer=true old=source_file recycled=2 gone=true",
        "workers are replaced after a parse past the threshold"
    );
}

/// An ordinary file parses inside a 64 MiB worker (E7i.2): the worker's
/// allowance is its parse's, not its threads' malloc arenas', each of
/// which reserves 64 MiB of the address space `RLIMIT_AS` counts.
#[test]
fn e7i_an_ordinary_file_parses_inside_a_64_mib_worker() {
    let mut text = String::new();
    while text.len() < 100_000 {
        text.push_str(
            "## A heading\n\nSome *emphasis*, a `code span` and a [link](https://example.org).\n\n",
        );
    }
    let probe = "local t = pmacs.parse.tree(b)\n\
         local r = pmacs.parse._unit_report(b)\n\
         return string.format('len=%d deaths=%d', t:source_len(), r.deaths)";
    let report = probe_in_unit_with(
        "ordinary.md",
        &text,
        probe,
        "pmacs.config.set('syntax.parse-memory-limit-mb', 64)",
    );
    assert_eq!(
        report,
        format!("mode=process len={} deaths=0", text.len()),
        "the parse installed inside 64 MiB"
    );
}

/// `pmacs_grammar_fuzz replay-unit --unit <worker> …` with the run's
/// corpus, extras and limits; its exit status and report.
fn replay(unit: &Path, extras: &[&str]) -> (Option<i32>, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = dir.path().join("corpus");
    for (grammar, file) in [("rust", "src/fold.rs"), ("markdown", "docs/invariants.md")] {
        std::fs::create_dir_all(corpus.join(grammar)).expect("corpus");
        std::fs::copy(repo().join(file), corpus.join(grammar).join("seed")).expect("seed");
    }
    let out = dir.path().join("out");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_pmacs_grammar_fuzz"));
    command.args(["replay-unit", "--unit"]).arg(unit);
    command.arg("--corpus").arg(&corpus).arg("--out").arg(&out);
    // The arms' own deadline. At 1 s a loaded machine let the deadline cut
    // #296's inline layer before the debug worker grew to 64 MiB, so the
    // row saw time where it asserts memory (E7i fix round 1: one red in
    // two runs of this suite beside its own parallel rows; at fa176de the
    // same cut read "answered" and failed the counts instead).
    command.args(["--memory-mb", "64", "--deadline-ms", "5000"]);
    for extra in extras {
        command
            .arg("--extra")
            .arg(format!("markdown={}", repo().join(extra).display()));
    }
    let output = command.output().expect("run the replay");
    let report = std::fs::read_to_string(out.join("unit-report.md")).unwrap_or_default();
    (output.status.code(), report)
}

/// E7i.2: the fuzz harness drives the editor's own worker binary through
/// its protocol, a parse then reads of the tree before it while an edit's
/// parse runs; ordinary seeds are answered, #296 and #301 contained by the
/// worker's memory and time limits, and nothing crashed, so it exits 0.
#[test]
fn e7i_the_fuzz_replay_drives_the_worker_and_finds_296_and_301_contained() {
    let worker = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("target dir")
        .join("pmacs-parse-unit");
    let (code, report) = replay(
        &worker,
        &[
            "fuzz/regress/markdown/296-underscores-16k.input",
            "fuzz/regress/markdown/301-nested-openers-98.input",
        ],
    );
    assert_eq!(code, Some(0), "no crash: {report}");
    assert!(report.contains("| markdown | 3 | 1 | 2 | 0 |"), "{report}");
    assert!(report.contains("| rust | 1 | 1 | 0 | 0 |"), "{report}");
    assert!(
        report.contains("296-underscores-16k.input`, memory: RLIMIT_AS")
            || report.contains("296-underscores-16k.input`, memory: the worker's watch"),
        "#296 contained by its memory limit: {report}"
    );
    assert!(
        report.contains("301-nested-openers-98.input`, time: "),
        "#301 contained by its time limit: {report}"
    );
}

/// And a worker that crashes fails the replay: a stand-in that answers
/// the protocol's Hello and dies by SIGSEGV at the first parse.
#[test]
fn e7i_the_fuzz_replay_fails_a_worker_that_crashes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fake = dir.path().join("crashing-unit");
    // Hello's request frame is 10 bytes; the answer is (1, Hello { this
    // build's protocol }), each a one-byte varint.
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
    let (code, report) = replay(&fake, &[]);
    assert_eq!(code, Some(1), "a crash fails the replay: {report}");
    assert!(
        report.contains("CRASHED") && report.contains("signal 11"),
        "{report}"
    );
}

/// A missing worker is said once, with the ways out (E7i): with
/// `syntax.parse-unit-path` naming no file, no buffer can be parsed, and
/// the user is told once a session, however many buffers try, that nothing
/// is highlighted, why, and what to do: keep the worker the release archive
/// ships beside pmacs, or name it in `syntax.parse-unit-path`. Since 2.0.0
/// those are the only two, and the notice no longer offers
/// `syntax.isolation`'s `none`, which that release removed.
#[test]
fn e7i_a_missing_worker_is_said_once_with_its_remedy() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let a = dir.join("a.rs");
    let b = dir.join("b.rs");
    std::fs::write(&a, "fn a() {}\n").expect("a");
    std::fs::write(&b, "fn b() {}\n").expect("b");
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         pmacs.config.set('syntax.parse-unit-path', {missing:?})\n\
         pmacs.buffer.find_or_open({a:?})\n\
         pmacs.buffer.find_or_open({b:?})\n\
         pmacs.async(function()\n\
           pmacs.workers.sleep(3000):await()\n\
           local said = ''\n\
           for _, x in ipairs(pmacs.buffer.list()) do\n\
             if x:name() == '*errors*' then said = x:slice(0, x:len()) end\n\
           end\n\
           local _, told = said:gsub('so nothing is highlighted', '')\n\
           local f = assert(io.open({report:?}, 'w'))\n\
           f:write(string.format('told=%d path=%s together=%s named=%s archive=%s none=%s\\n', told,\n\
             tostring(said:find('does not exist', 1, true) ~= nil),\n\
             tostring(said:find('keep the two in one directory', 1, true) ~= nil),\n\
             tostring(said:find('name the worker in syntax.parse-unit-path', 1, true) ~= nil),\n\
             tostring(said:find('ships beside pmacs in the release archive', 1, true) ~= nil),\n\
             tostring(said:find('syntax.isolation', 1, true) ~= nil)))\n\
           f:close()\n\
         end)\n",
        missing = dir.join("no-such-worker").display().to_string(),
        a = a.display().to_string(),
        b = b.display().to_string(),
        report = report.display().to_string(),
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, Duration::from_secs(30), |t| {
        t.ends_with('\n')
    });
    assert_eq!(
        text, "told=1 path=true together=true named=true archive=true none=false\n",
        "told once, naming the missing path and the two ways out, the first one \
         the release archive itself (E7i review 1: the remedy named a binary \
         the archive did not carry), and no third: no setting parses in the \
         editor since 2.0.0"
    );
}

/// E7i.5: #296's row named its removal as "a limit on the parse itself
/// stops #296's 32 KB paragraph in the editor". Here it is, at the editor's
/// own defaults (no limit set in the test): the 32 KB paragraph, which in
/// the editor's process grew to 9.8 GB, is stopped by whichever of the two
/// limits it meets first, the user told once, the daemon alive. On this
/// laptop that is the worker's 1 GiB allowance (`RLIMIT_AS` on Linux, the
/// watch on macOS); on CI's slower runners the debug build grows slower
/// than the 5 s deadline allows it to reach 1 GiB, and the deadline's kill
/// stops it first (PR #309's run 37133953154, `luajit, no crdt`). Either is
/// a limit on the parse itself; the 64 MiB rows witness the memory limit
/// alone. Which one stopped it goes to the test's stderr.
#[test]
fn e7i_296s_32_kb_paragraph_is_stopped_at_the_editor_s_defaults() {
    let (mut daemon, _dir, report) = daemon("process", &underscores(56), "");
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, Duration::from_secs(40), |t| {
        victim_died(t) && t.contains("told 1")
    });
    let victim = text
        .lines()
        .find(|l| l.starts_with("victim"))
        .unwrap_or_default();
    let _ = std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("e7i 296 at defaults: {victim}\n").as_bytes(),
    );
    assert!(
        victim.contains("death=memory:") || victim.contains("death=time:"),
        "#296's 32 KB paragraph was stopped by a limit on the parse itself: {text}"
    );
    assert!(daemon.is_alive(), "the daemon outlived the stopped parse");
}

/// E7i.5: #301's row named its removal as "a time limit on the parse
/// itself stops #301's nested openers in the editor". At the editor's
/// default deadline, 5 s, the worker is killed 100 ms past it.
#[test]
fn e7i_301_is_stopped_at_the_editor_s_default_deadline() {
    let victim =
        std::fs::read_to_string(repo().join("fuzz/regress/markdown/301-nested-openers-98.input"))
            .expect("#301's input");
    let (mut daemon, _dir, report) = daemon("process", &victim, "");
    let log = PathBuf::from(format!("{}.stderr.log", daemon.socket_path().display()));
    let text = wait_report(&report, &log, Duration::from_secs(40), |t| {
        victim_died(t) && t.contains("tree=rust")
    });
    assert!(
        text.lines()
            .find(|l| l.starts_with("victim"))
            .is_some_and(|l| l.contains("death=time:") && l.contains("deadline of 5000 ms")),
        "#301 was stopped at the default deadline: {text}"
    );
    assert!(daemon.is_alive(), "the daemon outlived the stopped parse");
}

/// A killed buffer's worker goes with it (E7i): with the worker the
/// default, a long-lived editor must not keep a process per buffer it ever
/// opened. The worker that parsed the buffer is alive, the buffer is
/// killed, and the worker is gone and the buffer has no unit any more.
#[test]
fn e7i_killing_a_buffer_ends_its_worker() {
    let probe = "local pid = pmacs.parse._unit_report(b).unit:match('%d+')\n\
         local function alive()\n\
           local p = io.popen('kill -0 ' .. pid .. ' 2>/dev/null; echo $?')\n\
           local code = p:read('*l'); p:close(); return code == '0'\n\
         end\n\
         local before = alive()\n\
         pmacs.buffer.kill(b)\n\
         local t0 = pmacs.editor.monotonic_ms()\n\
         while alive() and pmacs.editor.monotonic_ms() - t0 < 5000 do\n\
           pmacs.workers.sleep(20):await()\n\
         end\n\
         return string.format('before=%s after=%s unit=%s', tostring(before), tostring(alive()),\n\
           tostring(pmacs.parse._unit_report(b)))";
    let report = probe_in_unit("killed.rs", "fn main() {}\n", probe);
    assert_eq!(
        report, "mode=process before=true after=false unit=nil",
        "the worker ended with its buffer"
    );
}

/// The deadline bounds the parse, not what follows it (E7i): a fresh
/// worker's first request also compiles the language's queries and walks
/// the spans, which in-process ran at settle and under no deadline. A small
/// LaTeX document's parse returns in under a millisecond in a debug worker
/// and its first request answers after about half a second, almost all of
/// it compiling LaTeX's highlight query (measured: 0.3–0.6 ms against
/// 486–614 ms). Under a 50 ms deadline the request installs: the worker says
/// when its parse returned, and the editor's kill bounds only what came
/// before.
#[test]
fn e7i_a_fresh_worker_s_query_compile_is_not_under_the_deadline() {
    let probe = "local r = pmacs.parse._unit_report(b)\n\
         return string.format('deaths=%d tree=%s', r.deaths, pmacs.parse.tree(b):language())";
    let report = probe_in_unit_with(
        "fresh.tex",
        "\\documentclass{article}\n\\begin{document}\nHello \\emph{world}.\n\\end{document}\n",
        probe,
        "pmacs.config.set('syntax.parse-deadline-ms', 50)",
    );
    assert_eq!(
        report, "mode=process deaths=0 tree=latex",
        "the first parse installed under a 50 ms deadline"
    );
}
