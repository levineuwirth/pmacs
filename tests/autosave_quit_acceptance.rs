// tests/autosave_quit_acceptance.rs --- #365, quit asks before it waits on the recovery copies' sync.

//! `C-x C-c` with unsaved text asks its question before it waits on the
//! disk, and the recovery copy of that text is still on the disk, synced,
//! before the editor exits.
//!
//! **What #365 was, measured.** `editor.quit` runs the
//! `editor.before-quit` hooks before it asks, and autosave's writes a
//! recovery copy of every modified buffer whose text changed since its
//! last copy, each through `state::write_private`: the temporary file
//! synced, renamed over its key, the directory synced. Under a writer of
//! 2 GB files with `fdatasync` on this laptop's btrfs the question waited
//! on those syncs for up to 7.8 s, the screen unchanged. A recovery copy
//! is the only copy of the unsaved text it holds, so the syncs stay.
//! Quit's sweep now writes each copy's temporary file and stages it;
//! once the question is on the screen a thread syncs and installs them
//! by the calls the sweep made, in its order, and the exit waits for that
//! thread after the screen is restored, saying so.
//!
//! **What the rows assert.** What quit did, never how long it took: the
//! question reached the terminal before any sync of a recovery copy
//! (ordered in one `strace` log); the copy holds the typed text and was
//! synced before the editor exited (R1); a cancelled or vetoed quit
//! leaves the copy synced while the editor runs, as it did before (R2).
//! The exit wait, [`HANG_GUARD`], is there for a quit that never returns
//! and is not a budget.
//!
//! **Reading the screen.** The rows read what the editor showed from two
//! places: an `init.lua` hook's record of the minibuffer's prompt, and
//! the terminal's bytes for a word with no space in it. The TUI writes
//! only the cells that changed, so text with a space in it can reach the
//! terminal in pieces.

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

/// A word of the quit question, and of nothing else the rows show.
const ASKED: &str = "anyway?";

/// One row's world: a text file, an `init.lua`, a state root of its own.
struct World {
    dir: tempfile::TempDir,
}

