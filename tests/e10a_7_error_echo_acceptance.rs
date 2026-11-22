// tests/e10a_7_error_echo_acceptance.rs --- E10a.7: an old error is not
// shown again.

//! The echo line fell back to the newest error whenever it had nothing
//! else to say, for as long as the error was unread (E5.1's rule): a
//! request a server refused, or a server that did not start, came back
//! after every unrelated key for the rest of the session, and in the
//! GPU's status band it stood where the buffer's name does. An error is
//! now a message like any other there: shown until the next key, and
//! not again. What persists is where it lived already: the mode line's
//! `LSP:!` and its unread mark, and the `*errors*` buffer.
//!
//! These rows make the baseline's second error, a language server that
//! does not start, and an error a command raises, then press unrelated
//! keys and read the echo line on a real TUI's screen and the GPU's
//! band through the headless probe. The echo's own rules, an error
//! outranked when it is reported among them, are unit rows beside
//! `build_status_line`.

use std::path::PathBuf;
use std::time::Duration;

mod common;
#[path = "support/mod.rs"]
mod support;

use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

const LIMIT: Duration = Duration::from_secs(30);

/// A rust server that cannot start, and a command that raises an error.
const INIT: &str = r#"
pmacs.lsp.config = {}
pmacs.lsp.config.rust = { command = "/nonexistent/e10a7-language-server" }
pmacs.command.define {
  name = "e10a7.refuse",
  description = "Raise an error, as a refused request does.",
  fn = function() pmacs.error("e10a7: the request was refused") end,
}
pmacs.keymap.bind { scope = "global", sequence = "C-c e", command = "e10a7.refuse" }
"#;

struct Fixture {
    dir: PathBuf,
    config: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10a7-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("config");
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    std::fs::write(config.join("pmacs/init.lua"), INIT).unwrap();
    std::fs::write(
        dir.join("a.rs"),
        "fn one() {}\nfn two() {}\nfn three() {}\n",
    )
    .unwrap();
    std::fs::write(dir.join("notes.txt"), "one\ntwo\nthree\n").unwrap();
    Fixture { dir, config }
}

fn tui_on(fx: &Fixture, name: &str) -> ScreenTui {
    let file = fx.dir.join(name).display().to_string();
    let (pty, tap) = spawn_pmacs_in_pty_tapped(
        &[&file],
        &[
            ("HOME", fx.dir.as_path()),
            ("XDG_CONFIG_HOME", fx.config.as_path()),
        ],
        24,
        100,
    );
    ScreenTui::new(pty, tap, 24, 100)
}

/// The mode line: the row above the echo line.
fn mode_line(tui: &ScreenTui) -> String {
    let rows = tui.rows();
    rows[rows.len() - 2].clone()
}

// ---------------------------------------------------------------------------
// The TUI's echo line
// ---------------------------------------------------------------------------

/// A rust file whose server does not start: the echo line says so, and
/// the mode line carries `LSP:!` and the unread mark. After unrelated
/// keys the echo line is clear and stays clear, and the mode line still
/// carries both. At the base the error stood in the echo line again
/// after every key.
#[test]
fn e10a_7_a_server_that_did_not_start_is_said_once_and_stays_on_the_mode_line() {
    let fx = fixture("server");
    let mut tui = tui_on(&fx, "a.rs");
    tui.wait_for(LIMIT, "the failure on the echo line", &|t| {
        t.bottom_row().contains("did not start")
    });
    assert!(
        mode_line(&tui).contains("LSP:!") && mode_line(&tui).contains("!1"),
        "premise: the mode line marks the server and the unread error; {}",
        tui.describe()
    );
    for (key, place) in [(&b"\x0e"[..], "L2:C1 "), (&b"\x06"[..], "L2:C2 ")] {
        tui.write(key);
        tui.wait_for(
            Duration::from_secs(10),
            "the key taken and the echo line without the error",
            &|t| t.screen_contains(place) && !t.bottom_row().contains("did not start"),
        );
        assert_eq!(
            tui.bottom_row(),
            "",
            "the echo line is clear; {}",
            tui.describe()
        );
    }
    assert!(
        mode_line(&tui).contains("LSP:!") && mode_line(&tui).contains("!1"),
        "the standing state stays where it lives, on the mode line; {}",
        tui.describe()
    );
}

/// An error a command raises is on the echo line until the next key and
/// not after it; raised again, it is a new message and is shown again.
/// At the base it came back after the first unrelated key.
#[test]
fn e10a_7_an_error_a_command_raised_is_gone_at_the_next_key_and_a_new_one_shows() {
    let fx = fixture("command");
    let mut tui = tui_on(&fx, "notes.txt");
    tui.wait_for(LIMIT, "the document", &|t| t.screen_contains("three"));
    tui.write(b"\x03e");
    tui.wait_for(LIMIT, "the error on the echo line", &|t| {
        t.bottom_row().contains("e10a7: the request was refused")
    });
    tui.write(b"\x0e");
    tui.wait_for(
        Duration::from_secs(10),
        "the key taken and the echo line without the error",
        &|t| t.screen_contains("L2:C1 ") && !t.bottom_row().contains("e10a7"),
    );
    tui.write(b"\x0e");
    tui.wait_for(
        Duration::from_secs(10),
        "a second key, and still no error",
        &|t| t.screen_contains("L3:C1 ") && !t.bottom_row().contains("e10a7"),
    );
    assert!(
        mode_line(&tui).contains("!1"),
        "the unread mark stays on the mode line; {}",
        tui.describe()
    );
    tui.write(b"\x03e");
    tui.wait_for(LIMIT, "the error raised again, shown again", &|t| {
        t.bottom_row().contains("e10a7: the request was refused")
    });
}

// ---------------------------------------------------------------------------
// The GPU's status band
// ---------------------------------------------------------------------------

/// The GPU's band, through the headless probe's production `App`: the
/// failure is in the band when the window opens, and after one
/// unrelated key and after two the band carries no message. At the base
/// it carried the error again after each.
#[cfg(feature = "crdt")]
#[test]
fn e10a_7_the_gpu_band_drops_the_error_at_the_next_key() {
    use common::daemon::TestDaemon;

    let fx = fixture("gpu");
    let init = format!(
        "{INIT}pmacs.buffer.find_or_open({:?})\n",
        fx.dir.join("a.rs").display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let steps = "shows:a.rs band:LSP: place:said key:C-n quiet:700 place:one \
                 key:C-f quiet:700 place:two";
    let Some(facts) = common::gpu_probe::run_script_probe(daemon.socket_path(), "e10a7", steps)
    else {
        support::record_skip("the GPU probe did not run");
        return;
    };
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert!(
        fact("said.status").contains("did not start"),
        "premise: the band says the server did not start; facts {facts:?}"
    );
    assert_eq!(
        (fact("one.line"), fact("two.col")),
        ("2".to_owned(), "2".to_owned()),
        "premise: the two keys moved the caret; facts {facts:?}"
    );
    assert_eq!(
        (fact("one.status"), fact("two.status")),
        ("\"\"".to_owned(), "\"\"".to_owned()),
        "the band carries no message after either key; facts {facts:?}"
    );
    assert_eq!(
        fact("two.name"),
        "a.rs",
        "and names the buffer; facts {facts:?}"
    );
}
