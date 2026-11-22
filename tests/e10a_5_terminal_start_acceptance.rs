// tests/e10a_5_terminal_start_acceptance.rs --- E10a.5: the terminal
// starts where the work is, and echoes.

//! `C-c t` opened a terminal in `$HOME` whatever buffer it was opened
//! from, and opened its PTY in the REPL's raw mode, so a shell that
//! keeps the modes it is given showed nothing typed, wrote each line a
//! row down from where the last ended, and took no signal from `C-c`.
//! These rows open a terminal as a user does, on a real TUI in a PTY,
//! under `/bin/sh` by a profile, and read the screen: a typed line and
//! its two lines of output, an interrupt, and where the shell started.

use std::path::{Path, PathBuf};
use std::time::Duration;

mod common;

use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

const LIMIT: Duration = Duration::from_secs(30);

/// A home, a project (`Cargo.toml` marks its root), a directory that
/// is no project, and an `init.lua` that makes `/bin/sh` the terminal
/// and keeps project detection inside this fixture.
struct Fixture {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("pmacs-e10a5-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home-e10a5");
    let config = root.join("config");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    std::fs::create_dir_all(root.join("proj-e10a5/src")).unwrap();
    std::fs::write(
        root.join("proj-e10a5/Cargo.toml"),
        "[package]\nname = \"p\"\n",
    )
    .unwrap();
    std::fs::write(root.join("proj-e10a5/src/lib.rs"), "// lib\n").unwrap();
    std::fs::create_dir_all(root.join("loose-e10a5")).unwrap();
    std::fs::write(root.join("loose-e10a5/notes.txt"), "notes\n").unwrap();
    std::fs::write(
        config.join("pmacs/init.lua"),
        format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.terminal.profiles.sh = {{ command = \"/bin/sh\" }}\n\
             pmacs.config.set(\"terminal.default-profile\", \"sh\")\n\
             pmacs.project.set_search_boundary({:?})\n",
            root.display().to_string()
        ),
    )
    .unwrap();
    Fixture { root, home, config }
}

/// A TUI on `file` (or on nothing), with `C-c t` pressed and the
/// terminal's own mode line on screen.
fn tui_with_terminal(fx: &Fixture, file: Option<&Path>) -> ScreenTui {
    let name = file.map(|f| f.display().to_string());
    let args: Vec<&str> = name.as_deref().into_iter().collect();
    let (pty, tap) = spawn_pmacs_in_pty_tapped(
        &args,
        &[
            ("HOME", fx.home.as_path()),
            ("XDG_CONFIG_HOME", fx.config.as_path()),
        ],
        24,
        80,
    );
    let mut tui = ScreenTui::new(pty, tap, 24, 80);
    let shown = file.map_or("*scratch*".to_owned(), |f| {
        f.file_name().unwrap().to_string_lossy().into_owned()
    });
    tui.wait_for(LIMIT, "the buffer's mode line", &|t| {
        t.screen_contains(&shown)
    });
    tui.write(b"\x03t");
    tui.wait_for(LIMIT, "the terminal's mode line", &|t| {
        t.screen_contains("*terminal:")
    });
    tui
}

/// Some row of the screen reads exactly `text`, from its first column.
fn row_reads(tui: &ScreenTui, text: &str) -> bool {
    tui.rows().iter().any(|row| row.trim_end() == text)
}

// ---------------------------------------------------------------------------
// The line discipline
// ---------------------------------------------------------------------------

/// `/bin/sh` sets no terminal modes of its own. A typed line is on
/// screen as it is typed, and its two lines of output are on two rows,
/// each from the first column. At the base the typed line was not shown
/// and `two` began where `one` ended, a row down.
#[test]
fn e10a_5_a_shell_that_sets_no_modes_echoes_and_starts_each_line_at_the_margin() {
    let fx = fixture("echo");
    let mut tui = tui_with_terminal(&fx, None);
    tui.write(b"echo one; echo two\r");
    tui.wait_for(LIMIT, "the typed line echoed", &|t| {
        t.screen_contains("echo one; echo two")
    });
    tui.wait_for(LIMIT, "`one` and `two` on two rows from the margin", &|t| {
        let rows = t.rows();
        rows.windows(2)
            .any(|pair| pair[0].trim_end() == "one" && pair[1].trim_end() == "two")
    });
}

/// `C-c C-c` sends the interrupt key to the child, and the terminal's
/// line discipline turns it into a signal: a `sleep` ends and the shell
/// runs the next command. At the base the key reached the shell as a
/// byte of input and the sleep ran on. Each marker is one the shell
/// computes, so the typed line's echo is never taken for it, and the
/// first is waited for so that the key arrives with the sleep running.
#[test]
fn e10a_5_the_interrupt_key_ends_the_foreground_command() {
    let fx = fixture("signal");
    let mut tui = tui_with_terminal(&fx, None);
    tui.write(b"echo started-$((40+2)); sleep 300\r");
    tui.wait_for(LIMIT, "the command running", &|t| {
        t.screen_contains("started-42")
    });
    // The escape key twice: the chord itself, to the child.
    tui.write(b"\x03\x03");
    tui.write(b"echo after-$((40+2))\r");
    tui.wait_for(
        Duration::from_secs(15),
        "the next command's output, the sleep having been interrupted",
        &|t| t.screen_contains("after-42"),
    );
}

// ---------------------------------------------------------------------------
// Where it starts
// ---------------------------------------------------------------------------

/// Opened from a file in a project, the shell starts in the project's
/// root, not in the file's own directory and not in `$HOME`.
#[test]
fn e10a_5_a_terminal_opened_from_a_project_file_starts_in_the_project_root() {
    let fx = fixture("project");
    let mut tui = tui_with_terminal(&fx, Some(&fx.root.join("proj-e10a5/src/lib.rs")));
    tui.write(b"basename \"$PWD\"\r");
    tui.wait_for(LIMIT, "the shell's directory", &|t| {
        ["proj-e10a5", "home-e10a5"]
            .iter()
            .any(|name| t.screen_contains(name))
    });
    assert!(
        row_reads(&tui, "proj-e10a5"),
        "the shell started in the project's root; {}",
        tui.describe()
    );
}

/// Opened from a file that is in no project, the shell starts in the
/// file's directory.
#[test]
fn e10a_5_a_terminal_opened_from_a_file_outside_any_project_starts_beside_it() {
    let fx = fixture("loose");
    let mut tui = tui_with_terminal(&fx, Some(&fx.root.join("loose-e10a5/notes.txt")));
    tui.write(b"basename \"$PWD\"\r");
    tui.wait_for(LIMIT, "the shell's directory", &|t| {
        ["loose-e10a5", "home-e10a5"]
            .iter()
            .any(|name| t.screen_contains(name))
    });
    assert!(
        row_reads(&tui, "loose-e10a5"),
        "the shell started beside the file; {}",
        tui.describe()
    );
}

/// Opened from a buffer with no file, the shell starts in `$HOME`, as it
/// did before.
#[test]
fn e10a_5_a_terminal_opened_from_a_buffer_with_no_file_starts_in_home() {
    let fx = fixture("home");
    let mut tui = tui_with_terminal(&fx, None);
    tui.write(b"basename \"$PWD\"\r");
    tui.wait_for(LIMIT, "the shell's directory", &|t| {
        row_reads(t, "home-e10a5")
    });
}
