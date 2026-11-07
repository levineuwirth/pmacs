// tests/e7e_haskell_acceptance.rs --- aside E7e, Haskell.

//! Haskell is three registrations: the `tree-sitter-haskell` grammar in
//! `BUILTIN_LANGUAGES` claiming `.hs`, and `pmacs.lsp.config.haskell`
//! running `haskell-language-server-wrapper --lsp`. The grammar rows run
//! everywhere and spawn nothing (the LSP table is emptied first); the
//! server row runs against the real HLS inside a cabal project and is
//! gated by `PMACS_REQUIRE_HLS`.
//!
//! The server row puts its error in as one key, `C-y`, through the
//! production key dispatch, because a Lua `buf:insert` outside a command
//! reaches no server until the next keystroke.
//!
//! E7g unshipped the grammar and E7h restored it; the last row says what
//! holds it now.

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

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

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

/// The host's target triple, from `cargo -vV`. `cargo metadata` takes it as
/// `--filter-platform`, so it resolves only the crates a host build uses:
/// unfiltered and offline it needs every locked crate's source, and CI's
/// runners hold only the host's (at `cbcff4b` every test leg failed here on
/// `android-activity`, which only an Android build fetches).
fn host() -> String {
    let out = Command::new(env!("CARGO"))
        .arg("-vV")
        .output()
        .expect("cargo -vV");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .expect("cargo -vV names the host")
        .to_owned()
}

/// The file E7g reduced the owner's abort to: two `LANGUAGE` pragmas, the
/// most ordinary opening a Haskell module has.
const TWO_PRAGMAS: &str = "{-# LANGUAGE OverloadedStrings #-}\n\
                           {-# LANGUAGE ScopedTypeVariables #-}\n";

