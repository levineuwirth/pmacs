// tests/autosave_periodic_acceptance.rs --- #368, the periodic sweep's syncs wait on no key.

//! Autosave's periodic sweep writes its recovery copies without the
//! editor waiting on their syncs, and nothing it does grows, reorders or
//! hides what #365 made quit keep.
//!
//! **What #368 was, measured.** Every `autosave.interval-ms` (30 s by
//! default) `autosave.lua`'s tick ran `pmacs.autosave.sweep`, which
//! wrote each modified buffer's copy through `state::write_private`: the
//! temporary file synced, renamed over its key, the directory synced, on
//! the main thread. Under a writer of 2 GB files with `fdatasync` on this
//! laptop's btrfs a sweep after typing held the editor up to 8.4 s, keys
//! typed and unechoed meanwhile, at every interval in which something
//! was typed. The sweep also began by joining the thread #365 installs
//! quit's copies on, so a sweep falling due while a cancelled quit's copy
//! synced held the editor as long. A sweep now writes each copy's
//! temporary file and hands the copies to one thread that installs them
//! by the same calls; a sweep falling due while copies install is
//! deferred to the tick that takes them, and nothing on the main thread
//! waits for that thread but the editor's drop, a forced
//! `pmacs.autosave.sweep()` and what `recover-file` reads and claims.
//!
//! **What the rows assert.** What the editor did and in what order,
//! never how long it took. A disk slower than anything a row waits on is
//! made by `strace` itself, holding each sync at its entry
//! (`inject=…:delay_enter`), and a failing one by `inject=…:error=EIO`;
//! one `strace -f` log orders the editor's threads. Typing goes on while
//! a copy syncs; a cancelled quit's copy installing does not hold the
//! editor when a sweep falls due; on a disk slower than the interval one
//! batch installs at a time and no second is staged behind it; quit asks
//! at once while a periodic copy installs; an install in flight at exit
//! is waited for and said; a sync that failed on the thread is named in
//! the next quit question; a kill while its copy installs leaves no copy
//! behind. [`HANG_GUARD`] is there for an exit that never comes and is
//! not a budget.
//!
//! **Reading the editor.** An `init.lua` hook records, each frame it
//! changes, the buffer's first line, how many copies are not yet at
//! their keys, the last failure, the minibuffer's prompt and what a
//! row's own hooks did. It writes
//! the record with one `write`, which the `strace` log shows on the
//! editor's main thread with its text, so a row orders the editor's
//! response to a key against a thread's sync.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;
mod support;

use common::pty::{PmacsPty, spawn_pmacs_in_pty_under};

/// How long a row waits for the editor to exit once it should. A guard,
/// not a budget: no row reads the time against it.
const HANG_GUARD: Duration = Duration::from_mins(1);

/// From spawn to each state a row waits for.
const READY_BOUND: Duration = Duration::from_mins(1);

const TEXT: &str = "one\ntwo\nthree\n";

/// The fastest sweep autosave allows.
const INTERVAL_MS: u64 = 1000;

/// A word of the quit question, and of nothing else the rows show.
const ASKED: &str = "anyway?";

/// How `strace` stands in for the disk.
#[derive(Clone, Copy)]
enum Disk {
    /// Every sync held this long at its entry.
    Slow(Duration),
    /// Every `fsync` fails with `EIO`.
    Failing,
}

/// One row's world: a text file, an `init.lua`, a state root of its own.
struct World {
    dir: tempfile::TempDir,
}