impl World {
    /// `extra` is Lua appended to the row's `init.lua`.
    fn new(extra: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("notes.txt"), TEXT).expect("write notes.txt");
        let config = dir.path().join("config").join("pmacs");
        std::fs::create_dir_all(&config).expect("config dir");
        std::fs::create_dir_all(dir.path().join("state")).expect("state dir");
        let status = dir.path().join("status");
        let init = format!(
            r#"
pmacs.lsp.config = {{}}
-- No periodic sweep inside a row: the copy it reads is quit's.
pmacs.autosave.interval_ms(600000)
-- Each frame: whether the buffer is modified, how many of quit's copies
-- are not yet at their keys (none before #365 staged any), the prompt
-- the minibuffer shows, and whether the row's own veto ran; written
-- when they change.
local last
pmacs.hook.add("process.after-tick", function()
  local buf = pmacs.window.buffer()
  local d = buf and pmacs.describe.buffer(buf)
  local staged = pmacs.autosave._staged and #pmacs.autosave._staged() or 0
  local prompt = pmacs.minibuffer.is_active() and pmacs.minibuffer.prompt() or ""
  local text = "modified=" .. tostring(d and d.modified)
    .. "\nstaged=" .. tostring(staged)
    .. "\nvetoed=" .. tostring(_G.row_vetoed == true)
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

    /// Run the editor on `notes.txt` under `wrapper` (empty: none).
    fn spawn(&self, wrapper: &[&str]) -> PmacsPty {
        let file = self.path("notes.txt").display().to_string();
        let envs = [
            ("XDG_CONFIG_HOME", self.path("config")),
            ("XDG_STATE_HOME", self.path("state")),
            ("PMACS_STATE_HOME", self.path("state")),
        ];
        let envs: Vec<(&str, &Path)> = envs.iter().map(|(k, v)| (*k, v.as_path())).collect();
        spawn_pmacs_in_pty_under(wrapper, &[&file], &envs, 40, 200)
    }

    /// Run the editor under `strace`, which logs to `syncs.strace` every
    /// write, sync and rename the editor and its threads make, with the
    /// path of each descriptor.
    fn spawn_traced(&self) -> PmacsPty {
        let log = self.path("syncs.strace").display().to_string();
        self.spawn(&[
            "strace",
            "-f",
            "-qq",
            "-y",
            "-s",
            "4096",
            "-e",
            "trace=write,fsync,fdatasync,sync_file_range,syncfs,rename,renameat,renameat2",
            "-o",
            &log,
        ])
    }

    fn trace(&self) -> String {
        std::fs::read_to_string(self.path("syncs.strace")).expect("read the strace log")
    }

    /// The hook's last record, or empty before its first frame.
    fn record(&self) -> String {
        std::fs::read_to_string(self.path("status")).unwrap_or_default()
    }

    /// The prompt in the hook's last record, empty when none is up.
    fn prompt(&self) -> String {
        self.record()
            .lines()
            .find_map(|l| l.strip_prefix("prompt="))
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

    /// The recovery copy autosave holds for `notes.txt`: the header line
    /// naming it and the text after it. `None` when there is none.
    fn recovery_copy(&self) -> Option<(String, String)> {
        let dir = self.autosave_dir();
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".pmacs-tmp.") {
                continue;
            }
            let bytes = std::fs::read(entry.path()).expect("read a recovery copy");
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let (header, body) = text.split_once('\n').expect("a recovery copy has a header");
            if header.contains("notes.txt") {
                return Some((header.to_owned(), body.to_owned()));
            }
        }
        None
    }

    /// Require the copy to hold `typed` ahead of the file's text.
    fn require_copy(&self, typed: &str) {
        let (header, body) = self
            .recovery_copy()
            .expect("no recovery copy of notes.txt under the row's state root");
        assert_eq!(
            body,
            format!("{typed}{TEXT}"),
            "the recovery copy holds the text typed before the quit; header {header}"
        );
    }
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

/// Whether `strace` runs here and can trace a child. A row that cannot
/// use it records its skip, and `PMACS_REQUIRE_STRACE` makes that a
/// failure.
fn strace_or_skip() -> bool {
    let usable = match std::process::Command::new("strace")
        .args(["-f", "-qq", "-o", "/dev/null", "true"])
        .output()
    {
        Err(e) => Err(format!("`strace` not runnable: {e}")),
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "`strace` cannot trace a child here: {}",
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

/// The indexes in `log`'s lines of the editor's writes of the question
/// to its terminal, in log order.
fn question_writes(log: &str) -> Vec<usize> {
    log.lines()
        .enumerate()
        .filter(|(_, l)| l.contains("write(1<") && l.contains(ASKED))
        .map(|(i, _)| i)
        .collect()
}

/// The indexes of the sync calls on a recovery copy's temporary file and
/// on the autosave directory, in log order. Under `-f` a call another
/// thread interrupts is logged at its entry, `<unfinished ...>`.
fn copy_syncs(log: &str, autosave: &Path) -> (Vec<usize>, Vec<usize>) {
    let temp = format!("<{}/.pmacs-tmp.", autosave.display());
    let dir = format!("<{}>", autosave.display());
    let syncs = |needle: &str| -> Vec<usize> {
        log.lines()
            .enumerate()
            .filter(|(_, line)| {
                ["fsync(", "fdatasync("].iter().any(|c| line.contains(c)) && line.contains(needle)
            })
            .map(|(i, _)| i)
            .collect()
    };
    (syncs(&temp), syncs(&dir))
}

/// **Quit asks first, and the copy is on the disk before it exits** (the
/// fix, and R1). Text typed into `notes.txt`, then `C-x C-c`; once the
/// question is up and on the terminal, `y`. In the editor's `strace` log
/// the question's write comes before every sync of the recovery copy,
/// and the copy's temporary file and the autosave directory are each
/// synced before the editor exits. The copy holds the typed text.
///
/// Fails at `a1f0c27`, whose sweep synced before the question: "the
/// question waited on the recovery copy's sync". Fails with no sync of
/// the copy at all (the staged copy never finished): "never synced". The
/// question's write in the log is the instrument's positive control.
#[test]
fn quit_asks_before_it_syncs_and_the_copy_is_on_disk_before_it_exits() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new("");
    let mut pty = world.spawn_traced();
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"abc").expect("type abc");
    world.wait(&mut pty, "the edit", &["modified=true"]);
    assert!(
        world.recovery_copy().is_none(),
        "precondition: nothing wrote a recovery copy before the quit"
    );

    world.quit_and_see_the_question(&mut pty);
    pty.write_input(b"y").expect("answer y");
    exits(&mut pty);

    let log = world.trace();
    let question = *question_writes(&log)
        .first()
        .unwrap_or_else(|| panic!("positive control: the log holds the question's write\n{log}"));
    let (temp, dir) = copy_syncs(&log, &world.autosave_dir());
    assert!(
        !temp.is_empty() && !dir.is_empty(),
        "the recovery copy's temporary file and its directory are synced before the editor \
         exits; it was never synced (file {temp:?}, directory {dir:?}):\n{log}"
    );
    let first = temp[0].min(dir[0]);
    assert!(
        question < first,
        "the question waited on the recovery copy's sync: question at log line {question}, \
         first sync of the copy at {first}:\n{log}"
    );
    world.require_copy("abc");
}

/// **A quit answered at once writes the copy after the screen is
/// restored, and says so** (R1 on the exit path). `C-x C-c y` arrives in
/// one write, so the editor exits before any tick could start the copy's
/// install: it is synced as the editor drops, after the TUI has left the
/// alternate screen, and stderr names the wait first. The copy holds the
/// typed text.
///
/// Fails with the install at the editor's drop removed: no line, no sync
/// of the copy, and no copy at its key.
#[test]
fn a_quit_answered_at_once_writes_the_copy_after_the_screen_and_says_so() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new("");
    let mut pty = world.spawn_traced();
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"abc").expect("type abc");
    world.wait(&mut pty, "the edit", &["modified=true"]);

    pty.write_input(b"\x18\x03y")
        .expect("type C-x C-c y in one write");
    exits(&mut pty);

    let log = world.trace();
    let said = "pmacs: quitting: writing the recovery copy of ";
    let told = log
        .lines()
        .position(|l| l.contains("write(2<") && l.contains(said) && l.contains("notes.txt to disk"))
        .unwrap_or_else(|| {
            panic!("the exit says what it waits on, `{said}…notes.txt to disk`:\n{log}")
        });
    let (temp, dir) = copy_syncs(&log, &world.autosave_dir());
    assert!(
        !temp.is_empty() && !dir.is_empty(),
        "the copy is synced before the editor exits (file {temp:?}, directory {dir:?}):\n{log}"
    );
    assert!(
        told < temp[0],
        "the line saying what the exit waits on comes before the wait:\n{log}"
    );
    world.require_copy("abc");
}

/// **A cancelled quit leaves the copy synced while the editor runs**
/// (R2). `C-x C-c`, then `n`: the editor keeps running and installs the
/// copy on its own, the hook's record reading `staged=0` with no prompt
/// up. Then a second `C-x C-c` asks again and `y` ends it. The copy's
/// syncs come before the second question in the log, so they did not
/// wait for the exit; the copy holds the text typed before the first
/// quit, nothing sweeping on a clock in between.
///
/// Fails with the sweep moved after the question's `y`, or with the
/// install left to the editor's drop: either way the copy's first sync
/// comes after the second question, or never before it.
#[test]
fn a_cancelled_quit_leaves_the_copy_synced_while_the_editor_runs() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new("");
    let mut pty = world.spawn_traced();
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"abc").expect("type abc");
    world.wait(&mut pty, "the edit", &["modified=true"]);

    world.quit_and_see_the_question(&mut pty);
    pty.write_input(b"n").expect("answer n");
    world.wait(
        &mut pty,
        "the quit cancelled and the copy installed",
        &["modified=true", "staged=0", "prompt="],
    );
    world.require_copy("abc");

    world.quit_and_see_the_question(&mut pty);
    pty.write_input(b"y").expect("answer y");
    exits(&mut pty);

    let log = world.trace();
    let questions = question_writes(&log);
    assert!(
        questions.len() >= 2,
        "positive control: the log holds both questions' writes ({questions:?}):\n{log}"
    );
    let (temp, dir) = copy_syncs(&log, &world.autosave_dir());
    assert!(
        temp.first()
            .zip(dir.first())
            .is_some_and(|(t, d)| *t.max(d) < questions[1]),
        "a cancelled quit's copy is synced while the editor runs, before the next quit asks: \
         file syncs {temp:?}, directory syncs {dir:?}, questions {questions:?}:\n{log}"
    );
    world.require_copy("abc");
}

