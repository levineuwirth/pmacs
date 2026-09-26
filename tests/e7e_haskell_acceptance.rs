// tests/e7e_haskell_acceptance.rs --- aside E7e, Haskell.

//! Haskell is three registrations: the `tree-sitter-haskell` grammar in
//! `BUILTIN_LANGUAGES` claiming `.hs`, and `pmacs.lsp.config.haskell`
//! running `haskell-language-server-wrapper --lsp`. The grammar rows run
//! everywhere and spawn nothing (the LSP table is emptied first); the
//! server row runs against the real HLS inside a cabal project and is
//! gated by `PMACS_REQUIRE_HLS`.
//!
//! The server row types its error through the production key dispatch,
//! because a Lua `buf:insert` outside a command reaches no server until
//! the next keystroke.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Color, Glyph};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};

#[path = "common/iso.rs"]
mod iso;
#[path = "support/mod.rs"]
mod support;

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

/// Tick until `pred` holds or `secs` pass.
fn wait(s: &mut EditorState, secs: u64, mut pred: impl FnMut(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if pred(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fresh editor whose state and project search stop at `dir`.
fn editor_in(dir: &Path) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(
        &s,
        &format!(
            "pmacs.project.set_search_boundary({:?})",
            dir.display().to_string()
        ),
    );
    s
}

fn open(s: &EditorState, path: &Path) {
    exec(
        s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ),
    );
}

const ROWS: u32 = 24;
const COLS: u32 = 100;

/// The grid frontend's frame through `paint_frame`, one `Vec<Cell>` per
/// row.
fn frame(s: &EditorState) -> Vec<Vec<Cell>> {
    let size = CellSize::new(ROWS, COLS);
    let mut cells = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: COLS,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    cells.chunks(COLS as usize).map(<[Cell]>::to_vec).collect()
}

fn text_of(row: &[Cell]) -> String {
    row.iter()
        .map(|c| match &c.glyph {
            Glyph::Char(ch) => *ch,
            _ => ' ',
        })
        .collect()
}

/// The foreground painted on the first cell of `needle` in the first row
/// that contains `line`.
fn fg_of(frame: &[Vec<Cell>], line: &str, needle: &str) -> Option<Color> {
    let row = frame.iter().find(|r| text_of(r).contains(line))?;
    let text = text_of(row);
    let col = text.find(line)? + line.find(needle)?;
    // `text` is char-indexed like `row` only while it is ASCII, which
    // every fixture line here is.
    Some(row[col].style.fg)
}

fn buffer_language(s: &EditorState) -> Option<String> {
    eval(
        s,
        "return pmacs.parse.buffer_language(pmacs.window.buffer())",
    )
}

/// The language of the parse tree installed for the active buffer, once
/// one has settled.
fn tree_language(s: &EditorState) -> Option<String> {
    eval(
        s,
        "local t = pmacs.parse.tree(pmacs.window.buffer()) return t and t:language() or nil",
    )
}

const MAIN_HS: &str = "module Main (main) where\n\
                       \n\
                       greeting :: String\n\
                       greeting = \"hello\" -- said once\n\
                       \n\
                       main :: IO ()\n\
                       main = putStrLn greeting\n";

#[test]
fn e7e_a_hs_file_is_haskell_and_the_grid_paints_its_syntax() {
    let dir = temp_dir("grammar");
    let s = editor_in(&dir);
    exec(&s, "pmacs.lsp.config = {}");
    let file = dir.join("Main.hs");
    std::fs::write(&file, MAIN_HS).unwrap();
    open(&s, &file);
    let mut s = s;
    assert_eq!(buffer_language(&s).as_deref(), Some("haskell"));
    assert!(
        wait(&mut s, 10, |s| tree_language(s).as_deref()
            == Some("haskell")),
        "a haskell parse settles"
    );
    let mut painted = Vec::new();
    assert!(
        wait(&mut s, 10, |s| {
            painted = frame(s);
            fg_of(&painted, "module Main", "module") == Some(Color::Indexed(5))
        }),
        "`module` paints the keyword style; frame:\n{}",
        painted
            .iter()
            .map(|r| text_of(r))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let f = &painted;
    assert_eq!(
        fg_of(f, "(main) where", "where"),
        Some(Color::Indexed(5)),
        "`where`"
    );
    assert_eq!(
        fg_of(f, "greeting :: String", "String"),
        Some(Color::Indexed(3)),
        "a type"
    );
    assert_eq!(
        fg_of(f, "greeting = \"hello\"", "\"hello\""),
        Some(Color::Indexed(2)),
        "a string"
    );
    assert_eq!(
        fg_of(f, "-- said once", "--"),
        Some(Color::Indexed(8)),
        "a comment"
    );
    assert_eq!(
        fg_of(f, "main :: IO ()", "::"),
        Some(Color::Indexed(6)),
        "`::`"
    );
}

#[test]
fn e7e_an_lhs_file_is_not_haskell_and_paints_plain() {
    let dir = temp_dir("lhs");
    let s = editor_in(&dir);
    exec(&s, "pmacs.lsp.config = {}");
    let file = dir.join("Main.lhs");
    std::fs::write(
        &file,
        "A literate module.\n\n> module Main where\n> main = pure ()\n",
    )
    .unwrap();
    open(&s, &file);
    let mut s = s;
    assert_ne!(buffer_language(&s).as_deref(), Some("haskell"));
    // Nothing to settle: tick a while and read the frame.
    wait(&mut s, 1, |_| false);
    assert_eq!(tree_language(&s), None, "no grammar parses an .lhs file");
    assert_eq!(
        fg_of(&frame(&s), "> module Main where", "module"),
        Some(Color::Default),
        "`module` in a Bird track is not painted as a keyword"
    );
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

fn stdout_of(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The `with-compiler:` the fixture's `cabal.project` needs so the server
/// the wrapper starts was built against the GHC the project compiles
/// with: none when one exists for the default `ghc`, else the GHC the
/// installed server names, if it is on PATH. HLS on a mismatched GHC
/// attaches and reports only the mismatch, so this is the fixture's to
/// arrange and not the row's to witness.
fn compiler_pin() -> Result<Option<String>, String> {
    let default =
        stdout_of("ghc", &["--numeric-version"]).ok_or("`ghc --numeric-version` failed")?;
    let minor = default
        .rsplit_once('.')
        .map_or(default.as_str(), |(m, _)| m);
    if on_path(&format!("haskell-language-server-{default}"))
        || on_path(&format!("haskell-language-server-{minor}"))
    {
        return Ok(None);
    }
    let version = stdout_of("haskell-language-server", &["--version"])
        .ok_or("no haskell-language-server for the default ghc, and no unsuffixed one")?;
    let built = version
        .split("(GHC: ")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .ok_or_else(|| format!("no GHC in `{version}`"))?
        .to_owned();
    if built == default {
        return Ok(None);
    }
    let pinned = format!("ghc-{built}");
    if on_path(&pinned) {
        Ok(Some(pinned))
    } else {
        Err(format!(
            "haskell-language-server is built against GHC {built}, the default ghc is \
             {default}, and `{pinned}` is not on PATH"
        ))
    }
}

fn lsp_segment(s: &EditorState) -> Option<String> {
    let outcome = evaluate_statusline(
        s.lua_host.lua(),
        &s.core,
        &s.statusline_registry,
        StatuslineEvaluationTarget::Grid {
            frontend_id: FrontendId::LOCAL,
        },
    );
    let StatuslineEvaluationOutcome::Ready(windows) = outcome.outcome else {
        return None;
    };
    windows
        .into_iter()
        .flat_map(|w| w.right)
        .find(|seg| seg.face == "ui.modeline.lsp")
        .map(|seg| seg.text.trim_end().to_owned())
}

/// Every diagnostic the store holds for the active document:
/// `(severity, start_line, start_col, end_line, end_col, first line)`.
fn diagnostics(s: &EditorState) -> Vec<(String, i64, i64, i64, i64, String)> {
    let rows: Vec<mlua::Table> = eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         if not rec then return {} end
         return pmacs.diag.list(rec.uri)",
    );
    rows.into_iter()
        .map(|d| {
            let message: String = d.get("message").unwrap();
            (
                d.get("severity").unwrap(),
                d.get("start_line").unwrap(),
                d.get("start_col").unwrap(),
                d.get("end_line").unwrap(),
                d.get("end_col").unwrap(),
                message.lines().next().unwrap_or("").trim().to_owned(),
            )
        })
        .collect()
}

fn canonical(path: &Path) -> String {
    std::fs::canonicalize(path).unwrap().display().to_string()
}

/// `pmacs.lsp.config.haskell.root` on `file`, as `project_root_for`
/// calls it.
fn resolve_root(s: &EditorState, file: &Path) -> Option<String> {
    eval(
        s,
        &format!(
            "return pmacs.lsp.config.haskell.root({:?})",
            file.display().to_string()
        ),
    )
}

#[test]
fn e7e_the_haskell_root_is_the_innermost_project_marker() {
    let top = temp_dir("root");
    let s = editor_in(&top);
    let write = |rel: &str| {
        let p = top.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
        p
    };
    // `cabal init`: the package named after its directory.
    let file = write("myproj/app/Main.hs");
    write("myproj/myproj.cabal");
    assert_eq!(
        resolve_root(&s, &file),
        Some(canonical(&top.join("myproj")))
    );
    // `stack new`: `stack.yaml` at the top.
    let file = write("stacked/src/Lib.hs");
    write("stacked/stack.yaml");
    assert_eq!(
        resolve_root(&s, &file),
        Some(canonical(&top.join("stacked")))
    );
    // Several packages under one `cabal.project`: the innermost marker,
    // the package, whose `cabal` finds the project above it.
    let file = write("multi/pkg-a/src/A.hs");
    write("multi/cabal.project");
    write("multi/pkg-a/pkg-a.cabal");
    assert_eq!(
        resolve_root(&s, &file),
        Some(canonical(&top.join("multi/pkg-a")))
    );
    // A `.cabal` file not named after its directory is not seen, and a
    // bare file has no marker: both decline to the marker walk.
    let file = write("renamed/app/Main.hs");
    write("renamed/other.cabal");
    assert_eq!(resolve_root(&s, &file), None);
    let file = write("bare/Script.hs");
    assert_eq!(resolve_root(&s, &file), None);
    // A directory named like a marker is not one.
    let file = write("dirmark/app/Main.hs");
    std::fs::create_dir_all(top.join("dirmark/cabal.project")).unwrap();
    assert_eq!(resolve_root(&s, &file), None);
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

fn type_line(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
    }
    press(s, KeyCode::Enter);
}

fn buffer_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    )
}

/// The grid's status row: the frame's last row.
fn status_row(s: &EditorState) -> String {
    text_of(frame(s).last().unwrap()).trim_end().to_owned()
}

/// `cabal init`'s layout under a fresh directory: `hsproj/hsproj.cabal`
/// named after its directory, the module in `hsproj/app/`, no `.git`, and
/// a `cabal.project` only to carry `pin`. The marker walk alone would
/// root HLS at `app/`, where it finds no component. Returns the top, the
/// project and the module.
fn cabal_init_fixture(pin: Option<&str>) -> (PathBuf, PathBuf, PathBuf) {
    let top = temp_dir("hls");
    let dir = top.join("hsproj");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("hsproj.cabal"),
        "cabal-version:      3.0\n\
         name:               hsproj\n\
         version:            0.1.0.0\n\
         build-type:         Simple\n\
         \n\
         executable hsproj\n\
         \x20   main-is:          Main.hs\n\
         \x20   hs-source-dirs:   app\n\
         \x20   build-depends:    base\n\
         \x20   default-language: Haskell2010\n",
    )
    .unwrap();
    if let Some(ghc) = pin {
        std::fs::write(
            dir.join("cabal.project"),
            format!("packages: .\nwith-compiler: {ghc}\n"),
        )
        .unwrap();
    }
    std::fs::create_dir_all(dir.join("app")).unwrap();
    let file = dir.join("app").join("Main.hs");
    std::fs::write(&file, MAIN_HS).unwrap();
    (top, dir, file)
}

