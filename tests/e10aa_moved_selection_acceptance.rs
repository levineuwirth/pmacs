// tests/e10aa_moved_selection_acceptance.rs --- E10aa (#546): a moved
// selection takes RET in `find-file` and `write-file`.

//! D18 as amended 2026-11-25 (the owner's ruling on #546): under the
//! `typed` accept policy a selection the user has moved is what RET
//! takes. A move is Up or Down over candidates, or `M-n`/`M-p`; any
//! change to the field's text since the move returns RET to the field,
//! and `C-j` always takes the field. With no move `typed` is as before:
//! `C-x C-f nots RET` creates `nots`, and RET on the untouched prefilled
//! directory opens it in dired.
//!
//! The felt case, at `de59feb`: `C-x C-f`, Down (the prompt showing
//! `[notes.txt]`), RET opened the directory in dired.
//!
//! Every row runs twice through the outermost path: a real TUI in a PTY,
//! its keys read by crossterm as a terminal's, and a real TUI attached to
//! a daemon, whose keys reach the daemon as `FrontendEvent::Key`. What
//! the editor then holds is read from the editor itself: `C-c 9` runs a
//! command this suite defines, which writes the active buffer's name and
//! path and every buffer's name to a file beside `HOME`, so a row reads
//! the daemon's state and not only the screen.
//!
//! A row about what returns RET to the field (a typed letter, `C-j`, a
//! recompute that selects another candidate, no move at all) would pass
//! at `de59feb`, where RET under `typed` never takes the selection. So
//! each first shows, in its own session, that a move takes RET there
//! ([`Session::premise_a_move_takes_ret`]), and only then that its own
//! action gives RET back to the field: it fails at the base on that
//! premise and shows its action, not the base's indifference.
//!
//! The fixtures are `.txt` files, so no language server starts.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::daemon::TestDaemon;
use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

const ROWS: u16 = 24;
/// Wide enough for the overwrite question's whole path on one row.
const COLS: u16 = 220;
const PATIENCE: Duration = Duration::from_secs(30);
/// How long a screen must stay unchanged to count as answered.
const QUIET: Duration = Duration::from_millis(400);

