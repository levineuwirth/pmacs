// tests/quit_state_write_acceptance.rs --- #360, quit writes its state and waits for no sync.

//! `C-x C-c` writes the place it left to the `places` state file and
//! returns without waiting for the disk to take it.
//!
//! **What #360 was, measured.** `editor.quit` runs the
//! `editor.before-quit` hooks on the main thread before it exits, and
//! saveplace's writes `places` through the state store. Until #360 that
//! write synced twice, the temporary file before its rename and the
//! directory after it, a durability the store inherited from the
//! editor's own saves (F-006) when it chose `save_atomic` for its
//! atomicity. Under a gate sweep the release build took 5.9 to 20.6 s to
//! leave after `C-x C-c`, its screen unchanged, the main thread in the
//! filesystem's sync path; under a writer of 2 GB files with
//! `fdatasync`, `strace` put each run's wait in those two calls (E8c
//! review 1). The state store now syncs nothing (`state::write`): what
//! it holds is cheap to lose, and a sync's wait has no bound.
//!
//! **What the rows assert.** What quit did, never how long it took: the
//! place is in `places` and the editor exited cleanly; and, under
//! `strace`, no sync named the state root while the user's own save in
//! the same session still synced, which is the instrument's positive
//! control. How long quit took is printed as a measurement. The exit
//! wait, [`HANG_GUARD`], is there for a quit that never returns (#299's
//! class) and is not a budget.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;
mod support;

use common::pty::{PmacsPty, spawn_pmacs_in_pty, spawn_pmacs_in_pty_under};

/// How long a row waits for `C-x C-c` to end the editor before calling
/// it hung. A guard, not a budget: three times the longest wait E8c
/// review 1 measured with the syncs in (20.6 s), and no row reads the
/// time against it.
const HANG_GUARD: Duration = Duration::from_mins(1);

/// From spawn to each state a row waits for.
const READY_BOUND: Duration = Duration::from_mins(1);

/// The file each row visits; `C-n` three times from the start reaches
/// byte 14, the start of `four`.
const TEXT: &str = "one\ntwo\nthree\nfour\nfive\n";