impl World {
    /// `extra` is Lua appended to the row's `init.lua`; `interval_ms`
    /// is autosave's interval as the editor starts.
    fn new(interval_ms: u64, extra: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("notes.txt"), TEXT).expect("write notes.txt");
        let config = dir.path().join("config").join("pmacs");
        std::fs::create_dir_all(&config).expect("config dir");
        std::fs::create_dir_all(dir.path().join("state")).expect("state dir");
        let status = dir.path().join("status");
        let init = format!(
            r#"
pmacs.lsp.config = {{}}
pmacs.autosave.interval_ms({interval_ms})
-- Each frame: the buffer's first line, whether it is modified, how many
-- copies are not yet at their keys and the most there have been, the
-- last failure autosave reported, the minibuffer's prompt, and what the
-- row's own hooks did; written with one `write` when it changes.
local last
local most = 0
pmacs.hook.add("process.after-tick", function()
  local buf = pmacs.window.buffer()
  local d = buf and pmacs.describe.buffer(buf)
  local head = buf and buf:slice(0, math.min(buf:len(), 200)) or ""
  local pending = #pmacs.autosave._staged()
  if pending > most then most = pending end
  local prompt = pmacs.minibuffer.is_active() and pmacs.minibuffer.prompt() or ""
  local text = "line=" .. (head:match("^[^\n]*") or "")
    .. "\nmodified=" .. tostring(d and d.modified)
    .. "\npending=" .. tostring(pending)
    .. "\nmost=" .. tostring(most)
    .. "\nfailed=" .. tostring(pmacs.autosave.failure() ~= nil)
    .. "\narmed=" .. tostring(_G.row_armed == true)
    .. "\ndue=" .. tostring(_G.row_due == true)
    .. "\nprompt=" .. prompt .. "\n"
  if text ~= last then
    last = text
    local f = io.open({part:?}, "w")
    f:write(text)
    f:close()
    os.rename({part:?}, {status:?})
  end
end)
{extra}
"#,
            part = format!("{}.part", status.display()),
            status = status.display().to_string(),
        );
        std::fs::write(config.join("init.lua"), init).expect("write init.lua");
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// The question quit asks about `notes.txt`, as the minibuffer holds
    /// it: the buffer is named by the path the editor was given.
    fn question(&self) -> String {
        format!(
            "Modified buffers exist ({})",
            self.path("notes.txt").display()
        )
    }

    /// The directory autosave keeps its copies in, canonical.
    fn autosave_dir(&self) -> PathBuf {
        std::fs::canonicalize(self.path("state"))
            .expect("canonical state root")
            .join("pmacs")
            .join("autosave")
    }

    /// Run the editor on `notes.txt` under `strace -f`, which logs to
    /// `trace.strace` every write, sync, rename and unlink the editor's
    /// threads make, with each descriptor's path, and stands in for the
    /// disk as `disk` says. `--kill-on-exit`: the editor dies with it.
    fn spawn_traced(&self, disk: Disk) -> PmacsPty {
        let log = self.path("trace.strace").display().to_string();
        let inject = match disk {
            Disk::Slow(hold) => format!("fsync,fdatasync:delay_enter={}", hold.as_micros()),
            Disk::Failing => "fsync:error=EIO".to_owned(),
        };
        let file = self.path("notes.txt").display().to_string();
        let envs = [
            ("XDG_CONFIG_HOME", self.path("config")),
            ("XDG_STATE_HOME", self.path("state")),
            ("PMACS_STATE_HOME", self.path("state")),
        ];
        let envs: Vec<(&str, &Path)> = envs.iter().map(|(k, v)| (*k, v.as_path())).collect();
        spawn_pmacs_in_pty_under(
            &[
                "strace",
                "-f",
                "--kill-on-exit",
                "--seccomp-bpf",
                "-qq",
                "-y",
                "-s",
                "4096",
                "-e",
                "trace=execve,write,fsync,fdatasync,rename,renameat,renameat2,unlink,unlinkat",
                "-e",
                &format!("inject={inject}"),
                "-o",
                &log,
            ],
            &[&file],
            &envs,
            40,
            200,
        )
    }

    fn trace(&self) -> Trace {
        Trace::new(std::fs::read_to_string(self.path("trace.strace")).expect("read the strace log"))
    }

    /// The hook's last record, or empty before its first frame.
    fn record(&self) -> String {
        std::fs::read_to_string(self.path("status")).unwrap_or_default()
    }

    /// The value of `key` in the hook's last record.
    fn field(&self, key: &str) -> String {
        let prefix = format!("{key}=");
        self.record()
            .lines()
            .find_map(|l| l.strip_prefix(prefix.as_str()))
            .unwrap_or_default()
            .to_owned()
    }

