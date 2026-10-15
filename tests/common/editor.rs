// tests/common/editor.rs --- the in-process editor's Lua driver, shared.

//! The two helpers an in-process suite drives its editor with: `exec`
//! runs a Lua chunk against the editor's own host, `eval` runs one and
//! returns what it produced. Until E8e every integration-test binary
//! re-declared them; this is the body 67 suites carried byte for byte
//! at `587de75` (64 at the audit's `2440089`), unchanged here but for
//! `pub`. The copies that differ --- `to_owned` for `to_string`, an
//! `expect` naming the failing chunk, an `eval` fixed to `String` ---
//! stay where they are until a later batch has compared each class.
//!
//! Included per file rather than through `mod common;`, the way `iso`
//! is, so a suite that spawns no daemon or PTY compiles neither:
//!
//! ```ignore
//! #[path = "common/editor.rs"]
//! mod editor;
//! use editor::{eval, exec};
//! ```
//!
//! Not declared in `common/mod.rs`: ten suites pull both `mod common;`
//! and this include, and `clippy::duplicate_mod` refuses a file loaded
//! as a module twice, so the include is the one route.

#![allow(dead_code)] // not every including suite uses both

use pmacs::editor::EditorState;

pub fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

pub fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}