/// One row's world: a text file, an `init.lua`, a state root of its own,
/// and the file the hook writes.
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
-- Each frame, the cursor's byte and whether the buffer is modified,
-- written when they change.
local last
pmacs.hook.add("process.after-tick", function()
  local buf = pmacs.window.buffer()
  local d = buf and pmacs.describe.buffer(buf)
  local text = "cursor=" .. tostring(pmacs.editor.cursor())
    .. "\nmodified=" .. tostring(d and d.modified) .. "\n"
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

    fn envs(&self) -> [(&'static str, PathBuf); 3] {
        [
            ("XDG_CONFIG_HOME", self.path("config")),
            ("XDG_STATE_HOME", self.path("state")),
            ("PMACS_STATE_HOME", self.path("state")),
        ]
    }

    fn spawn(&self, wrapper: &[&str]) -> PmacsPty {
        let file = self.path("notes.txt").display().to_string();
        let envs = self.envs();
        let envs: Vec<(&str, &Path)> = envs.iter().map(|(k, v)| (*k, v.as_path())).collect();
        if wrapper.is_empty() {
            spawn_pmacs_in_pty(&[&file], &envs, 40, 120)
        } else {
            spawn_pmacs_in_pty_under(wrapper, &[&file], &envs, 40, 120)
        }
    }

    /// The hook's last record, or empty before its first frame.
    fn status(&self) -> String {
        std::fs::read_to_string(self.path("status")).unwrap_or_default()
    }

    /// Poll until the hook's record holds every line of `want`.
    fn wait(&self, pty: &mut PmacsPty, what: &str, want: &[&str]) {
        let deadline = Instant::now() + READY_BOUND;
        loop {
            let status = self.status();
            if want.iter().all(|w| status.lines().any(|l| l == *w)) {
                return;
            }
            assert!(
                pty.wait_for_exit(Duration::ZERO).is_none(),
                "the editor exited while waiting for {what}; last record:\n{status}"
            );
            assert!(
                Instant::now() < deadline,
                "precondition: {what} within {READY_BOUND:?}; last record:\n{status}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The `places` file under the row's state root, wherever the store
    /// put it.
    fn places(&self) -> Option<PathBuf> {
        let mut dirs = vec![self.path("state")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.file_name().is_some_and(|n| n == "places") {
                    return Some(path);
                }
            }
        }
        None
    }

    /// Require `places` to lead with `cursor 0 <notes.txt>`, the line
    /// saveplace writes for the file at that byte and view top 0.
    fn require_place(&self, cursor: usize) {
        let places = self
            .places()
            .expect("quit wrote no places file under the row's state root");
        let text = std::fs::read_to_string(&places).expect("read places");
        let first = text.lines().next().unwrap_or_default();
        let given = self.path("notes.txt");
        let canonical = std::fs::canonicalize(&given).expect("canonical notes.txt");
        assert!(
            [&given, &canonical]
                .iter()
                .any(|p| first == format!("{cursor} 0 {}", p.display())),
            "places should lead with the place quit left, `{cursor} 0 {}`; it holds:\n{text}",
            given.display()
        );
    }
}

/// Press `C-x C-c` and require a clean exit inside [`HANG_GUARD`].
/// Prints how long it took, as a measurement.
fn quit(pty: &mut PmacsPty) {
    pty.write_input(b"\x18\x03").expect("type C-x C-c");
    let started = Instant::now();
    let Some(status) = pty.wait_for_exit(HANG_GUARD) else {
        panic!("C-x C-c did not end the editor within {HANG_GUARD:?}, a hang guard");
    };
    let took = started.elapsed();
    assert!(
        status.success(),
        "quit is a clean exit, got {status:?} after {took:?}"
    );
    eprintln!("quit took {took:?} (a measurement, not asserted)");
}

/// **What quit does with the place.** The default configuration but for
/// its roots: saveplace on, a file visited and the cursor moved, then
/// `C-x C-c`. The editor exits cleanly and `places` holds the place it
/// left.
///
/// Fails if quit no longer writes the place (saveplace's
/// `editor.before-quit` hook gone or its write refused): no `places`
/// file, or one leading with another place. Nothing before the keys
/// writes `places`, which the row checks first.
#[test]
fn quit_writes_the_place_it_left_and_returns() {
    let world = World::new("");
    let mut pty = world.spawn(&[]);
    world.wait(&mut pty, "the editor's first frame", &["cursor=0"]);
    assert!(
        world.places().is_none(),
        "precondition: nothing wrote places before quit"
    );

    pty.write_input(b"\x0e\x0e\x0e")
        .expect("type C-n three times");
    world.wait(&mut pty, "the cursor on `four`", &["cursor=14"]);
    assert!(
        world.places().is_none(),
        "precondition: moving the cursor wrote no places"
    );

    quit(&mut pty);
    world.require_place(14);
}

/// Whether `strace` runs here and can trace a child: present on `PATH`,
/// and `ptrace` not refused (a container's seccomp, a Yama setting).
fn strace_usable() -> Result<(), String> {
    match std::process::Command::new("strace")
        .args(["-f", "-qq", "-o", "/dev/null", "true"])
        .output()
    {
        Err(e) => Err(format!("`strace` not runnable: {e}")),
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "`strace` cannot trace a child here: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// Every path `strace -y` printed as the descriptor of a sync call.
fn synced_paths(log: &str) -> Vec<String> {
    log.lines()
        .filter(|line| {
            ["fsync(", "fdatasync(", "sync_file_range(", "syncfs("]
                .iter()
                .any(|call| line.contains(call))
        })
        .filter_map(|line| {
            let start = line.find('<')? + 1;
            let end = start + line[start..].find(">)")?;
            Some(line[start..end].to_owned())
        })
        .collect()
}

/// **What the state write waits for.** The editor runs under `strace`,
/// which records every sync call it and its children make with the path
/// of the descriptor. In one session the user types into the file and
/// saves it, moves, and quits. saveplace writes `places` twice, at the
/// save and at quit; neither write syncs anything under the state root,
/// and the save of the user's file, which syncs by design (F-006), shows
/// the instrument sees a sync when there is one. `places` holds the
/// place quit left.
///
/// Fails with `state::write` syncing again, on the path of `places`'s
/// temporary file and its directory. Autosave is off, so its recovery
/// copy, which does sync and lives under the same root, cannot reach the
/// assertion. Linux with `strace`; elsewhere it records its skip, and
/// `PMACS_REQUIRE_STRACE` makes a missing or refused `strace` a failure.
#[test]
fn the_state_written_at_quit_waits_for_no_sync_and_a_save_still_syncs() {
    if let Err(why) = strace_usable() {
        assert!(
            std::env::var_os("PMACS_REQUIRE_STRACE").is_none_or(|v| v.is_empty()),
            "PMACS_REQUIRE_STRACE is set, but {why}"
        );
        support::record_skip(&format!("{why}, and PMACS_REQUIRE_STRACE unset"));
        eprintln!("{why}; skipping (set PMACS_REQUIRE_STRACE to make this fatal)");
        return;
    }
    let world = World::new(
        "-- Autosave's recovery copy syncs by design; this row is about\n\
         -- the rest of the state root.\n\
         pmacs.autosave.enable(false)",
    );
    let log = world.path("syncs.strace");
    let log_arg = log.display().to_string();
    let mut pty = world.spawn(&[
        "strace",
        "-f",
        "-qq",
        "-y",
        "-e",
        "trace=fsync,fdatasync,sync_file_range,syncfs,sync",
        "-o",
        &log_arg,
    ]);
    world.wait(
        &mut pty,
        "the editor's first frame",
        &["cursor=0", "modified=false"],
    );

    pty.write_input(b"x").expect("type x");
    world.wait(&mut pty, "the edit", &["cursor=1", "modified=true"]);
    pty.write_input(b"\x18\x13").expect("type C-x C-s");
    world.wait(&mut pty, "the save", &["modified=false"]);
    assert!(
        std::fs::read_to_string(world.path("notes.txt"))
            .expect("read notes.txt")
            .starts_with("xone\n"),
        "precondition: C-x C-s wrote the edit to notes.txt"
    );
    // From byte 1 of `xone`, three lines down: `four`'s byte 1.
    pty.write_input(b"\x0e\x0e\x0e")
        .expect("type C-n three times");
    world.wait(&mut pty, "the cursor in `four`", &["cursor=16"]);

    quit(&mut pty);

    let traced = std::fs::read_to_string(&log).expect("read the strace log");
    let synced = synced_paths(&traced);
    let dir = std::fs::canonicalize(world.dir.path()).expect("canonical world dir");
    let state = std::fs::canonicalize(world.path("state")).expect("canonical state root");
    let saved = format!("{}/.pmacs-tmp.notes.txt.", dir.display());
    assert!(
        synced.iter().any(|p| p.starts_with(&saved)),
        "positive control: the save of notes.txt syncs its temporary file, so the instrument \
         sees a sync; it saw these:\n{synced:#?}\nlog:\n{traced}"
    );
    let state_syncs: Vec<&String> = synced
        .iter()
        .filter(|p| Path::new(p).starts_with(&state))
        .collect();
    assert!(
        state_syncs.is_empty(),
        "the state store synced, so quit and every save wait on the disk (#360): {state_syncs:#?}"
    );
    world.require_place(16);
}