    /// Poll until `ready` holds of the hook's record.
    fn wait_for(&self, pty: &mut PmacsPty, what: &str, ready: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + READY_BOUND;
        loop {
            let record = self.record();
            if ready(&record) {
                return;
            }
            assert!(
                pty.wait_for_exit(Duration::ZERO).is_none(),
                "the editor exited while waiting for {what}; last record:\n{record}"
            );
            assert!(
                Instant::now() < deadline,
                "precondition: {what} within {READY_BOUND:?}; last record:\n{record}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Poll until the hook's record holds every line of `want`.
    fn wait(&self, pty: &mut PmacsPty, what: &str, want: &[&str]) {
        self.wait_for(pty, what, |record| {
            want.iter().all(|w| record.lines().any(|l| l == *w))
        });
    }

    /// Type `C-x C-c` and wait until quit's question is up and the
    /// terminal has been sent it.
    fn quit_and_see_the_question(&self, pty: &mut PmacsPty) {
        let shown = asked_count(pty);
        pty.write_input(b"\x18\x03").expect("type C-x C-c");
        let question = self.question();
        self.wait_for(pty, "the quit question", |record| {
            record.lines().any(|l| {
                l.strip_prefix("prompt=")
                    .is_some_and(|p| p.starts_with(&question))
            })
        });
        let deadline = Instant::now() + READY_BOUND;
        while asked_count(pty) <= shown {
            assert!(
                Instant::now() < deadline,
                "the terminal is sent the question within {READY_BOUND:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The recovery copy autosave holds for `notes.txt`, the text after
    /// its header line. `None` when there is none.
    fn recovery_copy(&self) -> Option<String> {
        for entry in std::fs::read_dir(self.autosave_dir()).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".pmacs-tmp.") {
                continue;
            }
            let bytes = std::fs::read(entry.path()).expect("read a recovery copy");
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let (header, body) = text.split_once('\n').expect("a recovery copy has a header");
            if header.contains("notes.txt") {
                return Some(body.to_owned());
            }
        }
        None
    }

    /// Everything in the autosave directory, temporary files included.
    fn autosave_entries(&self) -> Vec<String> {
        std::fs::read_dir(self.autosave_dir())
            .map(|dir| {
                dir.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// End the editor where it stands: `SIGKILL` to it, not to `strace`,
/// which then exits on its own with every line written.
fn kill_editor(pty: &mut PmacsPty) {
    let tracer = pty.process_id().expect("strace's process id");
    let editor = child_of(tracer).expect("the editor is strace's child");
    let killed = std::process::Command::new("kill")
        .args(["-KILL", &editor.to_string()])
        .status()
        .expect("run kill");
    assert!(killed.success(), "kill -KILL {editor}");
    assert!(
        pty.wait_for_exit(HANG_GUARD).is_some(),
        "strace exits once the editor is gone, within the hang guard {HANG_GUARD:?}"
    );
}

/// The process whose parent is `parent`, read from `/proc`.
fn child_of(parent: u32) -> Option<u32> {
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
        let (_, rest) = stat.rsplit_once(") ")?;
        let ppid: u32 = rest.split_whitespace().nth(1)?.parse().ok()?;
        (ppid == parent).then_some(pid)
    })
}

/// How many times the terminal has been sent the quit question's word.
fn asked_count(pty: &PmacsPty) -> usize {
    String::from_utf8_lossy(&pty.output())
        .matches(ASKED)
        .count()
}

/// Wait for a clean exit, printing how long it took as a measurement.
fn exits(pty: &mut PmacsPty) {
    let started = Instant::now();
    let Some(status) = pty.wait_for_exit(HANG_GUARD) else {
        panic!("the editor did not exit within {HANG_GUARD:?}, a hang guard");
    };
    assert!(status.success(), "quit is a clean exit, got {status:?}");
    eprintln!(
        "exit took {:?} after the answer (a measurement, not asserted)",
        started.elapsed()
    );
}

/// Whether `strace` runs here and can do what the rows ask of it:
/// follow threads, kill its tracee on exit, filter by seccomp and hold a
/// sync. A row that cannot use it records its skip, and
/// `PMACS_REQUIRE_STRACE` makes that a failure.
fn strace_or_skip() -> bool {
    let usable = match std::process::Command::new("strace")
        .args([
            "-f",
            "--kill-on-exit",
            "--seccomp-bpf",
            "-qq",
            "-e",
            "trace=fsync",
            "-e",
            "inject=fsync:delay_enter=1",
            "-o",
            "/dev/null",
            "true",
        ])
        .output()
    {
        Err(e) => Err(format!("`strace` not runnable: {e}")),
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "`strace` cannot trace a child here as the rows ask: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    };
    match usable {
        Ok(()) => true,
        Err(why) => {
            assert!(
                std::env::var_os("PMACS_REQUIRE_STRACE").is_none_or(|v| v.is_empty()),
                "PMACS_REQUIRE_STRACE is set, but {why}"
            );
            support::record_skip(&format!("{why}, and PMACS_REQUIRE_STRACE unset"));
            eprintln!("{why}; skipping (set PMACS_REQUIRE_STRACE to make this fatal)");
            false
        }
    }
}

/// One `strace -f` log, its lines in the order `strace` met them.
struct Trace {
    log: String,
}

impl Trace {
    fn new(log: String) -> Self {
        Self { log }
    }

    fn lines(&self) -> Vec<&str> {
        self.log.lines().collect()
    }

    /// The thread a line belongs to.
    fn tid(line: &str) -> Option<u32> {
        line.split_whitespace().next()?.parse().ok()
    }

    /// The editor's main thread: the one that ran `execve`.
    fn main_tid(&self) -> u32 {
        self.lines()
            .iter()
            .find(|l| l.contains(" execve("))
            .and_then(|l| Self::tid(l))
            .unwrap_or_else(|| {
                panic!(
                    "positive control: the log holds the editor's execve\n{}",
                    self.log
                )
            })
    }

    /// The indexes of the sync calls' entries on a recovery copy's
    /// temporary file, or on the autosave directory itself, in log
    /// order. An entry another thread interrupted is logged
    /// `<unfinished ...>`.
    fn copy_syncs(&self, autosave: &Path, temp: bool) -> Vec<usize> {
        let needle = if temp {
            format!("<{}/.pmacs-tmp.", autosave.display())
        } else {
            format!("<{}>", autosave.display())
        };
        self.lines()
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                ["fsync(", "fdatasync("].iter().any(|c| l.contains(c)) && l.contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Where the call entered at `entry` returned: the entry itself when
    /// it was logged whole, else its thread's `<... resumed>` line, else
    /// past the end (the editor was killed inside it).
    fn returned(&self, entry: usize) -> usize {
        let lines = self.lines();
        let line = lines[entry];
        if !line.ends_with("<unfinished ...>") {
            return entry;
        }
        let tid = Self::tid(line);
        let call = line
            .split_whitespace()
            .nth(1)
            .and_then(|w| w.split('(').next())
            .unwrap_or_default();
        let resumed = format!("<... {call} resumed>");
        lines
            .iter()
            .enumerate()
            .skip(entry + 1)
            .find(|(_, l)| Self::tid(l) == tid && l.contains(&resumed))
            .map_or(lines.len(), |(i, _)| i)
    }

    /// The indexes of `tid`'s writes of the hook's record holding every
    /// one of `want`.
    fn records(&self, tid: u32, want: &[&str]) -> Vec<usize> {
        self.lines()
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                Self::tid(l) == Some(tid)
                    && l.contains(" write(")
                    && l.contains("/status.part>")
                    && want.iter().all(|w| l.contains(w))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Whether `tid` wrote to the terminal strictly between `from` and
    /// `to`.
    fn painted_between(&self, tid: u32, from: usize, to: usize) -> bool {
        self.lines()
            .iter()
            .enumerate()
            .take(to)
            .skip(from + 1)
            .any(|(_, l)| Self::tid(l) == Some(tid) && l.contains(" write(1<"))
    }
}

/// **Typing goes on while a periodic copy syncs** (the fix). One key
/// edits the buffer; within the interval the sweep hands its copy to the
/// installing thread, which `strace` holds in the copy's first sync. A
/// second key, typed once the record shows the copy installing, is in
/// the editor's record, written by its main thread, and painted, before
/// that sync returns; and the sync was not the main thread's.
///
/// Fails at `1448046`, whose sweep synced on the main thread: no copy is
/// ever installing (the precondition), and the first sync of the copy is
/// the main thread's.
#[test]
fn typing_goes_on_while_a_periodic_copy_syncs() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(INTERVAL_MS, "");
    let mut pty = world.spawn_traced(Disk::Slow(Duration::from_secs(20)));
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"a").expect("type a");
    world.wait(
        &mut pty,
        "the sweep's copy installing",
        &["line=aone", "pending=1"],
    );
    pty.write_input(b"b").expect("type b");
    world.wait(&mut pty, "the second key", &["line=abone"]);
    kill_editor(&mut pty);

    let trace = world.trace();
    let main = trace.main_tid();
    let entry = *trace
        .copy_syncs(&world.autosave_dir(), true)
        .first()
        .unwrap_or_else(|| {
            panic!(
                "the recovery copy's temporary file is synced\n{}",
                trace.log
            )
        });
    let lines = trace.lines();
    assert_ne!(
        Trace::tid(lines[entry]),
        Some(main),
        "the recovery copy is synced on the editor's main thread: {}",
        lines[entry]
    );
    let returned = trace.returned(entry);
    let typed = *trace
        .records(main, &["line=abone"])
        .first()
        .unwrap_or_else(|| {
            panic!(
                "positive control: the main thread records the second key\n{}",
                trace.log
            )
        });
    assert!(
        entry < typed && typed < returned,
        "the editor took the key typed while the copy synced, before the sync returned: \
         sync entered at log line {entry}, returned at {returned}, key recorded at {typed}\n{}",
        trace.log
    );
    assert!(
        trace.painted_between(main, typed, returned),
        "the editor painted the key before the copy's sync returned\n{}",
        trace.log
    );
}

/// **Quit asks at once while a periodic copy installs** (#365's
/// promise kept under #368). A periodic copy is installing, held in its
/// first sync, when a second key changes the buffer and `C-x C-c` asks:
/// quit stages its newer copy behind the installing one without waiting
/// for it, so the question is recorded by the main thread, and painted,
/// before that sync returns.
///
/// Fails at `1448046`, where no periodic copy is ever installing (the
/// precondition); fails with quit's staging joining the installer first:
/// the question waits for the sync.
#[test]
fn quit_asks_at_once_while_a_periodic_copy_installs() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(INTERVAL_MS, "");
    let mut pty = world.spawn_traced(Disk::Slow(Duration::from_secs(20)));
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"a").expect("type a");
    world.wait(
        &mut pty,
        "the sweep's copy installing",
        &["line=aone", "pending=1"],
    );
    pty.write_input(b"b").expect("type b");
    world.wait(&mut pty, "the second key", &["line=abone"]);
    world.quit_and_see_the_question(&mut pty);
    kill_editor(&mut pty);

    let trace = world.trace();
    let main = trace.main_tid();
    let entry = *trace
        .copy_syncs(&world.autosave_dir(), true)
        .first()
        .unwrap_or_else(|| panic!("the periodic copy is synced\n{}", trace.log));
    let returned = trace.returned(entry);
    let asked = *trace
        .records(main, &["prompt=Modified buffers exist"])
        .first()
        .unwrap_or_else(|| panic!("positive control: the question, recorded\n{}", trace.log));
    assert!(
        entry < asked && asked < returned,
        "quit's question waited on the installing copy's sync: sync entered at log line \
         {entry}, question recorded at {asked}, sync returned at {returned}\n{}",
        trace.log
    );
    assert!(
        trace.painted_between(main, asked, returned),
        "the question was painted before the installing copy's sync returned\n{}",
        trace.log
    );
}

/// **A cancelled quit's copy installing does not hold the editor when a
/// sweep falls due** (#365's narrow case, a finding at `1448046`). Text
/// typed, `C-x C-c`, `n`: quit's copy goes to its thread, which `strace`
/// holds in its first sync. The row's `init.lua` shortens the interval
/// to the floor once the question is gone, and records the sweep due
/// once that much time has passed since autosave's clock started, its
/// own first tick. With the sweep due and the copy still installing on
/// the record, a key typed is recorded by the main thread before the
/// copy's sync returns.
///
/// Fails at `1448046`: the sweep that falls due begins by settling,
/// which joins the installing thread on the main thread, so the editor
/// records the sweep due only once the copy is taken.
#[test]
fn a_cancelled_quits_copy_installing_does_not_hold_a_sweep_falling_due() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(
        600_000,
        "pmacs.hook.add('editor.before-quit', function() _G.row_quit = true end)\n\
         local first\n\
         pmacs.hook.add('process.after-tick', function()\n\
           local now = pmacs.editor.monotonic_ms()\n\
           first = first or now\n\
           if _G.row_quit and not _G.row_armed and not pmacs.minibuffer.is_active() then\n\
             _G.row_armed = true\n\
             pmacs.autosave.interval_ms(1000)\n\
           end\n\
           if _G.row_armed and now - first >= 1000 then _G.row_due = true end\n\
         end)",
    );
    let mut pty = world.spawn_traced(Disk::Slow(Duration::from_secs(20)));
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"a").expect("type a");
    world.wait(&mut pty, "the edit", &["line=aone"]);

    world.quit_and_see_the_question(&mut pty);
    pty.write_input(b"n").expect("answer n");
    world.wait(
        &mut pty,
        "the quit cancelled, its copy installing and a sweep due",
        &["prompt=", "pending=1", "armed=true", "due=true"],
    );
    pty.write_input(b"b").expect("type b");
    world.wait(&mut pty, "the key after the cancel", &["line=abone"]);
    kill_editor(&mut pty);

    let trace = world.trace();
    let main = trace.main_tid();
    let entry = *trace
        .copy_syncs(&world.autosave_dir(), true)
        .first()
        .unwrap_or_else(|| panic!("the quit's copy is synced\n{}", trace.log));
    let returned = trace.returned(entry);
    let due = *trace
        .records(main, &["due=true", "pending=1"])
        .first()
        .unwrap_or_else(|| {
            panic!(
                "positive control: a sweep due while the copy installed, recorded\n{}",
                trace.log
            )
        });
    let typed = *trace
        .records(main, &["line=abone"])
        .first()
        .unwrap_or_else(|| {
            panic!(
                "positive control: the key after the cancel, recorded\n{}",
                trace.log
            )
        });
    assert!(
        entry < typed && due < typed && typed < returned,
        "the sweep that fell due while the cancelled quit's copy synced held the editor: \
         sync entered at log line {entry}, sweep due at {due}, key recorded at {typed}, \
         sync returned at {returned}\n{}",
        trace.log
    );
}

/// **On a disk slower than the interval one batch installs at a time and
/// none is staged behind it**, and the copy still ends with every key.
/// Each sync is held 1.5 s, so a copy takes 3 s against a 1 s interval,
/// and forty keys go in, one every 200 ms. The hook's record never counts
/// more than one copy not at its key; in the log, no copy's sync begins
/// while another copy's is unfinished. Then `C-x C-c y`: the copy holds
/// the text as typed.
///
/// Fails at `1448046` at its positive control, no copy ever installing
/// on a thread; fails with the sweep staging while a batch installs:
/// two copies not at their keys.
#[test]
fn a_disk_slower_than_the_interval_installs_one_batch_and_stages_none_behind_it() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(INTERVAL_MS, "");
    let mut pty = world.spawn_traced(Disk::Slow(Duration::from_millis(1500)));
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    let mut typed = String::new();
    for _ in 0..40 {
        pty.write_input(b"x").expect("type x");
        typed.push('x');
        std::thread::sleep(Duration::from_millis(200));
    }
    world.wait(
        &mut pty,
        "every key in the buffer",
        &[&format!("line={typed}one")],
    );
    let most: usize = world.field("most").parse().expect("the record's count");
    assert!(
        most >= 1,
        "positive control: a copy was recorded installing on its thread"
    );
    assert!(
        most <= 1,
        "at most one copy is ever not at its key on a disk slower than the interval, \
         {most} were"
    );

    world.quit_and_see_the_question(&mut pty);
    pty.write_input(b"y").expect("answer y");
    exits(&mut pty);

    let trace = world.trace();
    let autosave = world.autosave_dir();
    let lines = trace.lines();
    let mut syncs: Vec<(usize, usize)> = trace
        .copy_syncs(&autosave, true)
        .into_iter()
        .chain(trace.copy_syncs(&autosave, false))
        .map(|entry| (entry, trace.returned(entry)))
        .collect();
    syncs.sort_unstable();
    assert!(
        syncs.len() >= 4,
        "positive control: more than one copy was synced, {} sync calls\n{}",
        syncs.len(),
        trace.log
    );
    for pair in syncs.windows(2) {
        let ((_, returned), (next, _)) = (pair[0], pair[1]);
        assert!(
            returned < next,
            "a copy's sync began while another's was unfinished: {} / {}\n{}",
            lines[pair[0].0],
            lines[next],
            trace.log
        );
    }
    assert_eq!(
        world.recovery_copy().as_deref(),
        Some(format!("{typed}{TEXT}").as_str()),
        "the copy holds every key typed"
    );
}

/// **An install in flight at exit is waited for, and said** (the
/// editor's drop, unchanged by #368 and held here). A periodic copy is
/// installing, held in its first sync, when `C-x C-c y` arrives in one
/// write. The exit says on stderr that it is writing the copy before
/// the copy's rename, the copy is synced and in place, and it holds the
/// text.
///
/// Fails at `1448046`, where no periodic copy is ever installing (the
/// precondition); fails with the drop's settle removed: no line, and no
/// copy at its key.
#[test]
fn a_periodic_install_in_flight_at_exit_is_waited_for_and_said() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(INTERVAL_MS, "");
    let mut pty = world.spawn_traced(Disk::Slow(Duration::from_secs(10)));
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"abc").expect("type abc");
    world.wait(
        &mut pty,
        "the sweep's copy installing",
        &["line=abcone", "pending=1"],
    );
    pty.write_input(b"\x18\x03y")
        .expect("type C-x C-c y in one write");
    exits(&mut pty);