/// The report command, bound to `C-c 9`, and nothing else changed.
const REPORT_INIT: &str = r#"
pmacs.lsp.config = {}
local reports = 0
pmacs.command.define {
  name = "e10aa.report",
  description = "Write what the editor holds, for an E10aa row to read.",
  fn = function()
    reports = reports + 1
    local lines = {}
    local buf = pmacs.window.buffer()
    lines[#lines + 1] = "active=" .. pmacs.describe.buffer(buf).name
    local ok, path = pcall(function() return buf:path() end)
    lines[#lines + 1] = "path=" .. tostring(ok and path or "")
    for _, id in ipairs(pmacs.buffer.list()) do
      lines[#lines + 1] = "buffer=" .. pmacs.describe.buffer(id).name
    end
    local f = assert(io.open(os.getenv("HOME") .. "/e10aa.report", "w"))
    f:write(table.concat(lines, "\n"), "\n")
    f:close()
    pmacs.editor.set_status("e10aa: reported " .. reports)
  end,
}
pmacs.keymap.bind { scope = "global", sequence = "C-c 9", command = "e10aa.report" }
"#;

const C_X: &[u8] = b"\x18";
const C_F: &[u8] = b"\x06";
const C_W: &[u8] = b"\x17";
const C_J: &[u8] = b"\n";
const C_D: &[u8] = b"\x04";
const RET: &[u8] = b"\r";
const DOWN: &[u8] = b"\x1b[B";
const UP: &[u8] = b"\x1b[A";
const M_N: &[u8] = b"\x1bn";

#[derive(Clone, Copy, Debug)]
enum Route {
    /// `pmacs <file>` in a PTY: the editor in the TUI's own process.
    InProcess,
    /// `pmacs --attach --socket <daemon>` in a PTY.
    Attached,
}

/// What `C-c 9` wrote.
#[derive(Debug)]
struct Report {
    active: String,
    path: String,
    buffers: Vec<String>,
}

impl Report {
    fn dired(&self) -> Vec<&String> {
        self.buffers.iter().filter(|b| b.ends_with('/')).collect()
    }
}

/// A TUI on `proj/start.txt`, in a directory holding `alpha.txt`,
/// `notes.txt` and `start.txt` (the issue's), so `find-file` lists those
/// three in that order and selects `alpha.txt`.
struct Session {
    tui: ScreenTui,
    route: Route,
    proj: PathBuf,
    report: PathBuf,
    reports: u32,
    _daemon: Option<TestDaemon>,
    _root: tempfile::TempDir,
}

impl Session {
    fn open(route: Route) -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let proj = root.path().join("proj");
        std::fs::create_dir_all(&proj).expect("proj");
        for name in ["alpha", "notes", "start"] {
            std::fs::write(proj.join(format!("{name}.txt")), format!("{name}\n"))
                .expect("write a fixture file");
        }
        let start = proj.join("start.txt").display().to_string();
        let (tui, report, daemon) = match route {
            Route::InProcess => {
                let config = root.path().join("config");
                std::fs::create_dir_all(config.join("pmacs")).expect("config");
                std::fs::write(config.join("pmacs/init.lua"), REPORT_INIT).expect("init.lua");
                // This session's own roots, its prompt history among them.
                let state = root.path().join("state");
                let data = root.path().join("data");
                let cache = root.path().join("cache");
                for dir in [&state, &data, &cache] {
                    std::fs::create_dir_all(dir).expect("a root");
                }
                let (pty, tap) = spawn_pmacs_in_pty_tapped(
                    &[&start],
                    &[
                        ("HOME", root.path()),
                        ("XDG_CONFIG_HOME", &config),
                        ("XDG_STATE_HOME", &state),
                        ("PMACS_STATE_HOME", &state),
                        ("XDG_DATA_HOME", &data),
                        ("XDG_CACHE_HOME", &cache),
                    ],
                    ROWS,
                    COLS,
                );
                (
                    ScreenTui::new(pty, tap, ROWS, COLS),
                    root.path().join("e10aa.report"),
                    None,
                )
            }
            Route::Attached => {
                let init = format!(
                    "{REPORT_INIT}\npmacs.buffer.find_or_open({start:?})\n\
                     for _, b in ipairs(pmacs.buffer.list()) do\n  \
                     if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\nend\n"
                );
                let daemon = TestDaemon::spawn_with_config(&init);
                let socket = daemon.socket_path().to_str().expect("utf-8").to_owned();
                // The daemon's HOME is its socket's directory.
                let report = daemon
                    .socket_path()
                    .parent()
                    .expect("the socket's directory")
                    .join("e10aa.report");
                let (pty, tap) = spawn_pmacs_in_pty_tapped(
                    &["--attach", "--socket", &socket],
                    &[("HOME", root.path()), ("XDG_CONFIG_HOME", root.path())],
                    ROWS,
                    COLS,
                );
                (ScreenTui::new(pty, tap, ROWS, COLS), report, Some(daemon))
            }
        };
        let mut s = Self {
            tui,
            route,
            proj,
            report,
            reports: 0,
            _daemon: daemon,
            _root: root,
        };
        s.tui.wait_for(PATIENCE, "start.txt on screen", &|t| {
            t.screen_contains("start.txt") && t.screen_contains("start")
        });
        let r = s.report();
        assert_eq!(
            r.active, "start.txt",
            "premise: the session opens on start.txt; {r:?}"
        );
        s
    }

    /// Write `bytes`, then read until the bottom row holds `needle`.
    fn key(&mut self, bytes: &[u8], needle: &str) {
        self.tui.write(bytes);
        let what = format!("{:?}: {needle:?} on the bottom row", self.route);
        self.tui
            .wait_for(PATIENCE, &what, &|t| t.bottom_row().contains(needle));
    }

    /// `C-x C-f`, until the prompt shows its first candidate selected.
    fn find_file(&mut self) {
        self.tui.write(C_X);
        self.key(C_F, "Find file: ");
        self.tui
            .wait_for(PATIENCE, "the prompt's [alpha.txt]", &|t| {
                t.bottom_row().contains("[alpha.txt]")
            });
    }

    /// An accepting key, and the screen settled with `prompt` gone.
    fn accept(&mut self, bytes: &[u8], prompt: &str) {
        self.tui.write(bytes);
        let what = format!(
            "{:?}: the screen after the accept, {prompt:?} gone",
            self.route
        );
        self.tui.settle(QUIET, PATIENCE, &what, &|t| {
            !t.bottom_row().contains(prompt)
        });
    }

    /// `C-c 9`, and what it wrote.
    fn report(&mut self) -> Report {
        self.reports += 1;
        let said = format!("e10aa: reported {}", self.reports);
        self.tui.write(b"\x03");
        self.key(b"9", &said);
        let text = std::fs::read_to_string(&self.report).expect("the report file");
        let mut r = Report {
            active: String::new(),
            path: String::new(),
            buffers: Vec::new(),
        };
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("active=") {
                v.clone_into(&mut r.active);
            } else if let Some(v) = line.strip_prefix("path=") {
                v.clone_into(&mut r.path);
            } else if let Some(v) = line.strip_prefix("buffer=") {
                r.buffers.push(v.to_owned());
            }
        }
        r
    }

    fn in_proj(&self, name: &str) -> String {
        self.proj.join(name).display().to_string()
    }

    /// #546's case in this session: `C-x C-f`, Down, RET opens the
    /// selected `notes.txt`. At `de59feb` it opened `proj/` in dired.
    fn premise_a_move_takes_ret(&mut self) {
        self.find_file();
        self.key(DOWN, "[notes.txt]");
        self.accept(RET, "Find file: ");
        let r = self.report();
        assert_eq!(
            r.active, "notes.txt",
            "{:?}: premise: in this session a move takes RET (C-x C-f, Down, RET opens \
             notes.txt); {r:?}",
            self.route
        );
    }
}

fn same_dir(path: &str, dir: &Path) -> bool {
    Path::new(path).parent() == Some(dir)
}

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// #546: `C-x C-f`, Down, RET opens the selected file, and no dired
/// buffer exists. At `de59feb` it opened `proj/` in dired.
fn down_ret_opens_the_selection(route: Route) {
    let mut s = Session::open(route);
    s.find_file();
    s.key(DOWN, "[notes.txt]");
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "notes.txt",
        "{route:?}: RET takes the moved selection; {r:?}"
    );
    assert_eq!(
        r.path,
        s.in_proj("notes.txt"),
        "{route:?}: the file itself; {r:?}"
    );
    assert!(r.dired().is_empty(), "{route:?}: no dired buffer; {r:?}");
}

