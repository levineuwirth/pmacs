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
// An error raised while no key is pressed (review 1's probes)
// ---------------------------------------------------------------------------

/// A command that raises an error a second and a half after it runs,
/// with no key pressed in between: a server that dies while the user
/// reads, a job that fails in the background.
const LATER_INIT: &str = r#"
pmacs.lsp.config = {}
pmacs.command.define {
  name = "r1.later",
  description = "Raise an error a second and a half from now.",
  fn = function()
    pmacs.async(function()
      pmacs.workers.sleep(1500):await()
      pmacs.error("r1: raised with no key pressed")
    end)
  end,
}
pmacs.keymap.bind { scope = "global", sequence = "C-c z", command = "r1.later" }
"#;

fn later_tui(tag: &str) -> ScreenTui {
    let dir = std::env::temp_dir().join(format!("pmacs-r1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("config");
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    std::fs::write(config.join("pmacs/init.lua"), LATER_INIT).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    let file = file.display().to_string();
    let (pty, tap) = spawn_pmacs_in_pty_tapped(
        &[&file],
        &[
            ("HOME", dir.as_path()),
            ("XDG_CONFIG_HOME", config.as_path()),
        ],
        24,
        100,
    );
    let mut tui = ScreenTui::new(pty, tap, 24, 100);
    tui.wait_for(LIMIT, "the document", &|t| t.screen_contains("three"));
    tui
}

/// An error raised while no key is pressed reaches the echo line without
/// a key: the control for the rows under a prompt. Review 1's `r1_p1`,
/// as it ran; it passed at the reviewed head.
#[test]
fn r1_p1_an_error_raised_with_no_key_pressed_is_shown() {
    let mut tui = later_tui("idle");
    tui.write(b"\x03z");
    tui.wait_for(
        Duration::from_secs(10),
        "the error on the echo line, no key pressed after the command",
        &|t| t.bottom_row().contains("r1: raised with no key pressed"),
    );
}

/// An error raised while the `M-x` prompt holds the echo line is shown
/// once the prompt is gone. At the head it never is: the painter asks
/// for the error before it paints the prompt over the same row, the
/// asking marks it shown, and `C-g` spends it. At `e10d427` it stood on
/// the echo line after the key that cleared `C-g`'s word. Review 1's
/// `r1_p2`, as it ran; it failed at the reviewed head, `8f5fe3a`.
#[test]
fn r1_p2_an_error_raised_under_a_prompt_is_shown_once_the_prompt_is_gone() {
    let mut tui = later_tui("prompt");
    tui.write(b"\x03z");
    tui.write(b"\x1bx");
    tui.wait_for(LIMIT, "premise: the M-x prompt is open", &|t| {
        t.bottom_row().contains("M-x")
    });
    // The error lands while the prompt is up; nothing is typed.
    let _ = tui.watch_for(Duration::from_secs(3), &|_| false);
    tui.write(b"\x07");
    tui.wait_for(
        LIMIT,
        "premise: the prompt is gone and the error was raised (the unread mark)",
        &|t| !t.bottom_row().contains("M-x") && mode_line(t).contains("!1"),
    );
    // One unrelated key, which clears whatever `C-g` said.
    tui.write(b"\x0e");
    let shown = tui.watch_for(Duration::from_secs(5), &|t| {
        t.bottom_row().contains("r1: raised with no key pressed")
    });
    assert!(
        shown.is_some(),
        "the error raised under the prompt never reached the echo line; {}",
        tui.describe()
    );
}

/// The same under isearch, whose prompt holds the echo row as `M-x`'s
/// does: `C-s`, the error raised under it, `C-g`, one key, and the
/// error is on the echo line, once. Fails at `8f5fe3a` as `r1_p2` does.
#[test]
fn e10a_7_an_error_raised_under_isearch_is_shown_once_the_search_is_gone() {
    let mut tui = later_tui("isearch");
    tui.write(b"\x03z");
    tui.write(b"\x13");
    tui.wait_for(LIMIT, "premise: isearch holds the echo line", &|t| {
        t.bottom_row().contains("I-search")
    });
    let _ = tui.watch_for(Duration::from_secs(3), &|_| false);
    assert!(
        tui.bottom_row().contains("I-search") && mode_line(&tui).contains("!1"),
        "premise: the error was raised while the search was open; {}",
        tui.describe()
    );
    tui.write(b"\x07");
    tui.wait_for(LIMIT, "premise: the search is gone", &|t| {
        !t.bottom_row().contains("I-search")
    });
    tui.write(b"\x0e");
    let shown = tui.watch_for(Duration::from_secs(5), &|t| {
        t.bottom_row().contains("r1: raised with no key pressed")
    });
    assert!(
        shown.is_some(),
        "the error raised under isearch never reached the echo line; {}",
        tui.describe()
    );
    tui.write(b"\x10");
    tui.wait_for(
        Duration::from_secs(10),
        "the next key, and the error is gone",
        &|t| t.screen_contains("L1:C1 ") && !t.bottom_row().contains("r1: raised"),
    );
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

/// Review 1's steps through the GPU's production `App`: the error is
/// raised while the `M-x` prompt holds the band, and after `C-g` and one
/// unrelated key the band carries it. At `8f5fe3a` the daemon counted
/// it shown under the prompt (`under.status` held it beside
/// `under.prompt="M-x "`) and the key after `C-g` spent it, so `next`
/// read `""`.
#[cfg(feature = "crdt")]
#[test]
fn e10a_7_the_gpu_band_shows_an_error_raised_under_a_prompt_once_it_is_gone() {
    use common::daemon::TestDaemon;

    let dir = std::env::temp_dir().join(format!("pmacs-e10a7-gpuprompt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "one\ntwo\nthree\n").unwrap();
    let init = format!(
        "{LATER_INIT}pmacs.buffer.find_or_open({:?})\n",
        notes.display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let steps = "shows:notes.txt key:C-c key:z key:M-x quiet:3000 place:under \
                 key:C-g quiet:700 place:after key:C-n quiet:1500 place:next";
    let Some(facts) =
        common::gpu_probe::run_script_probe(daemon.socket_path(), "e10a7prompt", steps)
    else {
        support::record_skip("the GPU probe did not run");
        return;
    };
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert!(
        fact("under.prompt") == "\"M-x \"" && fact("under.right").contains("!1"),
        "premise: the error was raised while the M-x prompt was open; facts {facts:?}"
    );
    assert_eq!(
        (fact("after.prompt"), fact("next.line")),
        ("\"\"".to_owned(), "2".to_owned()),
        "premise: C-g closed the prompt and C-n moved the caret; facts {facts:?}"
    );
    assert!(
        fact("next.status").contains("r1: raised with no key pressed"),
        "the band shows the error raised under the prompt once it is gone; facts {facts:?}"
    );
}