    let trace = world.trace();
    let lines = trace.lines();
    let said = "pmacs: quitting: writing the recovery copy of ";
    let told = lines
        .iter()
        .position(|l| l.contains("write(2<") && l.contains(said) && l.contains("notes.txt to disk"))
        .unwrap_or_else(|| {
            panic!(
                "the exit says what it waits on, `{said}…notes.txt to disk`:\n{}",
                trace.log
            )
        });
    let autosave = world.autosave_dir();
    let entry = *trace
        .copy_syncs(&autosave, true)
        .first()
        .unwrap_or_else(|| panic!("the copy is synced\n{}", trace.log));
    let renamed = lines
        .iter()
        .position(|l| l.contains("rename(") && l.contains(&format!("\"{}/", autosave.display())))
        .unwrap_or_else(|| panic!("the copy is renamed over its key\n{}", trace.log));
    assert!(
        entry < told && told < renamed,
        "the exit said it was writing the copy while it installed: sync entered at log line \
         {entry}, said at {told}, renamed at {renamed}\n{}",
        trace.log
    );
    assert!(
        !trace.copy_syncs(&autosave, false).is_empty(),
        "the autosave directory is synced after the rename\n{}",
        trace.log
    );
    assert_eq!(
        world.recovery_copy().as_deref(),
        Some(format!("abc{TEXT}").as_str()),
        "the copy holds the text typed"
    );
}

