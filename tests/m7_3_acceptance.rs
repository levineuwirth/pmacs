// m7_3_acceptance.rs --- Acceptance suite for T M7.3 (`pmacs.packages.*`).

//! End-to-end acceptance tests for T M7.3 (`pmacs.packages.install`,
//! `pmacs.packages.install_project`).
//!
//! The three acceptance bullets from the task definition:
//!
//! 1. **User-config install of a sample package succeeds and the
//!    package's entry module is requireable.** Tested here as
//!    [`user_install_makes_entry_requireable`]. The test stages a
//!    bare git repo containing a `pmacs.toml` + `init.lua`, redirects
//!    the install machinery away from the developer's real
//!    `$XDG_*` paths via [`PackageInstallOverride`], calls
//!    `pmacs.packages.install`, and verifies that
//!    `require("samplepkg")` returns the entry module's table.
//! 2. **Project install for a sample package isolates from
//!    user-config.** Tested as
//!    [`project_install_isolates_from_user_install`]. A user install
//!    and a project install of the same upstream land in different
//!    on-disk roots; both are listed in `pmacs.packages.installed()`.
//! 3. **Both variants are documented at the public Lua API surface
//!    with `EmmyLua`-style annotations.** Verified by the static
//!    [`emmylua_doc_file_exists`] check, which reads
//!    `builtin/api/packages.lua` and confirms it contains `EmmyLua`
//!    annotations for `install` and `install_project`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use pmacs::lua::LuaHost;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Sample-package fixture
// ---------------------------------------------------------------------------

/// Build a sample-package bare repo whose entry is at `entry_path`
/// (relative to the package root) with the supplied Lua body.
/// Returns `(tempdir, bare_path)` --- the tempdir owns both the work
/// tree and the bare clone.
fn make_package_with_entry(name: &str, entry_path: &str, entry_body: &str) -> (TempDir, PathBuf) {
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
         entry = \"{entry_path}\"\n\
         exports = [\"{name}\"]\n"
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write pmacs.toml");
    let entry_full = work.join(entry_path);
    if let Some(parent) = entry_full.parent() {
        std::fs::create_dir_all(parent).expect("mkdir entry parent");
    }
    std::fs::write(&entry_full, entry_body).expect("write entry");

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

/// Build a sample-package bare repo with a `pmacs.toml` + `init.lua`,
/// tagged `v1.0.0`. Returns `(tempdir, bare_path)`.
fn make_sample_package(name: &str) -> (TempDir, PathBuf) {
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
         exports = [\"{name}\"]\n"
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write pmacs.toml");
    std::fs::write(
        work.join("init.lua"),
        format!("return {{ name = '{name}', version = '1.0.0' }}\n"),
    )
    .expect("write init.lua");

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
        String::from_utf8_lossy(&out.stderr)
    );
}

fn file_url(p: &Path) -> String {
    format!("file://{}", p.display())
}

/// Build a [`LuaHost`] with the package-install machinery redirected
/// to per-test tempdirs. Returns the host plus the paths so the test
/// can introspect what landed where.
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
// Acceptance bullet 1: user-config install + entry requireable.
// ---------------------------------------------------------------------------

#[test]
fn user_install_makes_entry_requireable() {
    let (_pkg_td, bare) = make_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        local installed = pmacs.packages.install {{
            "git:{url}",
            version = "^1.0.0",
        }}
        assert(installed.name == "samplepkg",
            "expected name == samplepkg, got " .. tostring(installed.name))
        assert(installed.version == "1.0.0",
            "expected version 1.0.0, got " .. tostring(installed.version))
        assert(installed.scope == "user",
            "expected scope user, got " .. tostring(installed.scope))

        -- The acceptance bullet: the entry module is requireable.
        local mod = require("samplepkg")
        assert(mod.name == "samplepkg",
            "require returned wrong table: " .. tostring(mod.name))
        assert(mod.version == "1.0.0",
            "require returned wrong version: " .. tostring(mod.version))

        -- The roster reflects the install.
        local list = pmacs.packages.installed()
        assert(#list == 1, "expected 1 installed package, got " .. tostring(#list))
        assert(list[1].name == "samplepkg")
    "#,
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("install + require failed: {e}");
    });

    assert!(
        host.errors().is_empty(),
        "errors after install: {:?}",
        host.errors()
    );
}

