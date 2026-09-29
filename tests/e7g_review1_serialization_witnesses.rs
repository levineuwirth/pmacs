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
//! Each row opens the file through the editor and fails by taking the test
//! binary down, which is what it does to the daemon. They are ignored so the
//! gate describes the reviewed tree; the fix round un-ignores them. Run:
//! `cargo test --test e7g_review1_serialization_witnesses -- --ignored`.

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
#[ignore = "E7g review 1 witness: aborts at 4ad9f3e (tree-sitter-md's unbounded serialize); the fix round un-ignores it"]
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
#[ignore = "E7g review 1 witness: aborts at 4ad9f3e (tree-sitter-md's unbounded serialize); the fix round un-ignores it"]
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
#[ignore = "E7g review 1 witness: aborts at 4ad9f3e (tree-sitter-python's off-by-one serialize); the fix round un-ignores it"]
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