/// **A sync that failed on the installing thread is named in the next
/// quit question.** Every `fsync` fails with `EIO`, a sync failing where
/// its write did not: the sweep writes its copy and its install fails,
/// met on the thread and reported at the tick that takes it. `C-x C-c` asks,
/// and the question says autosave FAILED, though quit's own write of
/// the copy succeeded.
///
/// Fails at `1448046`, where quit's staging succeeded and so cleared the
/// failure before the question read it: the question was the bare one.
#[test]
fn a_sync_that_failed_on_the_thread_is_named_in_the_next_question() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(INTERVAL_MS, "");
    let mut pty = world.spawn_traced(Disk::Failing);
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"abc").expect("type abc");
    world.wait(
        &mut pty,
        "the sweep's failure reported",
        &["line=abcone", "failed=true", "pending=0"],
    );

    world.quit_and_see_the_question(&mut pty);
    let named = format!(
        "{}; autosave FAILED, see *errors*; quit anyway? (y or n) ",
        world.question()
    );
    assert_eq!(
        world.field("prompt"),
        named,
        "the question names the failed sync"
    );
    let shown = String::from_utf8_lossy(&pty.output()).into_owned();
    assert!(
        shown.contains("FAILED,"),
        "the terminal is sent the failure with the question"
    );
    pty.write_input(b"y").expect("answer y");
    exits(&mut pty);
}

