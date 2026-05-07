// tests/m8_1c_acceptance.rs --- T M8.1c install_local acceptance.

//! Acceptance tests for `pmacs.packages.install_local(path)`.
//!
//! `install_local` is the dev-loop counterpart to
//! `pmacs.packages.install`: it symlinks a working tree into the
//! install root rather than fetching + extracting an archive.
//! Edits to the source survive without re-running the install path,
//! and `pmacs.packages.reload(name)` (M8.1d) picks them up without
//! restarting the editor.
//!
//! Tests pin the contracts the package author guide will document:
//!
//! - The install path is a symlink, not a real directory.
//! - The roster entry's `pin` reports `kind = "local"` and
//!   `value = <canonical source path>`.
//! - `require("<pkg>")` resolves against the symlinked source, so
//!   the package's `init.lua` runs.
//! - Calling `install_local` twice for the same name swaps the
//!   symlink (and invalidates `package.loaded` so a re-`require`
//!   against the new target works).
//! - `install_local` refuses to overwrite a real (fetched) install
//!   at the same name.
//! - No lockfile entry is written.

use std::path::PathBuf;

use pmacs::lua::LuaHost;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

/// Build a working-tree directory containing a valid pmacs.toml +
/// init.lua. Returns the directory's tempdir guard and the path.
/// The init.lua returns a small module table so a require can
/// observe it.
fn make_local_pkg(name: &str, version_marker: &str) -> (TempDir, PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().to_path_buf();
    let manifest = format!(
        "name = \"{name}\"\n\
         version = \"1.0.0\"\n\
         summary = \"M8.1c install_local fixture: {name}\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n"
    );
    std::fs::write(dir.join("pmacs.toml"), manifest).expect("write manifest");
    std::fs::write(
        dir.join("init.lua"),
        format!("return {{ name = '{name}', marker = '{version_marker}' }}\n"),
    )
    .expect("write entry");
    (td, dir)
}

fn host_with_overrides() -> (LuaHost, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    (host, cache, user_root)
}

// ---------------------------------------------------------------------------
// T M8.1c --- install_local creates a symlink and the package is requireable
// ---------------------------------------------------------------------------

#[test]
fn install_local_creates_a_symlink_and_require_works() {
    let (_pkg_td, pkg_path) = make_local_pkg("local-alpha", "v1");
    let pkg_str = pkg_path.display().to_string();

    let (mut host, _c, user_root) = host_with_overrides();

    let script = format!(
        r#"
        local pkg = pmacs.packages.install_local("{pkg_str}")
        assert(pkg.name == "local-alpha", "pkg.name should be 'local-alpha', got " .. tostring(pkg.name))
        assert(pkg.pin.kind == "local",
            "pin.kind should be 'local', got " .. tostring(pkg.pin.kind))

        -- Require the package; chunk runs from the symlinked tree.
        local m = require("local-alpha")
        assert(m.name == "local-alpha")
        assert(m.marker == "v1")
        return "ok"
    "#
    );
    host.eval(Some("install_local"), &script)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));

    // The install path must be a symlink (not a real dir).
    let install_path = user_root.path().join("local-alpha");
    let meta = std::fs::symlink_metadata(&install_path).expect("symlink_metadata");
    assert!(
        meta.file_type().is_symlink(),
        "install path must be a symlink, got file_type {:?}",
        meta.file_type()
    );
    let target = std::fs::read_link(&install_path).expect("read_link");
    assert_eq!(
        target,
        std::fs::canonicalize(&pkg_path).unwrap(),
        "symlink target must be the canonicalized source path"
    );
}

// ---------------------------------------------------------------------------
// T M8.1c --- install_local does not write a lockfile entry
// ---------------------------------------------------------------------------

#[test]
fn install_local_skips_lockfile_entry() {
    let (_pkg_td, pkg_path) = make_local_pkg("local-no-lock", "v1");
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, user_root) = host_with_overrides();
    let script = format!(r#"pmacs.packages.install_local("{pkg_str}")"#);
    host.eval(Some("install"), &script)
        .unwrap_or_else(|e| panic!("install_local failed: {e}"));

    // No `pmacs.lock` file should exist after an install_local-only run.
    let lock_path = user_root.path().join("pmacs.lock");
    assert!(
        !lock_path.exists(),
        "install_local must not write a lockfile; found one at {lock_path:?}"
    );
}