/// `M-n` is a move as Down is.
fn m_n_ret_opens_the_selection(route: Route) {
    let mut s = Session::open(route);
    s.find_file();
    s.key(M_N, "[notes.txt]");
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "notes.txt",
        "{route:?}: RET takes M-n's selection; {r:?}"
    );
    assert!(r.dired().is_empty(), "{route:?}: no dired buffer; {r:?}");
}

/// Down then Up returns to the first candidate, which RET opens: the
/// user chose it, though it is where the prompt began.
fn down_up_ret_opens_the_first_candidate(route: Route) {
    let mut s = Session::open(route);
    s.find_file();
    s.key(DOWN, "[notes.txt]");
    s.key(UP, "[alpha.txt]");
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "alpha.txt",
        "{route:?}: RET takes the first candidate, chosen; {r:?}"
    );
    assert!(r.dired().is_empty(), "{route:?}: no dired buffer; {r:?}");
}

/// A move then a typed letter creates the typed name. `n` leaves
/// `notes.txt`, the candidate moved to, selected, so only the change to
/// the field returns RET to it.
fn move_then_letter_creates_the_typed_name(route: Route) {
    let mut s = Session::open(route);
    s.premise_a_move_takes_ret();
    s.find_file();
    s.key(DOWN, "[notes.txt]");
    s.key(b"n", "/proj/n  [notes.txt]");
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(r.active, "n", "{route:?}: the typed name, created; {r:?}");
    assert!(same_dir(&r.path, &s.proj), "{route:?}: `n` in proj; {r:?}");
    assert!(
        !Path::new(&r.path).exists(),
        "{route:?}: a new file, not yet written; {r:?}"
    );
}

/// No move: `C-x C-f no RET` still creates `no` with `notes.txt`
/// selected (E6.1's fix). Its premise is the same field with a move.
fn no_move_creates_no(route: Route) {
    let mut s = Session::open(route);
    s.find_file();
    s.key(b"n", "/proj/n  [notes.txt]");
    s.key(b"o", "/proj/no  [notes.txt]");
    s.key(DOWN, "/proj/no  [notes.txt]");
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "notes.txt",
        "{route:?}: premise: `no`, Down, RET opens the selected notes.txt; {r:?}"
    );
    s.find_file();
    s.key(b"n", "/proj/n  [notes.txt]");
    s.key(b"o", "/proj/no  [notes.txt]");
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "no",
        "{route:?}: with no move the typed `no` is created; {r:?}"
    );
    assert!(same_dir(&r.path, &s.proj), "{route:?}: `no` in proj; {r:?}");
}