// ---------------------------------------------------------------------------
// Acceptance bullet 2: project install isolates from user install.
// ---------------------------------------------------------------------------

#[test]
fn project_install_isolates_from_user_install() {
    let (_pkg_td, bare) = make_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, user_root) = host_with_overrides();

    // The project root is its own tempdir, distinct from the user root.
    let project = tempfile::tempdir().expect("project tempdir");
    let project_root = project.path().display().to_string();
    let user_root_str = user_root.path().display().to_string();

    let script = format!(
        r#"
        local user = pmacs.packages.install {{
            "git:{url}",
            version = "^1.0.0",
        }}
        local proj = pmacs.packages.install_project {{
            "git:{url}",
            version = "^1.0.0",
            project_root = "{project_root}",
        }}

        assert(user.scope == "user", "user scope")
        assert(proj.scope == "project", "project scope")
        assert(user.install_path ~= proj.install_path,
            "install paths must differ: " .. user.install_path .. " vs " .. proj.install_path)
        assert(string.find(proj.install_path, "{project_root}", 1, true) ~= nil,
            "project path should be under project_root: " .. proj.install_path)
        assert(string.find(user.install_path, "{user_root_str}", 1, true) ~= nil,
            "user path should be under user_root: " .. user.install_path)

        -- Both records present.
        local list = pmacs.packages.installed()
        assert(#list == 2, "expected 2, got " .. tostring(#list))
    "#,
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("install isolation failed: {e}");
    });

    assert!(
        host.errors().is_empty(),
        "errors after dual install: {:?}",
        host.errors()
    );
}

// ---------------------------------------------------------------------------
// Acceptance bullet 3: EmmyLua annotations exist.
// ---------------------------------------------------------------------------

#[test]
fn emmylua_doc_file_exists() {
    // The acceptance bullet says "documented at the public Lua API
    // surface with `EmmyLua`-style annotations". This test pins that
    // documentation file so it stays in lockstep with the binding.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("builtin/api/packages.lua");
    let s = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("missing EmmyLua doc file at {}: {e}", p.display()));
    assert!(
        s.contains("@param"),
        "EmmyLua doc file must contain @param annotations"
    );
    assert!(s.contains("install"), "doc file must document `install`");
    assert!(
        s.contains("install_project"),
        "doc file must document `install_project`"
    );
}

// ---------------------------------------------------------------------------
// Init-time-only gate.
// ---------------------------------------------------------------------------

#[test]
fn install_after_init_complete_errors_with_workaround() {
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_init_complete();

    let err = host
        .eval(
            Some("test"),
            r#"pmacs.packages.install { "github:user/repo", version = "*" }"#,
        )
        .expect_err("post-init install must error");
    let msg = err.to_string();
    assert!(
        msg.contains("pmacs.packages.install"),
        "error must name the op: {msg}"
    );
    assert!(
        msg.contains("init.lua"),
        "error must name the right phase: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Spec-shape parsing.
// ---------------------------------------------------------------------------

#[test]
fn shorthand_string_form_is_accepted() {
    let (_pkg_td, bare) = make_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        local installed = pmacs.packages.install("git:{url}@^1.0.0")
        assert(installed.name == "samplepkg")
    "#,
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("shorthand form failed: {e}");
    });
}