// ---------------------------------------------------------------------------
// T M8.1c --- install_local replaces an existing symlink (different source)
// ---------------------------------------------------------------------------

#[test]
fn install_local_replaces_existing_symlink_to_a_different_source() {
    let (_a_td, a_path) = make_local_pkg("local-swap", "from-a");
    let (_b_td, b_path) = make_local_pkg("local-swap", "from-b");
    let a_str = a_path.display().to_string();
    let b_str = b_path.display().to_string();

    let (mut host, _c, user_root) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install_local("{a_str}")
        local before = require("local-swap")
        assert(before.marker == "from-a", "first install should yield 'from-a'")

        pmacs.packages.install_local("{b_str}")
        -- install_local invalidates package.loaded so re-require
        -- picks up the new symlink target's chunk.
        local after = require("local-swap")
        assert(after.marker == "from-b",
            "after second install_local, require should return 'from-b', got " .. tostring(after.marker))
    "#
    );
    host.eval(Some("swap"), &script)
        .unwrap_or_else(|e| panic!("install_local swap failed: {e}"));

    let target =
        std::fs::read_link(user_root.path().join("local-swap")).expect("read_link after swap");
    assert_eq!(
        target,
        std::fs::canonicalize(&b_path).unwrap(),
        "symlink must point at the second install_local source after swap"
    );
}

// ---------------------------------------------------------------------------
// T M8.1c --- install_local refuses a path with no pmacs.toml
// ---------------------------------------------------------------------------

#[test]
fn install_local_on_path_without_manifest_fails_with_clear_error() {
    let bad = tempfile::tempdir().expect("tempdir");
    // Just a directory with no manifest.
    std::fs::write(bad.path().join("readme.md"), b"hi").expect("write");
    let bad_str = bad.path().display().to_string();

    let (mut host, _c, _u) = host_with_overrides();
    let script = format!(r#"pmacs.packages.install_local("{bad_str}")"#);
    let err = host
        .eval(Some("missing"), &script)
        .expect_err("must fail without manifest");
    let msg = err.to_string();
    assert!(
        msg.contains("pmacs.toml"),
        "error must mention pmacs.toml; got {msg}"
    );
    assert!(
        msg.contains(&bad_str) || msg.contains("install_local"),
        "error must name the path or the call; got {msg}"
    );
}

// ---------------------------------------------------------------------------
// T M8.1c --- install_local refuses to overwrite a real install
// ---------------------------------------------------------------------------

#[test]
fn install_local_refuses_to_overwrite_a_real_install_dir() {
    let (_pkg_td, pkg_path) = make_local_pkg("local-collide", "v1");
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, user_root) = host_with_overrides();

    // Plant a real (non-symlink) directory at the install path that
    // install_local would otherwise occupy. install_local must
    // refuse rather than blow it away --- a fetched install with
    // user-modified files would otherwise be silently lost.
    let collide_path = user_root.path().join("local-collide");
    std::fs::create_dir_all(&collide_path).expect("plant real dir");
    std::fs::write(collide_path.join("important.lua"), b"-- do not delete\n")
        .expect("write important file");

    let script = format!(r#"pmacs.packages.install_local("{pkg_str}")"#);
    let err = host
        .eval(Some("collide"), &script)
        .expect_err("must refuse real-install collision");
    let msg = err.to_string();
    assert!(
        msg.contains("real install") || msg.contains("not a symlink"),
        "error must explain why install_local refused; got {msg}"
    );
    // The original directory survives.
    assert!(
        collide_path.join("important.lua").exists(),
        "install_local must not have touched the real install"
    );
}

// ---------------------------------------------------------------------------
// T M8.1c --- LockfilePin::from_install_pin returns None for Local
// ---------------------------------------------------------------------------

#[test]
fn lockfile_pin_from_install_pin_returns_none_for_local() {
    use pmacs::packages::{InstallPin, LockfilePin};
    let pin = InstallPin::Local {
        source_path: PathBuf::from("/tmp/wherever"),
    };
    assert!(
        LockfilePin::from_install_pin(&pin).is_none(),
        "Local pins must not produce a LockfilePin"
    );
}