/// **A vetoed quit leaves the copy synced while the editor runs** (R2).
/// `init.lua` adds a hook that vetoes the first quit and says so in the
/// record. `C-x C-c` is vetoed, and the copy autosave's hook wrote before
/// the veto is installed while the editor runs; the second `C-x C-c` asks
/// and `y` ends it. The copy's syncs come before that question.
///
/// Fails with the install left to the editor's drop.
#[test]
fn a_vetoed_quit_leaves_the_copy_synced_while_the_editor_runs() {
    if !strace_or_skip() {
        return;
    }
    let world = World::new(
        "pmacs.hook.add('editor.before-quit', function()\n\
           if not _G.row_vetoed then _G.row_vetoed = true; return false end\n\
         end)",
    );
    let mut pty = world.spawn_traced();
    world.wait(&mut pty, "the editor's first frame", &["modified=false"]);
    pty.write_input(b"abc").expect("type abc");
    world.wait(&mut pty, "the edit", &["modified=true"]);

    pty.write_input(b"\x18\x03").expect("type C-x C-c");
    world.wait(
        &mut pty,
        "the veto and the copy installed",
        &["modified=true", "staged=0", "vetoed=true", "prompt="],
    );
    world.require_copy("abc");

    world.quit_and_see_the_question(&mut pty);
    pty.write_input(b"y").expect("answer y");
    exits(&mut pty);

    let log = world.trace();
    let question = *question_writes(&log)
        .first()
        .unwrap_or_else(|| panic!("positive control: the log holds the question's write\n{log}"));
    let (temp, dir) = copy_syncs(&log, &world.autosave_dir());
    assert!(
        temp.first()
            .zip(dir.first())
            .is_some_and(|(t, d)| *t.max(d) < question),
        "a vetoed quit's copy is synced while the editor runs, before the next quit asks: file \
         syncs {temp:?}, directory syncs {dir:?}, question at {question}:\n{log}"
    );
    world.require_copy("abc");
}
