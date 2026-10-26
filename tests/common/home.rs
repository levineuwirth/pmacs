// tests/common/home.rs --- the HOME one in-process editor's Lua reads.

//! `pin_home` makes `os.getenv("HOME")` answer `home` in one editor's Lua
//! state and nowhere else. `*buffer-list*` writes a path under `$HOME` in
//! `~` form (E10c.5 as the owner amended it, 2026-10-25) and reads HOME
//! there, as find-file's `~` does, so a row can say what its paths look
//! like wherever its temporary directory sits: the gate's lies under the
//! real home, CI's does not. The process's environment is untouched.
//!
//! Included per file, as `iso` is:
//!
//! ```ignore
//! #[path = "common/home.rs"]
//! mod home;
//! use home::pin_home;
//! ```

use pmacs::editor::EditorState;

pub fn pin_home(s: &EditorState, home: &str) {
    s.lua_host
        .lua()
        .load(format!(
            "local getenv = os.getenv\n\
             os.getenv = function(name)\n\
               if name == 'HOME' then return {home:?} end\n\
               return getenv(name)\n\
             end"
        ))
        .exec()
        .unwrap();
}