#[test]
fn e7h_tree_sitter_haskell_ships_from_crates_io_on_the_conforming_array_header() {
    // tree-sitter-haskell 0.23.1 aborted the editor on TWO_PRAGMAS (E7g):
    // its published `tree_sitter/array.h` grows an array through an
    // `(Array *)` cast and reads it back through the element pointer,
    // undefined behavior under C's aliasing rule, and GCC 16 at -O2 and -O3
    // compiled it into a write through the pointer the grow had just freed
    // (glibc's `corrupted size vs. prev_size`; ASan: a heap-buffer-overflow
    // in `advance`). E7h restored it on `-fno-strict-aliasing`, which holds
    // for every compiler measured but only in builds cargo starts at the
    // repository root (review 1: a `cargo install --git` editor aborted on
    // this file in seven of nine runs), and its fix round 1 shipped it from
    // a copy carrying the header tree-sitter's CLI has generated since 0.24.
    // At fix round 2 it ships from crates.io again: 0.24.1 was published
    // on that header (its push assigns `contents` from the grow, through
    // `_array__cast`), with the same scanner, fuzzed 600 s on the `ubsan`,
    // `asan-strict` and `tysan` arms. So what holds the grammar is the
    // published source, and the flag stays beside it. This row fails if the
    // crate moves off the version that was fuzzed (a bump is a new grammar
    // under the rule in CLAUDE.md), if it is routed to a copy again, or if
    // the source it resolves to pushes through the cast. The test build
    // compiles grammar C at -O0, where the overflow never showed, so the
    // parse below is the reproduction as a file and not a witness of the
    // optimizer.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let block = lock
        .split("[[package]]")
        .find(|b| b.lines().any(|l| l == "name = \"tree-sitter-haskell\""))
        .expect("Cargo.lock lists tree-sitter-haskell");
    assert!(
        block.lines().any(|l| l == "version = \"0.24.1\""),
        "tree-sitter-haskell moved off 0.24.1, the version E7h fuzzed; fuzz the \
         new one before it ships:{block}"
    );
    assert!(
        block
            .lines()
            .any(|l| l == "source = \"registry+https://github.com/rust-lang/crates.io-index\""),
        "tree-sitter-haskell resolves somewhere other than crates.io; a copy needs \
         its own note and row:{block}"
    );
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        !manifest.contains("vendor/tree-sitter-haskell")
            && !root.join("vendor/tree-sitter-haskell").exists(),
        "a copy of tree-sitter-haskell is back in the tree or the manifest"
    );
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--offline", "--locked"])
        .args(["--filter-platform", &host()])
        .current_dir(root)
        .output()
        .expect("cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let source = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "tree-sitter-haskell")
        .map(|p| PathBuf::from(p["manifest_path"].as_str().unwrap()))
        .expect("cargo metadata resolves tree-sitter-haskell");
    let header =
        std::fs::read_to_string(source.parent().unwrap().join("src/tree_sitter/array.h")).unwrap();
    assert!(
        !header.contains("(Array *)(self)") && header.contains("(self)->contents = _array__"),
        "{}'s array.h pushes through the aliasing cast again",
        source.display()
    );
    let config = std::fs::read_to_string(root.join(".cargo/config.toml")).unwrap();
    for var in ["HOST_CFLAGS", "TARGET_CFLAGS"] {
        assert!(
            config.contains(&format!(
                "{var} = {{ value = \"-fno-strict-aliasing\", force = true }}"
            )),
            ".cargo/config.toml still forces -fno-strict-aliasing into {var}"
        );
    }
    let dir = temp_dir("pragmas");
    let s = editor_in(&dir);
    exec(&s, "pmacs.lsp.config = {}");
    let file = dir.join("Main.hs");
    std::fs::write(&file, format!("{TWO_PRAGMAS}{MAIN_HS}")).unwrap();
    open(&s, &file);
    let mut s = s;
    assert!(
        wait(&mut s, 10, |s| tree_language(s).as_deref()
            == Some("haskell")),
        "the two-pragma file parses as haskell"
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

/// `text` goes in at point as one key, `C-y`, the kill ring holding it.
fn yank(s: &mut EditorState, text: &str) {
    exec(s, &format!("pmacs.editor.clipboard_set({text:?})"));
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
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

/// The server's own words the row reads, recorded from before the file
/// opens: the mode line reads `LSP:ready` both before setup's progress begins
/// and after it ends, and the store keeps only the last publish.
fn record_server_notifications(s: &EditorState) {
    exec(
        s,
        r#"
        e7e_setup = { begun = 0, ended = 0, tokens = {} }
        e7e_publishes = {}
        pmacs.lsp.on_notification("$/progress", function(_, p)
          local value, token = p.value, tostring(p.token)
          if value.kind == "begin" and type(value.title) == "string"
              and value.title:find("^Setting up") then
            e7e_setup.tokens[token] = true
            e7e_setup.begun = e7e_setup.begun + 1
          elseif value.kind == "end" and e7e_setup.tokens[token] then
            e7e_setup.tokens[token] = nil
            e7e_setup.ended = e7e_setup.ended + 1
          end
        end)
        pmacs.lsp.on_notification("textDocument/publishDiagnostics", function(_, p)
          local holds = false
          for _, d in ipairs(p.diagnostics) do
            if d.severity == 1 and d.message:find("Couldn't match", 1, true) then
              holds = true
            end
          end
          e7e_publishes[#e7e_publishes + 1] = {
            uri = p.uri,
            version = type(p.version) == "number" and p.version or -1,
            count = #p.diagnostics,
            holds = holds,
          }
        end)
        "#,
    );
}

/// Setup's progress has begun and every token it began on has ended.
fn setup_ended(s: &EditorState) -> bool {
    eval(
        s,
        "return e7e_setup.begun > 0 and e7e_setup.ended == e7e_setup.begun",
    )
}

/// One `textDocument/publishDiagnostics` for the row's document, as the
/// server sent it.
struct Publish {
    version: i64,
    count: i64,
    holds_error: bool,
}

/// The publishes in the order they came: `v24:error`, `v15:1`, `v25:0`.
fn shown(publishes: &[Publish]) -> String {
    publishes
        .iter()
        .map(|p| {
            if p.holds_error {
                format!("v{}:error", p.version)
            } else {
                format!("v{}:{}", p.version, p.count)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The row's document's publishes so far, in the order they came.
fn publishes(s: &EditorState) -> Vec<Publish> {
    let rows: Vec<mlua::Table> = eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         local out = {}
         for _, p in ipairs(e7e_publishes) do
           if rec and p.uri == rec.uri then out[#out + 1] = p end
         end
         return out",
    );
    rows.into_iter()
        .map(|p| Publish {
            version: p.get("version").unwrap(),
            count: p.get("count").unwrap(),
            holds_error: p.get("holds").unwrap(),
        })
        .collect()
}

/// The bounds on setup's progress ending and on the publish after the key,
/// which a passing run meets in about 10 s and well under 1 s.
const SETUP_BOUND_SECS: u64 = 120;
const PUBLISH_BOUND_SECS: u64 = 120;
/// How long the row reads on past a publish it waited for: a server's answer
/// can be several, up to 190 ms apart in the traces, and the last is the one
/// that stands.
const ANSWER_WATCH: Duration = Duration::from_millis(500);
/// The bound on the store and the status row holding what a publish held: a
/// publish for an older version can empty them, and one after it restores it.
const STORE_BOUND_SECS: u64 = 30;

/// Ticks through [`ANSWER_WATCH`].
fn read_on(s: &mut EditorState) {
    let from = Instant::now();
    while from.elapsed() < ANSWER_WATCH {
        tick(s);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[allow(clippy::too_many_lines)] // one user's path, from attach to the status row
fn e7e_hls_attaches_in_a_cabal_project_and_reports_a_type_error() {
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
    record_server_notifications(&s);
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

    // Setup's progress ends before the edit. An edit typed while the server
    // sets up can leave it holding the right document and publishing nothing,
    // however many keys follow: #291's trace, and #480 is the user's case.
    assert!(
        wait(&mut s, SETUP_BOUND_SECS, |s| {
            note(s, &mut trace);
            setup_ended(s) && lsp_segment(s).as_deref() == Some("LSP:ready")
        }),
        "setup's progress ends within {SETUP_BOUND_SECS} s; trace {trace:?}"
    );

    // A type error at the end of the file, as one key. Typed key by key it
    // reaches the server as a burst of versions, each word's completion request
    // flushing the edit so far, and the row does not cover what a server does
    // with a burst: its silence is #480's and an older version's list #479's.
    let len: i64 = eval(&s, "return pmacs.window.buffer():len()");
    exec(&s, &format!("pmacs.editor.goto_byte({len})"));
    yank(&mut s, "oops :: Int\noops = True");
    let text = buffer_text(&s);
    assert!(
        text.ends_with("oops :: Int\noops = True"),
        "the error went in: {text:?}"
    );
    let version: i64 = eval(&s, "return pmacs.lsp.active_attachment().version");

    // The server's answer to that text is a publish at that version. One
    // without the error fails here: no key is typed to bring it back, and
    // nothing is sent again.
    let at_version = |s: &EditorState| -> Vec<bool> {
        publishes(s)
            .iter()
            .filter(|p| p.version == version)
            .map(|p| p.holds_error)
            .collect()
    };
    let published = wait(&mut s, PUBLISH_BOUND_SECS, |s| {
        note(s, &mut trace);
        !at_version(s).is_empty()
    });
    assert!(
        published,
        "HLS never published at the buffer's version {version} within \
         {PUBLISH_BOUND_SECS} s; publishes [{}]; trace {trace:?}",
        shown(&publishes(&s))
    );
    read_on(&mut s);
    let answers = at_version(&s);
    assert!(
        answers.iter().all(|holds| *holds),
        "HLS published at the buffer's version {version} without the error{}; \
         publishes [{}]; trace {trace:?}",
        if answers[0] {
            ", after one that held it"
        } else {
            ""
        },
        shown(&publishes(&s))
    );

    // The store has it, against the text that publish was computed for.
    let mut seen = Vec::new();
    let stored = wait(&mut s, STORE_BOUND_SECS, |s| {
        note(s, &mut trace);
        seen = diagnostics(s);
        seen.iter()
            .any(|d| d.0 == "error" && d.5.contains("Couldn't match"))
    });
    assert!(
        stored,
        "the store holds no error {STORE_BOUND_SECS} s after the publish at version \
         {version} that held it; store {seen:?}; publishes [{}]; trace {trace:?}",
        shown(&publishes(&s))
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
    // The component loaded: the row's error is the only one, so no
    // cradle error (a bare-`ghc` fallback, a GHC mismatch) sits on line 0.
    let errors: Vec<_> = seen.iter().filter(|d| d.0 == "error").collect();
    assert_eq!(errors, vec![&err], "the row's error is the only error");

    // Placed on the text it names: it covers `True`.
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
    let mut status = String::new();
    let named = wait(&mut s, STORE_BOUND_SECS, |s| {
        status = status_row(s);
        status.contains("error:") && status.contains("Couldn't match")
    });
    assert!(
        named,
        "the status row names the error at point: {status:?}; store {:?}; publishes [{}]",
        diagnostics(&s),
        shown(&publishes(&s))
    );
    assert!(
        wait(&mut s, 60, |s| lsp_segment(s).as_deref()
            == Some("LSP:ready")),
        "the modeline returns to LSP:ready"
    );
}