#[test]
fn install_spec_missing_address_errors() {
    let mut host = LuaHost::new().expect("LuaHost::new");
    let err = host
        .eval(Some("test"), r#"pmacs.packages.install { version = "*" }"#)
        .expect_err("missing address must error");
    let msg = err.to_string();
    assert!(
        msg.contains("address"),
        "error should name the missing field: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Reviewer-flagged item 10: install_project requires explicit project_root.
// ---------------------------------------------------------------------------

#[test]
fn install_project_without_project_root_errors_with_workaround() {
    // Pre-v0.1 the missing field silently fell back to
    // `std::env::current_dir()`. That was a footgun: CWD-at-startup
    // is whatever shell directory the user happened to invoke pmacs
    // from, almost never a meaningful project root. The fallback is
    // gone; the binding now requires an explicit field and the error
    // message names two concrete patterns for filling it in
    // (env-var lookup, init.lua-relative path).
    let mut host = LuaHost::new().expect("LuaHost::new");
    let err = host
        .eval(
            Some("test"),
            r#"pmacs.packages.install_project { "git:does-not-matter", version = "^1.0" }"#,
        )
        .expect_err("missing project_root must error");
    let msg = err.to_string();
    assert!(
        msg.contains("project_root"),
        "error must name the missing field: {msg}"
    );
    assert!(
        msg.contains("install_project"),
        "error must name the op: {msg}"
    );
    // Two concrete patterns the user can apply without hunting for
    // documentation. Both are mentioned in the error text so a CI
    // log line stands on its own.
    assert!(
        msg.contains("PMACS_PROJECT") || msg.contains("os.getenv"),
        "error should hint at env-var pattern: {msg}"
    );
    assert!(
        msg.contains("init.lua"),
        "error should hint at init.lua-relative pattern: {msg}"
    );
}

#[test]
fn install_project_relative_path_resolves_against_init_lua_dir() {
    // A relative `project_root` value resolves against the
    // directory of the loading chunk (the convention for
    // file-loaded `init.lua`). The chunk source label
    // `@<absolute-path>` is the standard hook --- `LuaHost::eval`
    // sets it via `set_name`, and `debug.getinfo("S").source`
    // reads it back.
    //
    // Setup: write a transient `init.lua` under tempdir-A, set
    // `project_root = "subproj"` from inside it (a relative path),
    // and assert the install lands at `tempdir-A/subproj/.pmacs/...`.
    let (_pkg_td, bare) = make_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let init_dir = tempfile::tempdir().expect("init dir");
    let init_path = init_dir.path().join("init.lua");
    let init_label = format!("@{}", init_path.display());

    let script = format!(
        r#"
        local p = pmacs.packages.install_project {{
            "git:{url}",
            version = "^1.0.0",
            project_root = "subproj",
        }}
        return p.install_path
    "#,
    );

    let value = host
        .eval(Some(&init_label), &script)
        .unwrap_or_else(|e| panic!("relative project_root install failed: {e}"));
    let install_path = match value {
        mlua::Value::String(s) => s.to_str().expect("string utf8").to_string(),
        other => panic!("expected install_path string, got {other:?}"),
    };

    let expected_prefix = init_dir.path().join("subproj");
    assert!(
        install_path.starts_with(&expected_prefix.display().to_string()),
        "expected install under {expected_prefix:?}, got {install_path:?}"
    );
}

// ---------------------------------------------------------------------------
// Reviewer-flagged item 7: custom entry paths must be requireable.
// ---------------------------------------------------------------------------
//
// A package whose manifest declares `entry = "main.lua"` (or any
// non-`init.lua` path) cannot be loaded via the standard
// `?.lua;?/init.lua` `package.path` pattern alone --- the path
// search misses the entry file. The custom searcher in
// `lua_bindings::register_package_searcher` closes the gap by
// mapping `require("<basename>")` directly to the manifest's
// declared entry path.

#[test]
fn package_with_main_lua_entry_is_requireable() {
    // Entry file at `main.lua` (not the conventional `init.lua`).
    // The path-based searcher misses it; the custom searcher must
    // route `require("samplepkg")` to `<install>/main.lua`.
    let (_pkg_td, bare) = make_package_with_entry(
        "samplepkg",
        "main.lua",
        "return { name = 'samplepkg', via = 'main.lua' }\n",
    );
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        local installed = pmacs.packages.install {{
            "git:{url}",
            version = "^1.0.0",
        }}
        assert(installed.entry:sub(-#"main.lua") == "main.lua",
            "manifest entry must be main.lua, got " .. installed.entry)
        local mod = require("samplepkg")
        assert(mod.name == "samplepkg",
            "custom searcher should have loaded main.lua, got: " .. tostring(mod.name))
        assert(mod.via == "main.lua",
            "module body must be the contents of main.lua, got via=" .. tostring(mod.via))
    "#
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("require(samplepkg) with entry=main.lua failed: {e}");
    });
}

#[test]
fn package_with_nested_entry_path_is_requireable() {
    // Entry at `lib/foo.lua` --- a package layout where the
    // user-facing module lives a couple of levels deep. The custom
    // searcher must read it from the manifest and resolve
    // `require("samplepkg")` to the nested file.
    let (_pkg_td, bare) = make_package_with_entry(
        "samplepkg",
        "lib/foo.lua",
        "return { name = 'samplepkg', via = 'lib/foo.lua' }\n",
    );
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        local installed = pmacs.packages.install {{
            "git:{url}",
            version = "^1.0.0",
        }}
        assert(installed.entry:sub(-#"lib/foo.lua") == "lib/foo.lua",
            "manifest entry must be lib/foo.lua, got " .. installed.entry)
        local mod = require("samplepkg")
        assert(mod.via == "lib/foo.lua",
            "module body must be the contents of lib/foo.lua, got via=" .. tostring(mod.via))
    "#
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("require(samplepkg) with nested entry failed: {e}");
    });
}

#[test]
fn searcher_misses_for_unknown_name_with_pmacs_specific_message() {
    // No installed package matches `unrelatedpkg`. The custom
    // searcher returns its "no installed pmacs package named ..."
    // string, which Lua appends to the standard require-failure
    // chain. The point: a `require` that fails with an obvious
    // typo produces an error message that names the pmacs-side
    // searcher's contribution to the search, so a user can see
    // why the install they thought they did did not satisfy this
    // require.
    let mut host = LuaHost::new().expect("LuaHost::new");
    let err = host
        .eval(
            Some("test"),
            r#"
            local mod = require("unrelatedpkg")
            return mod
            "#,
        )
        .expect_err("require for unknown name must error");
    let msg = err.to_string();
    assert!(
        msg.contains("unrelatedpkg"),
        "error must echo the require name: {msg}"
    );
    assert!(
        msg.contains("no installed pmacs package"),
        "error must mention the pmacs searcher's contribution: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Reviewer-flagged item 11: branch/commit install pins.
// ---------------------------------------------------------------------------
//
// The fetcher already supports `RefSpec::Branch` and `RefSpec::Commit`;
// item 11 is the Lua-surface plumbing that exposes those resolutions
// to user init.lua. The acceptance shape: a spec table with
// `branch = "..."` or `commit = "..."` (instead of `version = "..."`)
// installs that exact revision. The two are mutually exclusive --- a
// table with both must error.

/// Build a sample-package bare repo with two tagged versions plus a
/// `feature` branch carrying a third commit. Returns
/// `(tempdir, bare_path, feature_branch_commit_sha)`. Every field
/// caller may need to verify a branch/commit pin came out of the
/// install path correctly.
fn make_branched_sample_package(name: &str) -> (TempDir, PathBuf, String) {
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

    // First tagged release on `main`.
    write_manifest(&work, name, "1.0.0");
    std::fs::write(work.join("init.lua"), b"return { from = 'main@v1.0.0' }\n")
        .expect("write init");
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

    // Branch off `main` and commit a different init.lua. The branch
    // remains untagged --- only a branch ref points at it.
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("checkout"),
        OsStr::new("-b"),
        OsStr::new("feature"),
    ]);
    std::fs::write(
        work.join("init.lua"),
        b"return { from = 'feature-branch' }\n",
    )
    .expect("write feature init");
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
        OsStr::new("feature work"),
    ]);
    let feature_sha = git_rev_parse_head(&work);

    // Bare clone after both refs exist.
    run_git(&[
        OsStr::new("clone"),
        OsStr::new("--bare"),
        work.as_os_str(),
        bare.as_os_str(),
    ]);

    (td, bare, feature_sha)
}

