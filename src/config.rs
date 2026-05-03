// config.rs --- User configuration loader (T M2.10).

//! Loads `~/.config/pmacs/init.lua` on startup.
//!
//! The contract has three pieces:
//!
//! 1. **Missing config is not an error.** If neither
//!    `$XDG_CONFIG_HOME/pmacs/init.lua` nor `$HOME/.config/pmacs/init.lua`
//!    exists, the editor starts as if no config had been requested.
//! 2. **Broken config does not prevent startup.** If the file exists
//!    but the chunk fails to parse or raises at runtime, the error is
//!    captured (visible in the `*errors*` buffer and in the status
//!    line) but the editor still launches with the builtin defaults.
//! 3. **`require` resolves against the config dir** so a user's
//!    `init.lua` can split across multiple files. We do not ship a
//!    package manager (per spec); resolution is the conventional Lua
//!    `package.path` extension.
//!
//! # Testability
//!
//! Production callers use the no-arg [`load_user_config`], which reads
//! `XDG_CONFIG_HOME`/`HOME` from the live environment. Tests --- which
//! must not mutate the process environment, both because pmacs forbids
//! `unsafe` (and `set_var` is `unsafe` in edition 2024) and because
//! parallel test threads would race --- pass an explicit directory via
//! [`load_user_config_at`].

use std::path::{Path, PathBuf};

use mlua::Lua;

use crate::lua::LuaHost;

/// File name of the user's main config chunk.
pub const INIT_FILE: &str = "init.lua";

/// Subdirectory under the config root that pmacs owns.
pub const CONFIG_SUBDIR: &str = "pmacs";

/// Resolve the user's config directory from the live environment.
///
/// `$XDG_CONFIG_HOME/pmacs` if set and non-empty, else
/// `$HOME/.config/pmacs`. Returns `None` if neither environment
/// variable is set --- in that case there is no place for pmacs to
/// look, and config loading is silently skipped.
#[must_use]
pub fn user_config_dir() -> Option<PathBuf> {
    resolve_config_dir(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Pure resolution rule, factored out so tests can drive both arms
/// without touching the process environment.
#[must_use]
pub fn resolve_config_dir(
    xdg: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(xdg) = xdg {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join(CONFIG_SUBDIR));
        }
    }
    let home = home?;
    Some(PathBuf::from(home).join(".config").join(CONFIG_SUBDIR))
}

/// Path to the user's `init.lua`, if a config dir is resolvable.
#[must_use]
pub fn user_config_path() -> Option<PathBuf> {
    user_config_dir().map(|d| d.join(INIT_FILE))
}

/// Append the user's config directory to Lua's `package.path` so
/// `require("name")` finds `dir/name.lua` and `dir/name/init.lua`.
///
/// # Errors
///
/// Returns the underlying [`mlua::Error`] if the package table is
/// missing or the assignment fails. Both indicate a corrupt Lua state.
pub fn install_package_path(lua: &Lua, dir: &Path) -> mlua::Result<()> {
    let dir_str = dir.display().to_string();
    let pkg: mlua::Table = lua.globals().get("package")?;
    let existing: String = pkg.get("path").unwrap_or_default();
    let prepended = format!("{dir_str}/?.lua;{dir_str}/?/init.lua;{existing}");
    pkg.set("path", prepended)?;
    Ok(())
}

/// Load and evaluate the user's `init.lua` from the live environment.
///
/// Equivalent to [`load_user_config_at`] applied to the directory
/// returned by [`user_config_dir`]. A no-op if no config directory is
/// resolvable.
pub fn load_user_config(host: &mut LuaHost) {
    if let Some(dir) = user_config_dir() {
        load_user_config_at(host, &dir);
    }
}

