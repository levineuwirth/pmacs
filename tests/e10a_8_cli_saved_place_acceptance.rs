// tests/e10a_8_cli_saved_place_acceptance.rs --- E10a.8: `pmacs FILE`
// opens at its saved place.

//! A file named on the command line opened at its first line though a
//! place was saved for it, while `C-x C-f` on the same file opened at
//! the place. The command line's open happens while the editor is being
//! constructed, before the state directory is installed, so the hook
//! that restores a place found no state to read. No ruling made the top
//! deliberate: Q#DS7 keeps the *desktop* from restoring over a file
//! argument, and says nothing of the file's own place.
//!
//! These rows are two real sessions each, on a TUI in a PTY, sharing a
//! state directory of their own: the first leaves the file at a place
//! and quits, and the second names the file on the command line.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

mod common;

use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

const LIMIT: Duration = Duration::from_secs(30);

struct Fixture {
    dir: PathBuf,
    config: PathBuf,
    state: PathBuf,
    file: PathBuf,
}

fn fixture(tag: &str, init: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10a8-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("config");
    let state = dir.join("state");
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        config.join("pmacs/init.lua"),
        format!("pmacs.lsp.config = {{}}\n{init}"),
    )
    .unwrap();
    let mut text = String::new();
    for i in 1..=60 {
        let _ = writeln!(text, "line {i:02} of the file");
    }
    let file = dir.join("places.txt");
    std::fs::write(&file, text).unwrap();
    Fixture {
        dir,
        config,
        state,
        file,
    }
}

impl Fixture {
    /// A TUI started as `pmacs <args>`, on this fixture's roots.
    fn session(&self, args: &[&str]) -> ScreenTui {
        let (pty, tap) = spawn_pmacs_in_pty_tapped(
            args,
            &[
                ("HOME", self.dir.as_path()),
                ("XDG_CONFIG_HOME", self.config.as_path()),
                ("PMACS_STATE_HOME", self.state.as_path()),
            ],
            24,
            80,
        );
        ScreenTui::new(pty, tap, 24, 80)
    }

    fn file_arg(&self) -> String {
        self.file.display().to_string()
    }

    /// The first session: the file opened by name, the caret taken to
    /// line 29, column 5, and the editor quit, which records the place.
    fn leave_a_place(&self) {
        let mut tui = self.session(&[&self.file_arg()]);
        tui.wait_for(LIMIT, "the file at its top", &|t| {
            t.screen_contains("L1:C1 ")
        });
        tui.write(b"\x1bx");
        tui.wait_for(LIMIT, "the M-x prompt", &|t| t.screen_contains("M-x"));
        tui.write(b"goto-line\r29\r");
        tui.wait_for(LIMIT, "line 29", &|t| t.screen_contains("L29:C1 "));
        tui.write(b"\x06\x06\x06\x06");
        tui.wait_for(LIMIT, "column 5", &|t| t.screen_contains("L29:C5 "));
        tui.write(b"\x18\x03");
        let deadline = std::time::Instant::now() + LIMIT;
        while !self.state.join("places").exists()
            && !std::fs::read_dir(&self.state).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.path().join("places").exists())
            })
        {
            assert!(
                std::time::Instant::now() < deadline,
                "premise: quitting recorded a place under {}",
                self.state.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A place saved by one session; the next names the file on the command
/// line and opens at the place. At the base it opened at `L1:C1`.
#[test]
fn e10a_8_a_file_named_on_the_command_line_opens_at_its_saved_place() {
    let fx = fixture("cli", "");
    fx.leave_a_place();
    let mut tui = fx.session(&[&fx.file_arg()]);
    tui.wait_for(LIMIT, "the file on screen", &|t| {
        t.screen_contains("of the file")
    });
    tui.wait_for(
        Duration::from_secs(10),
        "the caret at the saved place, line 29 column 5",
        &|t| t.screen_contains("L29:C5 "),
    );
    assert!(
        tui.screen_contains("line 29 of the file"),
        "the place is in view; {}",
        tui.describe()
    );
}

/// The same place reached by `C-x C-f` in a session started on nothing:
/// the route that worked at the base, held beside the other.
#[test]
fn e10a_8_find_file_still_opens_at_the_saved_place() {
    let fx = fixture("find", "");
    fx.leave_a_place();
    let mut tui = fx.session(&[]);
    tui.wait_for(LIMIT, "the empty start", &|t| {
        t.screen_contains("*scratch*")
    });
    tui.write(b"\x18\x06");
    tui.wait_for(LIMIT, "the find-file prompt", &|t| {
        t.screen_contains("Find file")
    });
    // The prompt holds a directory already, and `C-k` does not clear a
    // prompt (the baseline's walk): one backspace a character, then the
    // file's whole path.
    tui.write(&[0x7f; 400]);
    tui.write(fx.file_arg().as_bytes());
    tui.write(b"\r");
    tui.wait_for(
        Duration::from_secs(10),
        "the caret at the saved place, line 29 column 5",
        &|t| t.screen_contains("L29:C5 "),
    );
}

/// The same open misses the recent-files list for the same reason: a
/// file named on the command line is in the `recentf` state file after
/// its session. At the base it was not, though `C-x C-f` put it there.
#[test]
fn e10a_8_a_file_named_on_the_command_line_enters_the_recent_files() {
    let fx = fixture("recentf", "");
    let mut tui = fx.session(&[&fx.file_arg()]);
    tui.wait_for(LIMIT, "the file on screen", &|t| {
        t.screen_contains("of the file")
    });
    let wanted = fx.file_arg();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let holds = |dir: &std::path::Path| {
        std::fs::read_to_string(dir.join("recentf"))
            .is_ok_and(|text| text.lines().any(|line| line == wanted))
    };
    loop {
        let found = holds(&fx.state)
            || std::fs::read_dir(&fx.state)
                .is_ok_and(|entries| entries.flatten().any(|entry| holds(&entry.path())));
        if found {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the recent-files list under {} names {wanted}",
            fx.state.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `pmacs.saveplace.enable(false)` in `init.lua` still means the top,
/// on the command line as elsewhere.
#[test]
fn e10a_8_with_saveplace_disabled_the_command_line_opens_at_the_top() {
    let fx = fixture("off", "");
    fx.leave_a_place();
    std::fs::write(
        fx.config.join("pmacs/init.lua"),
        "pmacs.lsp.config = {}\npmacs.saveplace.enable(false)\n",
    )
    .unwrap();
    let mut tui = fx.session(&[&fx.file_arg()]);
    // The place is restored before the first frame, so the first frame
    // that shows the file shows where it opened.
    tui.wait_for(LIMIT, "the file on screen", &|t| {
        t.screen_contains("of the file")
    });
    assert!(
        tui.screen_contains("L1:C1 ") && tui.screen_contains("line 01 of the file"),
        "the file opened at its top; {}",
        tui.describe()
    );
}