fn write_manifest(work: &Path, name: &str, version: &str) {
    let manifest = format!(
        "name = \"{name}\"\n\
         version = \"{version}\"\n\
         summary = \"acceptance fixture\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n"
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write pmacs.toml");
}

fn git_rev_parse_head(work: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(work)
        .arg("rev-parse")
        .arg("HEAD")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .output()
        .expect("git rev-parse spawn");
    assert!(out.status.success(), "git rev-parse failed");
    String::from_utf8(out.stdout)
        .expect("rev-parse stdout utf8")
        .trim()
        .to_string()
}

#[test]
fn install_with_branch_pin_uses_branch_head() {
    let (_pkg_td, bare, _feature_sha) = make_branched_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        local installed = pmacs.packages.install {{
            "git:{url}",
            branch = "feature",
        }}
        assert(installed.pin.kind == "branch",
            "pin.kind must be branch, got " .. tostring(installed.pin.kind))
        assert(installed.pin.value == "feature",
            "pin.value must echo the branch name, got " .. tostring(installed.pin.value))
        assert(installed.tag == "branch:feature",
            "tag descriptor must be branch:feature, got " .. tostring(installed.tag))
        local mod = require("samplepkg")
        assert(mod.from == "feature-branch",
            "module body must come from the feature branch's init.lua, got " .. tostring(mod.from))
    "#
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("branch pin install failed: {e}");
    });
}

