// tests/m7_7_acceptance.rs --- Acceptance tests for T M7.7 (loader / require).

//! End-to-end acceptance for T M7.7 (loader and `require` integration).
//! Spec acceptance bullets (`pmacs-tasks.tex:3300-3311`):
//!
//! 1. `require("pmacs-magit")` (or whatever a package's entry is)
//!    loads its entry module successfully.
//! 2. Attempting to `require("pmacs-magit.internal")` (non-exported)
//!    fails with a clear message naming the package and the missing
//!    export.
//! 3. Lua tests verify the environment-table boundary: package A
//!    cannot accidentally pollute package B's globals.
//! 4. `describe-package` (here: `pmacs.packages.describe`) reports a
//!    package's metadata, version, and exports list.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use pmacs::lua::LuaHost;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture: bare repo with custom layout
// ---------------------------------------------------------------------------

/// Build a bare repo with a manifest that declares the given exports
/// list, plus a set of (relative-path → file-body) extra files in the
/// package tree. The package always has an `init.lua` entry returning
/// `{ name = name, version = "1.0.0", ... }` so the bullet-1 path is
/// covered without bespoke fixtures.
fn make_pkg_with_exports(
    name: &str,
    exports: &[&str],
    extra_files: &[(&str, &str)],
) -> (TempDir, PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let work = td.path().join("work");
    let bare = td.path().join("upstream.git");

    run_git(&[
        OsStr::new("init"),
        OsStr::new("--initial-branch=main"),
        work.as_os_str(),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("config"),
        OsStr::new("user.email"),
        OsStr::new("test@example.com"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("config"),
        OsStr::new("user.name"),
        OsStr::new("Tester"),
    ]);

    let exports_lua = exports
        .iter()
        .map(|e| format!("\"{e}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = format!(
        "name = \"{name}\"\n\
         version = \"1.0.0\"\n\
         summary = \"acceptance fixture\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [{exports_lua}]\n",
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write manifest");
    std::fs::write(
        work.join("init.lua"),
        format!("return {{ name = '{name}', version = '1.0.0' }}\n"),
    )
    .expect("write entry");

    for (rel, body) in extra_files {
        let p = work.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir extra");
        }
        std::fs::write(&p, body).expect("write extra");
    }

    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("add"),
        OsStr::new("."),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("commit"),
        OsStr::new("-m"),
        OsStr::new("v1.0.0"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("tag"),
        OsStr::new("v1.0.0"),
    ]);
    run_git(&[
        OsStr::new("clone"),
        OsStr::new("--bare"),
        work.as_os_str(),
        bare.as_os_str(),
    ]);
    (td, bare)
}

fn run_git(args: &[&OsStr]) {
    let mut cmd = Command::new("git");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("LC_ALL", "C");
    let out = cmd.output().expect("git spawn");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

fn file_url(p: &Path) -> String {
    format!("file://{}", p.display())
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
// Bullet 1: require entry module
// ---------------------------------------------------------------------------

#[test]
fn require_loads_entry_module_for_installed_package() {
    let (_td, bare) = make_pkg_with_exports("samplepkg", &["samplepkg"], &[]);
    let url = file_url(&bare);
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{url}", version = "^1.0.0" }}
        local mod = require("samplepkg")
        assert(mod.name == "samplepkg", "wrong name: " .. tostring(mod.name))
        assert(mod.version == "1.0.0", "wrong version: " .. tostring(mod.version))
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("require entry failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}

// ---------------------------------------------------------------------------
// Bullet 2: non-exported submodule require fails clearly
// ---------------------------------------------------------------------------

#[test]
fn require_non_exported_submodule_fails_with_named_error() {
    // The package ships an `internal.lua` file but does not list it in
    // `exports`. `require("samplepkg.internal")` must fail with a
    // message naming the package and the missing export.
    let (_td, bare) = make_pkg_with_exports(
        "samplepkg",
        &["samplepkg.public"],
        &[
            ("public.lua", "return { kind = 'public' }\n"),
            ("internal.lua", "return { kind = 'internal' }\n"),
        ],
    );
    let url = file_url(&bare);
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{url}", version = "^1.0.0" }}
        local ok, err = pcall(require, "samplepkg.internal")
        assert(not ok, "require of unexported submodule unexpectedly succeeded")
        local msg = tostring(err)
        assert(string.find(msg, "samplepkg", 1, true), "error must name package: " .. msg)
        assert(string.find(msg, "internal", 1, true), "error must name missing export: " .. msg)
        assert(string.find(msg, "samplepkg.public", 1, true),
            "error must list available exports: " .. msg)
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("non-exported require check failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}

#[test]
fn require_exported_submodule_loads_module() {
    let (_td, bare) = make_pkg_with_exports(
        "samplepkg",
        &["samplepkg.public"],
        &[("public.lua", "return { kind = 'public' }\n")],
    );
    let url = file_url(&bare);
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{url}", version = "^1.0.0" }}
        local mod = require("samplepkg.public")
        assert(mod.kind == "public", "wrong kind: " .. tostring(mod.kind))
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("exported submodule require failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}

// ---------------------------------------------------------------------------
// Bullet 3: per-package environment isolation
// ---------------------------------------------------------------------------

#[test]
fn package_global_writes_stay_local_to_package_env() {
    // Two packages, each writes a same-name "global" in their entry
    // chunks. Neither write should touch the real `_G`, and each
    // package's entry table must reflect its own write only.
    //
    // The fixture entries are crafted to:
    //   - Set a global `MARKER` to a per-package string.
    //   - Read `MARKER` back and return both `MARKER` and a
    //     stdlib-derived value (`_G.tostring(...)`) — the read goes
    //     through __index on the env's metatable, so stdlib stays
    //     reachable even though the package has its own `_ENV`.
    //   - Return the package's own `_PACKAGE.name` for introspection.
    let (_td_a, bare_a) = make_pkg_with_exports("pkg-a", &["pkg-a"], &[]);
    let (_td_b, bare_b) = make_pkg_with_exports("pkg-b", &["pkg-b"], &[]);
    // Override entry bodies post-fixture by re-cloning, mutating, and
    // re-tagging would be invasive. Simpler: ship the test's expectations
    // through a single-package fixture that we instrument inline.
    let _ = (bare_a, bare_b); // (unused — using inline fixture below)

    // Two packages with custom entry bodies that touch globals.
    let (_td_a, bare_a) = make_pkg_with_custom_entry(
        "pkg-a",
        "MARKER = 'from-a'\nreturn { marker = MARKER, pkg = _PACKAGE.name }\n",
    );
    let (_td_b, bare_b) = make_pkg_with_custom_entry(
        "pkg-b",
        "MARKER = 'from-b'\nreturn { marker = MARKER, pkg = _PACKAGE.name }\n",
    );
    let url_a = file_url(&bare_a);
    let url_b = file_url(&bare_b);
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{url_a}", version = "^1.0.0" }}
        pmacs.packages.install {{ "git:{url_b}", version = "^1.0.0" }}

        local a = require("pkg-a")
        local b = require("pkg-b")

        assert(a.marker == "from-a", "pkg-a saw wrong marker: " .. tostring(a.marker))
        assert(b.marker == "from-b", "pkg-b saw wrong marker: " .. tostring(b.marker))
        assert(a.pkg == "pkg-a", "pkg-a _PACKAGE.name wrong: " .. tostring(a.pkg))
        assert(b.pkg == "pkg-b", "pkg-b _PACKAGE.name wrong: " .. tostring(b.pkg))

        -- The real _G must not have been polluted by either package.
        assert(MARKER == nil, "_G.MARKER should be nil after package loads, got: "
            .. tostring(MARKER))
        assert(rawget(_G, "MARKER") == nil,
            "rawget(_G, 'MARKER') should be nil, got: " .. tostring(rawget(_G, "MARKER")))
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("env isolation failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}

/// Build a bare repo with a custom entry body. Variant of
/// `make_pkg_with_exports` that lets tests inject globals-touching
/// chunks into the package's `init.lua`.
fn make_pkg_with_custom_entry(name: &str, init_body: &str) -> (TempDir, PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let work = td.path().join("work");
    let bare = td.path().join("upstream.git");

    run_git(&[
        OsStr::new("init"),
        OsStr::new("--initial-branch=main"),
        work.as_os_str(),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("config"),
        OsStr::new("user.email"),
        OsStr::new("test@example.com"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("config"),
        OsStr::new("user.name"),
        OsStr::new("Tester"),
    ]);

    let manifest = format!(
        "name = \"{name}\"\n\
         version = \"1.0.0\"\n\
         summary = \"acceptance fixture\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n",
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write manifest");
    std::fs::write(work.join("init.lua"), init_body).expect("write entry");

    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("add"),
        OsStr::new("."),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("commit"),
        OsStr::new("-m"),
        OsStr::new("v1.0.0"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("tag"),
        OsStr::new("v1.0.0"),
    ]);
    run_git(&[
        OsStr::new("clone"),
        OsStr::new("--bare"),
        work.as_os_str(),
        bare.as_os_str(),
    ]);
    (td, bare)
}

#[test]
fn package_can_read_stdlib_through_env_metatable() {
    // The per-package env's __index points at _G, so packages can
    // call stdlib functions transparently. A package that uses
    // `string.format`, `tostring`, and `pmacs.<x>` (if available)
    // through its env should work without explicit imports.
    let (_td, bare) = make_pkg_with_custom_entry(
        "stdlib-user",
        "local out = string.format('rev=%s', tostring(42))\n\
         return { out = out }\n",
    );
    let url = file_url(&bare);
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{url}", version = "^1.0.0" }}
        local mod = require("stdlib-user")
        assert(mod.out == "rev=42", "stdlib not reachable: " .. tostring(mod.out))
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("stdlib via env failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}

// ---------------------------------------------------------------------------
// Bullet 4: describe-package reports metadata
// ---------------------------------------------------------------------------

#[test]
fn describe_returns_full_metadata_for_installed_package() {
    let (_td, bare) = make_pkg_with_exports(
        "samplepkg",
        &["samplepkg", "samplepkg.public"],
        &[("public.lua", "return {}\n")],
    );
    let url = file_url(&bare);
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{url}", version = "^1.0.0" }}
        local d = pmacs.packages.describe("samplepkg")
        assert(d ~= nil, "describe returned nil")
        assert(d.name == "samplepkg", "wrong name: " .. tostring(d.name))
        assert(d.version == "1.0.0", "wrong version: " .. tostring(d.version))
        assert(d.summary == "acceptance fixture", "wrong summary: " .. tostring(d.summary))
        assert(d.pmacs_required == ">=0.1.0" or d.pmacs_required == ">= 0.1.0",
            "pmacs_required wrong: " .. tostring(d.pmacs_required))
        assert(type(d.exports) == "table", "exports must be a table")
        assert(#d.exports == 2, "expected 2 exports, got " .. tostring(#d.exports))
        -- Exports order mirrors the manifest's declaration order.
        assert(d.exports[1] == "samplepkg" and d.exports[2] == "samplepkg.public",
            "unexpected exports: " .. table.concat(d.exports, ","))
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("describe failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}

#[test]
fn describe_returns_nil_for_unknown_package() {
    let (mut host, _c, _u) = host_with_overrides();
    host.eval(
        Some("test"),
        r#"
        local d = pmacs.packages.describe("not-installed")
        assert(d == nil, "expected nil, got " .. type(d))
    "#,
    )
    .unwrap_or_else(|e| panic!("describe-nil failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}

// ---------------------------------------------------------------------------
// Searcher precedence: pmacs roster wins over package.path
// ---------------------------------------------------------------------------

#[test]
fn pmacs_searcher_runs_before_package_path() {
    // If a non-installed name is on package.path AND a similarly named
    // pmacs package is installed, the searcher must route through the
    // installed package (not the path) — exports gating depends on it.
    //
    // We construct: a package "shadow" installed via pmacs.packages.install,
    // PLUS a regular Lua file at path/to/shadow.lua placed on package.path.
    // require("shadow") must return the package's entry table (with
    // marker = "from-pmacs"), not the path file's table.
    let (_td, bare) = make_pkg_with_custom_entry("shadow", "return { marker = 'from-pmacs' }\n");
    let url = file_url(&bare);

    // Stage a path-side shadow.lua.
    let path_dir = tempfile::tempdir().expect("path dir");
    std::fs::write(
        path_dir.path().join("shadow.lua"),
        "return { marker = 'from-path' }\n",
    )
    .expect("write path shadow");
    let path_dir_str = path_dir.path().display().to_string();

    let (mut host, _c, _u) = host_with_overrides();
    let script = format!(
        r#"
        package.path = "{path_dir_str}/?.lua;" .. package.path
        pmacs.packages.install {{ "git:{url}", version = "^1.0.0" }}
        local mod = require("shadow")
        assert(mod.marker == "from-pmacs",
            "pmacs roster should win over package.path; got " .. tostring(mod.marker))
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("searcher precedence: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}