/// No move: RET on the untouched prefilled directory opens it in dired
/// (E6.1's fix, the other half).
fn no_move_ret_on_the_prefill_opens_dired(route: Route) {
    let mut s = Session::open(route);
    s.premise_a_move_takes_ret();
    s.find_file();
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "proj/",
        "{route:?}: with no move RET opens the directory; {r:?}"
    );
}

/// A move then `C-j` on the untouched prefill opens dired: `C-j` takes
/// the field whatever is selected.
fn move_then_c_j_opens_dired(route: Route) {
    let mut s = Session::open(route);
    s.premise_a_move_takes_ret();
    s.find_file();
    s.key(DOWN, "[notes.txt]");
    s.accept(C_J, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "proj/",
        "{route:?}: C-j takes the field, the directory; {r:?}"
    );
}

/// A recompute that leaves the same candidate selected keeps the move:
/// `C-d` at the end of the field deletes nothing and recomputes. In one
/// prompt: Down, then `C-d`, which puts the selection back on
/// `alpha.txt` (so `C-d` recomputes here); then Down and Up, a move to
/// `alpha.txt`, and `C-d` again, the selection staying on `alpha.txt`;
/// RET opens it.
fn recompute_on_the_same_candidate_keeps_the_move(route: Route) {
    let mut s = Session::open(route);
    s.find_file();
    s.key(DOWN, "[notes.txt]");
    s.key(C_D, "/proj/  [alpha.txt]");
    s.key(DOWN, "[notes.txt]");
    s.key(UP, "[alpha.txt]");
    s.tui.write(C_D);
    s.tui.settle(QUIET, PATIENCE, "the prompt after C-d", &|t| {
        t.bottom_row().contains("/proj/  [alpha.txt]")
    });
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "alpha.txt",
        "{route:?}: the recompute kept alpha.txt selected, so the move stands; {r:?}"
    );
}

/// A recompute that selects another candidate clears the move: Down,
/// then `C-d` at the end, which recomputes and puts the selection back
/// on `alpha.txt`; RET takes the field, the directory.
fn recompute_on_another_candidate_clears_the_move(route: Route) {
    let mut s = Session::open(route);
    s.premise_a_move_takes_ret();
    s.find_file();
    s.key(DOWN, "[notes.txt]");
    s.tui.write(C_D);
    s.tui
        .wait_for(PATIENCE, "the selection back on alpha.txt", &|t| {
            t.bottom_row().contains("/proj/  [alpha.txt]")
        });
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.active, "proj/",
        "{route:?}: the recompute selected another candidate, so RET takes the field; {r:?}"
    );
}

/// A history recall returns RET to the field. The premise puts
/// `proj/notes.txt` in `find-file`'s history; then Down, Down
/// (`[start.txt]`), `C-p` recalls `proj/notes.txt` into the field, and
/// RET opens that, not the `start.txt` the move chose.
fn history_recall_returns_ret_to_the_field(route: Route) {
    let mut s = Session::open(route);
    s.premise_a_move_takes_ret();
    // Back to start.txt, so the RET below is seen to open something.
    s.tui.write(C_X);
    s.key(b"b", "Switch to buffer: ");
    for ch in "start.txt".bytes() {
        s.tui.write(&[ch]);
    }
    s.tui
        .wait_for(PATIENCE, "the switch prompt's start.txt", &|t| {
            t.bottom_row().contains("Switch to buffer: start.txt")
        });
    s.accept(RET, "Switch to buffer: ");
    let r = s.report();
    assert_eq!(
        r.active, "start.txt",
        "{route:?}: premise: back on start.txt; {r:?}"
    );
    s.find_file();
    s.key(DOWN, "[notes.txt]");
    s.key(DOWN, "[start.txt]");
    s.key(b"\x10", "/proj/notes.txt");
    s.accept(RET, "Find file: ");
    let r = s.report();
    assert_eq!(
        r.path,
        s.in_proj("notes.txt"),
        "{route:?}: the recalled field, not the selection a move chose before it; {r:?}"
    );
    assert_eq!(r.active, "notes.txt", "{route:?}: {r:?}");
}