/// **A kill while its copy installs leaves no copy behind.** A periodic
/// copy is installing, held in its first sync, when `C-x k` kills the
/// modified buffer, its question answered `y`. The kill retires the
/// copy's key without waiting for it; once the install is taken the key
/// is removed, after the copy's rename, so the copy does not outlive the
/// kill: the autosave directory ends empty. A kill and not a save,
/// because a save syncs its own file on the main thread, which `strace`
/// holds as long as the copy's, so the copy would land before any save
/// could retire it.
///
/// Fails at `1448046`, where no periodic copy is ever installing (the
/// precondition); fails with the key removed at the kill: the copy lands
/// after the removal and stays.
#[test]
fn a_kill_while_its_copy_installs_leaves_no_copy_behind() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(INTERVAL_MS, "");
    let mut pty = world.spawn_traced(Disk::Slow(Duration::from_secs(20)));
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"abc").expect("type abc");
    world.wait(
        &mut pty,
        "the sweep's copy installing",
        &["line=abcone", "pending=1"],
    );
    pty.write_input(b"\x18k").expect("type C-x k");
    world.wait_for(&mut pty, "the kill's prompt", |record| {
        record.lines().any(|l| l.starts_with("prompt=Kill buffer"))
    });
    pty.write_input(b"\r")
        .expect("take the buffer in the window");
    world.wait_for(&mut pty, "the kill's question", |record| {
        record
            .lines()
            .any(|l| l.starts_with("prompt=") && l.contains("kill anyway?"))
    });
    pty.write_input(b"y").expect("answer y");
    world.wait(
        &mut pty,
        "the buffer killed and the copy's install taken",
        &["modified=false", "pending=0", "prompt="],
    );
    assert_eq!(
        world.autosave_entries(),
        Vec::<String>::new(),
        "no recovery copy or temporary file outlives the kill"
    );
    pty.write_input(b"\x18\x03").expect("type C-x C-c");
    exits(&mut pty);

    let trace = world.trace();
    let lines = trace.lines();
    let autosave = world.autosave_dir();
    let key = format!("{}/", autosave.display());
    let renamed = lines
        .iter()
        .position(|l| l.contains("rename(") && l.contains(&format!(", \"{key}")))
        .unwrap_or_else(|| {
            panic!(
                "positive control: the copy is renamed over its key\n{}",
                trace.log
            )
        });
    let removed = lines
        .iter()
        .rposition(|l| {
            l.contains("unlink") && l.contains(&format!("\"{key}")) && !l.contains(".pmacs-tmp.")
        })
        .unwrap_or_else(|| panic!("the copy's key is removed\n{}", trace.log));
    assert!(
        renamed < removed,
        "the key is removed after the copy installing at the kill lands: renamed at log line \
         {renamed}, removed at {removed}\n{}",
        trace.log
    );
}