#[test]
fn install_with_commit_pin_uses_exact_revision() {
    let (_pkg_td, bare, feature_sha) = make_branched_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        local installed = pmacs.packages.install {{
            "git:{url}",
            commit = "{feature_sha}",
        }}
        assert(installed.pin.kind == "commit",
            "pin.kind must be commit, got " .. tostring(installed.pin.kind))
        assert(installed.pin.value == "{feature_sha}",
            "pin.value must echo the SHA, got " .. tostring(installed.pin.value))
        assert(installed.commit == "{feature_sha}",
            "resolved commit must equal the pinned SHA, got " .. tostring(installed.commit))
        assert(installed.tag:sub(1, 7) == "commit:",
            "tag descriptor must start with commit:, got " .. tostring(installed.tag))
        local mod = require("samplepkg")
        assert(mod.from == "feature-branch",
            "module body must come from the pinned commit, got " .. tostring(mod.from))
    "#
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("commit pin install failed: {e}");
    });
}

#[test]
fn install_with_conflicting_pins_errors_with_field_list() {
    // Specifying more than one pin is ambiguous (which one wins?).
    // The error must name every conflicting field so the user can
    // see which to keep without re-reading the docs.
    let (_pkg_td, bare, _) = make_branched_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{
            "git:{url}",
            version = "^1.0.0",
            branch = "feature",
        }}
    "#
    );
    let err = host
        .eval(Some("test"), &script)
        .expect_err("conflicting pins must error");
    let msg = err.to_string();
    assert!(
        msg.contains("version") && msg.contains("branch"),
        "error must name both conflicting fields: {msg}"
    );
    assert!(
        msg.contains("exactly one"),
        "error must explain the mutual-exclusion rule: {msg}"
    );
}

#[test]
fn install_with_default_version_pin_when_no_pin_field_supplied() {
    // The reviewer's wording: existing default is `version = "*"`.
    // This test pins that contract: a spec table with no
    // version/branch/commit field defaults to "any tag".
    let (_pkg_td, bare) = make_sample_package("samplepkg");
    let url = file_url(&bare);
    let (mut host, _cache, _user_root) = host_with_overrides();

    let script = format!(
        r#"
        local installed = pmacs.packages.install {{ "git:{url}" }}
        assert(installed.pin.kind == "version",
            "default pin must be a version pin, got " .. tostring(installed.pin.kind))
        assert(installed.pin.value == "*",
            "default constraint must be `*`, got " .. tostring(installed.pin.value))
    "#
    );
    host.eval(Some("test"), &script).unwrap_or_else(|e| {
        panic!("default-pin install failed: {e}");
    });
}