/// `C-x C-w`, a move, RET writes to the selected file, after the
/// overwrite question it already asks. At `de59feb` RET took the field,
/// the directory.
fn write_file_move_ret_writes_the_selection(route: Route) {
    let mut s = Session::open(route);
    s.tui.write(C_X);
    s.key(C_W, "Write file: ");
    s.tui.wait_for(PATIENCE, "the prompt's [alpha.txt]", &|t| {
        t.bottom_row().contains("[alpha.txt]")
    });
    s.key(DOWN, "[notes.txt]");
    s.tui.write(RET);
    let question = format!("{} exists; overwrite?", s.in_proj("notes.txt"));
    let asked = s.tui.watch_for(Duration::from_secs(10), &|t| {
        t.bottom_row().contains(&question)
    });
    assert!(
        asked.is_some(),
        "{route:?}: RET after the move asks {question:?}; the bottom row is {:?}",
        s.tui.bottom_row()
    );
    s.key(b"y", "y");
    s.key(b"e", "ye");
    s.key(b"s", "yes");
    s.accept(RET, "overwrite?");
    let r = s.report();
    assert_eq!(
        r.path,
        s.in_proj("notes.txt"),
        "{route:?}: the buffer adopts notes.txt; {r:?}"
    );
    assert_eq!(
        std::fs::read_to_string(s.proj.join("notes.txt")).expect("notes.txt"),
        "start\n",
        "{route:?}: start.txt's text written over notes.txt"
    );
}

// ---------------------------------------------------------------------------
// The rows: each case on each route
// ---------------------------------------------------------------------------

macro_rules! on_both_routes {
    ($($case:ident => $in_process:ident, $attached:ident;)*) => {$(
        #[test]
        fn $in_process() {
            $case(Route::InProcess);
        }
        #[test]
        fn $attached() {
            $case(Route::Attached);
        }
    )*};
}

on_both_routes! {
    down_ret_opens_the_selection =>
        e10aa_tui_c_x_c_f_down_ret_opens_the_selected_file_and_no_dired,
        e10aa_attached_c_x_c_f_down_ret_opens_the_selected_file_and_no_dired;
    m_n_ret_opens_the_selection =>
        e10aa_tui_m_n_is_a_move_as_down_is,
        e10aa_attached_m_n_is_a_move_as_down_is;
    down_up_ret_opens_the_first_candidate =>
        e10aa_tui_down_then_up_ret_opens_the_first_candidate,
        e10aa_attached_down_then_up_ret_opens_the_first_candidate;
    move_then_letter_creates_the_typed_name =>
        e10aa_tui_a_move_then_a_typed_letter_creates_the_typed_name,
        e10aa_attached_a_move_then_a_typed_letter_creates_the_typed_name;
    no_move_creates_no =>
        e10aa_tui_no_move_c_x_c_f_no_ret_still_creates_no,
        e10aa_attached_no_move_c_x_c_f_no_ret_still_creates_no;
    no_move_ret_on_the_prefill_opens_dired =>
        e10aa_tui_no_move_ret_on_the_prefill_still_opens_dired,
        e10aa_attached_no_move_ret_on_the_prefill_still_opens_dired;
    move_then_c_j_opens_dired =>
        e10aa_tui_a_move_then_c_j_on_the_prefill_opens_dired,
        e10aa_attached_a_move_then_c_j_on_the_prefill_opens_dired;
    recompute_on_the_same_candidate_keeps_the_move =>
        e10aa_tui_a_recompute_on_the_same_candidate_keeps_the_move,
        e10aa_attached_a_recompute_on_the_same_candidate_keeps_the_move;
    recompute_on_another_candidate_clears_the_move =>
        e10aa_tui_a_recompute_on_another_candidate_clears_the_move,
        e10aa_attached_a_recompute_on_another_candidate_clears_the_move;
    history_recall_returns_ret_to_the_field =>
        e10aa_tui_a_history_recall_returns_ret_to_the_field,
        e10aa_attached_a_history_recall_returns_ret_to_the_field;
    write_file_move_ret_writes_the_selection =>
        e10aa_tui_c_x_c_w_a_move_ret_writes_the_selected_file_after_the_question,
        e10aa_attached_c_x_c_w_a_move_ret_writes_the_selected_file_after_the_question;
}