#[test]
fn e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error() {
    for tool in ["haskell-language-server-wrapper", "cabal", "ghc"] {
        if !on_path(tool) {
            support::skip_or_fail(tool, "PMACS_REQUIRE_HLS");
            return;
        }
    }
    let pin = match compiler_pin() {
        Ok(pin) => pin,
        Err(why) => {
            support::skip_or_fail(
                &format!("a GHC haskell-language-server serves ({why})"),
                "PMACS_REQUIRE_HLS",
            );
            return;
        }
    };

    let (top, dir, file) = cabal_init_fixture(pin.as_deref());
    let s = editor_in(&top);
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    assert_eq!(
        resolve_root(&s, &file),
        Some(canonical(&dir)),
        "the Haskell root is the `.cabal` file's directory"
    );
    open(&s, &file);
    let mut s = s;
    eprintln!("E7E with-compiler {pin:?}");

    // Attached: the modeline names the server's state and settles on
    // `ready`. That is not yet the file checked; the error below is.
    let t0 = Instant::now();
    let mut trace: Vec<(u128, String)> = Vec::new();
    let note = |s: &EditorState, trace: &mut Vec<(u128, String)>| {
        let text = lsp_segment(s).unwrap_or_default();
        if trace.last().map(|(_, t)| t) != Some(&text) {
            trace.push((t0.elapsed().as_millis(), text));
        }
    };
    assert!(
        wait(&mut s, 120, |s| {
            note(s, &mut trace);
            lsp_segment(s).as_deref() == Some("LSP:ready")
        }),
        "the modeline settles on LSP:ready; trace {trace:?}"
    );

    // A type error typed at the end of the file.
    let len: i64 = eval(&s, "return pmacs.window.buffer():len()");
    exec(&s, &format!("pmacs.editor.goto_byte({len})"));
    type_line(&mut s, "oops :: Int");
    type_line(&mut s, "oops = True");
    let text = buffer_text(&s);
    assert!(
        text.ends_with("oops :: Int\noops = True\n"),
        "typed: {text:?}"
    );

    let mut seen = Vec::new();
    assert!(
        wait(&mut s, 120, |s| {
            note(s, &mut trace);
            seen = diagnostics(s);
            seen.iter()
                .any(|d| d.0 == "error" && d.5.contains("Couldn't match"))
        }),
        "HLS reports the type error; store {seen:?}; trace {trace:?}"
    );
    let err = seen
        .iter()
        .find(|d| d.0 == "error" && d.5.contains("Couldn't match"))
        .unwrap()
        .clone();
    eprintln!(
        "E7E diagnostic {err:?} after {} ms; trace {trace:?}",
        t0.elapsed().as_millis()
    );
    // The component loaded: the typed error is the only one, so no
    // cradle error (a bare-`ghc` fallback, a GHC mismatch) sits on line 0.
    let errors: Vec<_> = seen.iter().filter(|d| d.0 == "error").collect();
    assert_eq!(errors, vec![&err], "the typed error is the only error");

    // Placed on the text it names: E7c carries the range to the current
    // text, and it covers `True`.
    let line = text.lines().nth(usize::try_from(err.1).unwrap()).unwrap();
    let covered = &line[usize::try_from(err.2).unwrap()..usize::try_from(err.4).unwrap()];
    assert_eq!(
        (err.1, err.3, covered),
        (8, 8, "True"),
        "the squiggle is on `True`"
    );

    // And E7d's diagnostic at point reads it on the grid's status row.
    let at = i64::try_from(text.find("oops = True").unwrap() + "oops = ".len()).unwrap();
    exec(&s, &format!("pmacs.editor.goto_byte({at})"));
    tick(&mut s);
    let status = status_row(&s);
    assert!(
        status.contains("error:") && status.contains("Couldn't match"),
        "the status row names the error at point: {status:?}"
    );
    assert!(
        wait(&mut s, 60, |s| lsp_segment(s).as_deref()
            == Some("LSP:ready")),
        "the modeline returns to LSP:ready"
    );
}
