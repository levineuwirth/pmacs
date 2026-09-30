// tests/e7g_review1_serialization_witnesses.rs --- E7g review 1: two
// grammars that ship abort the editor the way YAML's did.

//! tree-sitter keeps an external scanner's state in a 1024-byte buffer
//! (`lexer.debug_buffer`, the end of the `Lexer` at the head of the
//! `TSParser`) and asserts, after the scanner's `serialize` returns, that
//! it wrote no more than that. C7g unshipped tree-sitter-yaml 0.7.2 for
//! overrunning it at 254 levels of nesting. Two grammars that stay do the
//! same, and the fuzz run of record reached neither:
//!
//! - tree-sitter-md 0.5.3's block scanner writes five bytes and then four
//!   per open block with no bound at all, so 255 nested blockquotes or list
//!   items write 1025 bytes (and 50,000 write about 200 KB over the
//!   parser's own pointers) before the assert aborts;
//! - tree-sitter-python 0.25.0 checks `size < 1024` before writing an
//!   indent's two bytes, so with an odd number of open string delimiters
//!   and 511 indentation levels it writes 1025.
//!
//! Each row opens the file through the editor; at `4ad9f3e` each failed by
//! taking the test binary down, which is what it did to the daemon. E7h
//! ships both grammars from in-repo copies with the bound fixed (D36 as
//! amended, `vendor/*/PMACS-VENDOR.md`), and the rows run in the sweep.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;

#[path = "common/iso.rs"]
mod iso;

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_owned()).eval().unwrap()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7g-r1s-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Open `text` as `dir/name` in a fresh editor with no servers, tick until a
/// parse tree settles (or three seconds pass), and return the buffer's
/// language and the settled tree's.
fn open_and_settle(dir: &Path, name: &str, text: &str) -> (Option<String>, Option<String>) {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let file = dir.join(name);
    std::fs::write(&file, text).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    let tree =
        || "local t = pmacs.parse.tree(pmacs.window.buffer()) return t and t:language() or nil";
    let until = Instant::now() + Duration::from_secs(3);
    let mut settled: Option<String> = None;
    while Instant::now() < until && settled.is_none() {
        s.tick_processes();
        s.tick_lsp();
        s.tick_async();
        settled = eval(&s, tree());
        std::thread::sleep(Duration::from_millis(5));
    }
    let language: Option<String> = eval(
        &s,
        "return pmacs.parse.buffer_language(pmacs.window.buffer())",
    );
    (language, settled)
}

fn blockquotes(depth: usize) -> String {
    format!("{}q\n", "> ".repeat(depth))
}

fn list(depth: usize) -> String {
    let mut s = String::new();
    for i in 0..depth {
        s.push_str(&"  ".repeat(i));
        s.push_str("- i\n");
    }
    s
}

/// `depth` nested `if` blocks, one space deeper each, and `tail` at the
/// deepest level.
fn python(depth: usize, tail: &str) -> String {
    let mut s = String::new();
    for i in 0..depth {
        s.push_str(&" ".repeat(i));
        s.push_str("if 1:\n");
    }
    s.push_str(&" ".repeat(depth));
    s.push_str(tail);
    s.push('\n');
    s
}

#[test]
fn e7g_review1_a_markdown_file_quoting_255_deep_does_not_take_the_editor_down() {
    let dir = temp_dir("mdq");
    // Control: 254 levels serialize in 1021 bytes and parse.
    assert_eq!(
        open_and_settle(&dir, "ok.md", &blockquotes(254)),
        (Some("markdown".to_owned()), Some("markdown".to_owned()))
    );
    // 511 bytes on one line: 255 blocks, 1025 bytes, SIGABRT at 4ad9f3e.
    let (language, _) = open_and_settle(&dir, "deep.md", &blockquotes(255));
    assert_eq!(language.as_deref(), Some("markdown"));
}

#[test]
fn e7g_review1_a_markdown_list_nested_255_deep_does_not_take_the_editor_down() {
    let dir = temp_dir("mdl");
    assert_eq!(
        open_and_settle(&dir, "ok.md", &list(254)),
        (Some("markdown".to_owned()), Some("markdown".to_owned()))
    );
    let (language, _) = open_and_settle(&dir, "deep.md", &list(255));
    assert_eq!(language.as_deref(), Some("markdown"));
}

#[test]
fn e7g_review1_a_python_string_511_blocks_deep_does_not_take_the_editor_down() {
    let dir = temp_dir("py");
    // Controls: the same depth with no string open, and a string one level
    // shallower, both fit.
    assert_eq!(
        open_and_settle(&dir, "plain.py", &python(511, "x = 1")),
        (Some("python".to_owned()), Some("python".to_owned()))
    );
    assert_eq!(
        open_and_settle(&dir, "shallower.py", &python(510, "x = \"s\"")),
        (Some("python".to_owned()), Some("python".to_owned()))
    );
    // One open delimiter makes the size odd, so the check passes at 1023
    // and the loop writes 1025: SIGABRT at 4ad9f3e.
    let (language, _) = open_and_settle(&dir, "deep.py", &python(511, "x = \"s\""));
    assert_eq!(language.as_deref(), Some("python"));
}