/// Load and evaluate `dir/init.lua` if it exists, after registering
/// `dir` on the Lua `package.path`.
///
/// All failures (file missing, read I/O failure, Lua parse or runtime
/// error) are non-fatal. Lua errors flow through [`LuaHost::eval`]
/// into the host's error log and the `*errors*` buffer; I/O errors
/// are silently skipped --- the editor still starts.
pub fn load_user_config_at(host: &mut LuaHost, dir: &Path) {
    // Even if init.lua is absent, register the directory on
    // package.path so a user who later writes `require` chunks at
    // runtime can resolve them against their config dir without
    // having to reset the path.
    let _ = install_package_path(host.lua(), dir);

    let init = dir.join(INIT_FILE);
    let source = match std::fs::read_to_string(&init) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            // Other I/O errors (permission denied, unreadable, etc.)
            // are also non-fatal --- a user who's misconfigured their
            // home directory shouldn't lose the editor over it.
            return;
        }
    };
    // The leading `@` follows Lua's debug-info convention for a chunk
    // loaded from a file: stack traces show the path verbatim.
    let label = format!("@{}", init.display());
    let _ = host.eval(Some(&label), &source);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn xdg_config_home_takes_priority() {
        let xdg = OsString::from("/some/xdg");
        let home = OsString::from("/should/not/be/used");
        let dir = resolve_config_dir(Some(&xdg), Some(&home)).unwrap();
        assert_eq!(dir, PathBuf::from("/some/xdg/pmacs"));
    }

    #[test]
    fn empty_xdg_falls_through_to_home() {
        let xdg = OsString::from("");
        let home = OsString::from("/home/test");
        let dir = resolve_config_dir(Some(&xdg), Some(&home)).unwrap();
        assert_eq!(dir, PathBuf::from("/home/test/.config/pmacs"));
    }

    #[test]
    fn missing_xdg_falls_back_to_home_dot_config() {
        let home = OsString::from("/home/test");
        let dir = resolve_config_dir(None, Some(&home)).unwrap();
        assert_eq!(dir, PathBuf::from("/home/test/.config/pmacs"));
    }

    #[test]
    fn neither_set_yields_none() {
        assert!(resolve_config_dir(None, None).is_none());
    }

    #[test]
    fn missing_init_lua_is_a_silent_noop() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut host = LuaHost::new().unwrap();
        load_user_config_at(&mut host, dir.path());
        assert!(host.errors().is_empty(), "no config -> no errors");
    }

    #[test]
    fn broken_init_lua_is_captured_and_does_not_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(INIT_FILE), "this is not valid lua )").unwrap();

        let mut host = LuaHost::new().unwrap();
        load_user_config_at(&mut host, dir.path());

        let last = host.last_error().expect("error captured");
        assert!(last.source.as_deref().unwrap().contains("init.lua"));
        // Errors-buffer is populated.
        let id = host.errors_buffer_id().expect("buffer exists");
        let reg = host.registry().borrow();
        let buf = reg.get(id).unwrap();
        assert!(!buf.is_empty(), "errors buffer should not be empty");
    }

    #[test]
    fn runtime_error_in_init_lua_is_captured_with_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(INIT_FILE), "error('user broke it')").unwrap();

        let mut host = LuaHost::new().unwrap();
        load_user_config_at(&mut host, dir.path());

        let last = host.last_error().expect("error captured");
        assert!(last.message.contains("user broke it"));
        assert!(last.source.as_deref().unwrap().contains("init.lua"));
    }

    #[test]
    fn valid_init_lua_runs_and_can_set_globals() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(INIT_FILE), "USER_FLAG = 42").unwrap();

        let mut host = LuaHost::new().unwrap();
        load_user_config_at(&mut host, dir.path());
        assert!(host.errors().is_empty());
        let v: i64 = host.lua().globals().get("USER_FLAG").unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn package_path_includes_config_dir() {
        let lua = Lua::new();
        install_package_path(&lua, Path::new("/cfg")).unwrap();
        let p: String = lua
            .globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .get("path")
            .unwrap();
        assert!(
            p.starts_with("/cfg/?.lua;/cfg/?/init.lua;"),
            "package.path: {p}"
        );
    }

    #[test]
    fn user_init_can_rebind_a_key_in_under_five_lines() {
        // Acceptance: a user-written config of fewer than five lines can
        // rebind a key. Demonstrate via the editor's own attach_editor
        // path so the keymap is the real default keymap.
        use crate::editor_core::EditorCore;
        use std::cell::RefCell;
        use std::rc::Rc;

        let dir = tempfile::TempDir::new().unwrap();
        // Three lines: blank, comment, two statements via single-line each.
        // Two statements over two lines = 2 lines total. Well under 5.
        std::fs::write(
            dir.path().join(INIT_FILE),
            "pmacs.keymap.unbind { scope = 'global', sequence = 'C-a' }\n\
             pmacs.keymap.bind { scope = 'global', sequence = 'C-a', command = 'editor.cancel' }\n",
        )
        .unwrap();

        let registry: crate::lua_bindings::SharedRegistry =
            Rc::new(RefCell::new(crate::buffer_registry::BufferRegistry::new()));
        let core = Rc::new(RefCell::new(EditorCore::new(registry.clone())));
        let mut host = LuaHost::with_registry(registry).unwrap();
        host.attach_editor(&core).expect("attach builtins");
        load_user_config_at(&mut host, dir.path());
        assert!(host.errors().is_empty(), "errors: {:?}", host.errors());

        // After the override, C-a now resolves to editor.cancel.
        let kms = host.keymaps().borrow();
        let chord = crate::key::parse_sequence("C-a").unwrap();
        match kms.resolve(&chord, None, &[]) {
            crate::keymap_stack::StackResolution::Bound(rb) => {
                assert_eq!(rb.binding.command, "editor.cancel");
            }
            other => panic!("expected Bound, got {other:?}"),
        }
    }
}