/// The grammars shipped under D36 as amended, each from `vendor/<crate>`
/// with its serialize bound fixed: the crate, and its patched scanner.
const VENDORED_BOUNDS: &[(&str, &str)] = &[
    ("tree-sitter-md", "tree-sitter-markdown/src/scanner.c"),
    ("tree-sitter-python", "src/scanner.c"),
    ("tree-sitter-yaml", "src/scanner.c"),
];

#[test]
fn e7h_the_bounded_grammars_ship_from_their_vendored_copies() {
    // The rows above abort the test binary if an unpatched crate returns;
    // this one says why. Each crate must resolve to its in-repo copy, not
    // to crates.io: a path package in the lock (no `source`), the
    // `[patch.crates-io]` entry that makes it one, and the copy's scanner
    // still carrying the bound. Dropping a copy is its PMACS-VENDOR.md's
    // procedure: a released fix, fuzzed clean, then this list and the
    // patch entry in the same commit.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let patch = manifest
        .split("\n[patch.crates-io]\n")
        .nth(1)
        .expect("Cargo.toml has a [patch.crates-io] section");
    for (krate, scanner) in VENDORED_BOUNDS {
        let block = lock
            .split("[[package]]")
            .find(|b| b.lines().any(|l| l == format!("name = \"{krate}\"")))
            .unwrap_or_else(|| panic!("Cargo.lock lists {krate}"));
        assert!(
            !block.lines().any(|l| l.starts_with("source = ")),
            "{krate} resolves to crates.io again, whose serialize writes past \
             tree-sitter's 1024-byte buffer and aborts the editor:{block}"
        );
        assert!(
            patch.contains(&format!("{krate} = {{ path = \"vendor/{krate}\" }}")),
            "[patch.crates-io] routes {krate} to vendor/{krate}"
        );
        let copy = root.join("vendor").join(krate);
        assert!(
            copy.join("PMACS-VENDOR.md").is_file(),
            "{krate}'s copy says what changed"
        );
        let source = std::fs::read_to_string(copy.join(scanner)).unwrap();
        assert!(
            source.contains("pmacs (E7h, D36 as amended)"),
            "vendor/{krate}/{scanner} still carries the bound"
        );
    }
}

/// The grammars whose vendored copy carries tree-sitter's conforming
/// `tree_sitter/array.h` in place of the published one: python and yaml
/// since E7h (beside their bounds), bash, haskell and html since E7h's fix
/// round 1, for that alone.
const VENDORED_HEADERS: &[&str] = &[
    "tree-sitter-bash",
    "tree-sitter-haskell",
    "tree-sitter-html",
    "tree-sitter-python",
    "tree-sitter-yaml",
];

#[test]
fn e7h_the_conforming_array_header_ships_from_the_vendored_copies() {
    // The published headers of these five push through an `(Array *)` cast,
    // the aliasing UB GCC 16 compiled into tree-sitter-haskell's abort.
    // `-fno-strict-aliasing` forbids every compiler to exploit it, but only
    // in a build cargo starts at the repository root, where it reads
    // `.cargo/config.toml` (E7h review 1: a `cargo install --git` editor
    // aborted on the two-pragma file in seven of nine runs). A copy's header
    // goes wherever its source does. Each crate must resolve to its copy,
    // not crates.io, and the copy's header must be the conforming one; a
    // bump past the copy's version resolves to crates.io and fails here by
    // name, as a dropped patch entry does.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let patch = manifest
        .split("\n[patch.crates-io]\n")
        .nth(1)
        .expect("Cargo.toml has a [patch.crates-io] section");
    for krate in VENDORED_HEADERS {
        let block = lock
            .split("[[package]]")
            .find(|b| b.lines().any(|l| l == format!("name = \"{krate}\"")))
            .unwrap_or_else(|| panic!("Cargo.lock lists {krate}"));
        assert!(
            !block.lines().any(|l| l.starts_with("source = ")),
            "{krate} resolves to crates.io again, whose `array.h` is the aliasing \
             UB GCC 16 turned into tree-sitter-haskell's abort:{block}"
        );
        assert!(
            patch.contains(&format!("{krate} = {{ path = \"vendor/{krate}\" }}")),
            "[patch.crates-io] routes {krate} to vendor/{krate}"
        );
        let copy = root.join("vendor").join(krate);
        assert!(
            copy.join("PMACS-VENDOR.md").is_file(),
            "{krate}'s copy says what changed"
        );
        let header = std::fs::read_to_string(copy.join("src/tree_sitter/array.h")).unwrap();
        assert!(
            header.starts_with("/* pmacs (E7h.1): tree-sitter-rust 0.24.2's")
                && !header.contains("(Array *)(self)")
                && header.contains("(self)->contents = _array__grow("),
            "vendor/{krate}/src/tree_sitter/array.h is the conforming header"
        );
    }
}
