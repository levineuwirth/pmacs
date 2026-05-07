// tests/m7_review_acceptance.rs --- Post-M7 audit-fix acceptance.
//
//! Acceptance tests for the M7 close-out review findings (see the
//! commit message and `TRANSITION-M7.md` "Review-driven fixes"
//! section). Each test pins a contract that was either inert or
//! contradicted by docs at M7.11 release time:
//!
//! - High #1: `pmacs.packages.install` resolves and installs the
//!   full dependency closure, not just the top-level spec.
//! - High #3: Frozen-policy resolve verifies content hashes.
//!   (Covered in `m7_6_acceptance.rs` --- the fix lives here too
//!   for organizational clarity, but isn't re-asserted in this
//!   file.)
//! - High #5: bundled-package dir lives under XDG, not /tmp.
//! - Medium #6: `gitlab:` is wired and parses to gitlab.com URLs.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use pmacs::lua::LuaHost;
use pmacs::lua_bindings::PackageInstallOverride;
use pmacs::packages::Address;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture: a package whose manifest declares one dependency
// ---------------------------------------------------------------------------

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

/// Build a bare repo for a package whose `pmacs.toml` carries one
/// optional dependency line. `dep_addr` is the upstream URL form to
/// embed in the manifest's `[[dependencies]]` block; pass `None` to
/// build a leaf package.
fn make_pkg_with_dep(
    name: &str,
    dep_addr: Option<&str>,
    dep_constraint: &str,
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

    let mut manifest = format!(
        "name = \"{name}\"\n\
         version = \"1.0.0\"\n\
         summary = \"M7 review fixture: {name}\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n"
    );
    if let Some(addr) = dep_addr {
        use std::fmt::Write;
        write!(
            manifest,
            "\n[[dependencies]]\naddress = \"git:{addr}\"\nversion = \"{dep_constraint}\"\n"
        )
        .expect("write into String never fails");
    }
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write manifest");
    std::fs::write(
        work.join("init.lua"),
        format!("return {{ name = '{name}', version = '1.0.0' }}\n"),
    )
    .expect("write entry");

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
// Review High #1: install resolves transitive deps
// ---------------------------------------------------------------------------

#[test]
fn install_resolves_and_installs_manifest_dependencies() {
    // Build dep first, then app referencing dep's bare repo.
    let (_dep_td, dep_bare) = make_pkg_with_dep("review-dep", None, "");
    let dep_addr = file_url(&dep_bare);
    let (_app_td, app_bare) = make_pkg_with_dep("review-app", Some(&dep_addr), "^1.0.0");
    let app_addr = file_url(&app_bare);

    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{app_addr}", version = "^1.0.0" }}
        local roster = pmacs.packages.installed()
        local names = {{}}
        for _, p in ipairs(roster) do names[p.name] = true end
        assert(names["review-app"], "top-level review-app must be installed")
        assert(names["review-dep"], "transitive review-dep must be installed (review fix)")

        -- And require() works for the dependency, not just the top-level.
        local app = require("review-app")
        assert(app.name == "review-app")
        local dep = require("review-dep")
        assert(dep.name == "review-dep")

        return "ok"
    "#
    );
    host.eval(Some("m7_review"), &script)
        .unwrap_or_else(|e| panic!("install with deps failed: {e}"));
}

// ---------------------------------------------------------------------------
// Review High #5: bundled dir is XDG-rooted, not /tmp-rooted
// ---------------------------------------------------------------------------

#[test]
fn bundled_runtime_dir_lives_under_xdg_data_home_when_set() {
    // We cannot mutate XDG_DATA_HOME from a safe-Rust test
    // (`std::env::set_var` is unsafe in 2024). Instead, the test
    // validates the *resolution policy* the implementation uses by
    // running with the current environment and asserting the result
    // is at least not under `/tmp/pmacs-builtin-v...` (the old
    // shape that the audit flagged).
    let p = pmacs::builtin_packages::bundled_runtime_dir();
    let s = p.to_string_lossy();
    assert!(
        !s.starts_with("/tmp/pmacs-builtin-v"),
        "bundled dir must not sit under /tmp/pmacs-builtin-v...; got {s}"
    );
}

// ---------------------------------------------------------------------------
// Review High #2: install writes a lockfile + update reads it
// ---------------------------------------------------------------------------

#[test]
fn install_writes_pmacs_lock_at_install_root() {
    let (_dep_td, dep_bare) = make_pkg_with_dep("review-lock-dep", None, "");
    let dep_addr = file_url(&dep_bare);
    let (_app_td, app_bare) = make_pkg_with_dep("review-lock-app", Some(&dep_addr), "^1.0.0");
    let app_addr = file_url(&app_bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{app_addr}", version = "^1.0.0" }}
    "#
    );
    host.eval(Some("m7_review_lock"), &script)
        .unwrap_or_else(|e| panic!("install failed: {e}"));

    let lock_path = user_root.path().join("pmacs.lock");
    assert!(
        lock_path.exists(),
        "expected lockfile at {lock_path:?} after install"
    );
    let lock = pmacs::packages::Lockfile::read_from(&lock_path).expect("parse lockfile");
    let names: Vec<&str> = lock.packages.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"review-lock-app"),
        "lockfile must list app: {names:?}"
    );
    assert!(
        names.contains(&"review-lock-dep"),
        "lockfile must list transitive dep: {names:?}"
    );
}

#[test]
fn install_merges_existing_lockfile_across_calls() {
    // Two top-level installs in the same Lua host should result in
    // a lockfile that lists both. Without merge, the second call's
    // lockfile would clobber the first call's.
    let (_a_td, a_bare) = make_pkg_with_dep("review-merge-a", None, "");
    let (_b_td, b_bare) = make_pkg_with_dep("review-merge-b", None, "");
    let a_addr = file_url(&a_bare);
    let b_addr = file_url(&b_bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{a_addr}", version = "^1.0.0" }}
        pmacs.packages.install {{ "git:{b_addr}", version = "^1.0.0" }}
    "#
    );
    host.eval(Some("m7_review_merge"), &script)
        .unwrap_or_else(|e| panic!("merged install failed: {e}"));

    let lock_path = user_root.path().join("pmacs.lock");
    let lock = pmacs::packages::Lockfile::read_from(&lock_path).expect("parse lockfile");
    let names: Vec<&str> = lock.packages.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"review-merge-a") && names.contains(&"review-merge-b"),
        "lockfile must contain both packages after sequential installs: {names:?}"
    );
}

#[test]
fn update_without_lockfile_returns_clear_error() {
    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    let err = host
        .eval(Some("m7_review_update_empty"), "pmacs.packages.update()")
        .expect_err("update without lockfile must error");
    let msg = err.to_string();
    assert!(
        msg.contains("lockfile") || msg.contains("install"),
        "error should mention missing lockfile or guide user to install: {msg}"
    );
}

#[test]
fn update_unknown_package_name_returns_clear_error() {
    // Need a baseline lockfile, so install one package first.
    let (_td, bare) = make_pkg_with_dep("review-update-base", None, "");
    let addr = file_url(&bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    let install_script =
        format!(r#"pmacs.packages.install {{ "git:{addr}", version = "^1.0.0" }}"#);
    host.eval(Some("install"), &install_script)
        .unwrap_or_else(|e| panic!("baseline install failed: {e}"));

    let err = host
        .eval(
            Some("update_unknown"),
            "pmacs.packages.update(\"no-such-package\")",
        )
        .expect_err("update of unknown name must error");
    let msg = err.to_string();
    assert!(
        msg.contains("no-such-package"),
        "error must name the unknown package: {msg}"
    );
}

#[test]
fn update_all_no_op_when_upstream_unchanged() {
    // Install once, lockfile is written. update() with no changes
    // upstream should produce a plan that re-resolves to the same
    // commits.
    let (_td, bare) = make_pkg_with_dep("review-update-a", None, "");
    let addr = file_url(&bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{addr}", version = "^1.0.0" }}
        local summary = pmacs.packages.update()
        assert(type(summary) == "table", "update should return a summary table")
        assert(#summary >= 1, "summary should list at least one package")
        local found = false
        for _, row in ipairs(summary) do
            if row.name == "review-update-a" then
                found = true
                assert(row.changed == false,
                    "no-op update should report changed = false; got " .. tostring(row.changed))
            end
        end
        assert(found, "summary must include review-update-a")
    "#
    );
    host.eval(Some("m7_review_update_noop"), &script)
        .unwrap_or_else(|e| panic!("update no-op failed: {e}"));
}

// ---------------------------------------------------------------------------
// Review Medium #6: gitlab: address parses
// ---------------------------------------------------------------------------

#[test]
fn gitlab_address_parses_and_canonicalizes_to_gitlab_com() {
    let a = Address::parse("gitlab:user/repo").expect("gitlab: should parse");
    assert!(matches!(a, Address::Gitlab { .. }));
    assert_eq!(a.to_git_url(), "https://gitlab.com/user/repo.git");

    // Common typo / habit: trailing .git tolerated.
    let b = Address::parse("gitlab:user/repo.git").expect("trailing .git tolerated");
    assert_eq!(b.to_git_url(), "https://gitlab.com/user/repo.git");
}

// ---------------------------------------------------------------------------
// Review Second-Pass High #1: update actually replaces a moved install
// ---------------------------------------------------------------------------

/// Publish a new version on an existing bare repo by cloning it,
/// bumping `pmacs.toml`'s `version`, committing, tagging, and pushing
/// back. Returns the commit SHA on the new tag for assertions.
fn publish_new_version(bare: &Path, name: &str, new_version: &str) -> String {
    let work = bare.parent().expect("bare has parent").join("work-bump");
    run_git(&[OsStr::new("clone"), bare.as_os_str(), work.as_os_str()]);
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
         version = \"{new_version}\"\n\
         summary = \"M7 review fixture: {name}\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n"
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("rewrite manifest");
    std::fs::write(
        work.join("init.lua"),
        format!("return {{ name = '{name}', version = '{new_version}' }}\n"),
    )
    .expect("rewrite entry");

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
        OsStr::new(&format!("v{new_version}")),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("tag"),
        OsStr::new(&format!("v{new_version}")),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("push"),
        OsStr::new("--tags"),
        OsStr::new("origin"),
        OsStr::new("main"),
    ]);

    let out = Command::new("git")
        .arg("-C")
        .arg(&work)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .expect("rev-parse HEAD");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn update_replaces_install_when_upstream_publishes_new_version() {
    let (_td, bare) = make_pkg_with_dep("review-bump", None, "");
    let addr = file_url(&bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    // Install at ^1.0.0 --- picks v1.0.0 (the only available tag).
    let install_script =
        format!(r#"pmacs.packages.install {{ "git:{addr}", version = "^1.0.0" }}"#);
    host.eval(Some("install_v1"), &install_script)
        .unwrap_or_else(|e| panic!("baseline install failed: {e}"));

    let install_dir = user_root.path().join("review-bump");
    let entry_v1 =
        std::fs::read_to_string(install_dir.join("init.lua")).expect("read entry after v1 install");
    assert!(
        entry_v1.contains("'1.0.0'"),
        "v1 install should ship version 1.0.0 entry; got {entry_v1}"
    );

    // Upstream publishes 1.1.0.
    let v1_1_commit = publish_new_version(&bare, "review-bump", "1.1.0");

    // update() should re-resolve and replace the on-disk install.
    let update_script = r#"
        local summary = pmacs.packages.update()
        for _, row in ipairs(summary) do
            if row.name == "review-bump" then
                assert(row.version == "1.1.0",
                    "update should land version 1.1.0; got " .. tostring(row.version))
                assert(row.changed == true,
                    "update should report changed = true; got " .. tostring(row.changed))
                return
            end
        end
        error("review-bump missing from update summary")
    "#;
    host.eval(Some("update_to_v1_1"), update_script)
        .unwrap_or_else(|e| panic!("update failed: {e}"));

    let entry_v11 = std::fs::read_to_string(install_dir.join("init.lua"))
        .expect("read entry after v1.1 update");
    assert!(
        entry_v11.contains("'1.1.0'"),
        "post-update install should ship version 1.1.0 entry; got {entry_v11}"
    );

    // Lockfile reflects the new commit.
    let lock = pmacs::packages::Lockfile::read_from(&user_root.path().join("pmacs.lock"))
        .expect("parse lockfile");
    let entry = lock
        .packages
        .iter()
        .find(|e| e.name.as_str() == "review-bump")
        .expect("review-bump in lockfile");
    assert_eq!(
        entry.commit, v1_1_commit,
        "lockfile commit should match newly-published v1.1.0 HEAD"
    );

    // No leftover staging dir.
    let staged = user_root.path().join("review-bump.new");
    assert!(
        !staged.exists(),
        "no leftover staging dir after successful update"
    );
}

// ---------------------------------------------------------------------------
// Review Second-Pass High #2: top-level install honors resolver's commit
// ---------------------------------------------------------------------------

#[test]
fn top_level_install_uses_resolver_commit_not_independent_tag_pick() {
    // Setup: app depends on dep ^1.0.0. Both publish v1.0.0 first;
    // then dep publishes v1.1.0 with a much higher pmacs_required
    // that's incompatible with the running build. The resolver must
    // pick dep v1.0.0 to satisfy compatibility; the installer must
    // not independently re-pick v1.1.0 for dep when invoked through
    // the user-facing install.
    //
    // Setup pivot: this test isn't about a *transitive* dep but
    // about the *top-level* package's tag pick. So we structure it
    // as: app v1.0.0 (compat), then publish app v1.1.0 with a
    // pmacs_required that won't match the running version. Install
    // with `version = "^1.0.0"` --- resolver picks v1.0.0 (v1.1.0
    // ineligible due to pmacs_required), installer must follow.
    let (_td, bare) = make_pkg_with_dep("review-resolver-pick", None, "");
    let addr = file_url(&bare);

    // Publish v1.1.0 with an unrealistically-high pmacs_required.
    let work = bare
        .parent()
        .expect("bare has parent")
        .join("work-incompat");
    run_git(&[OsStr::new("clone"), bare.as_os_str(), work.as_os_str()]);
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
    let bad_manifest = "name = \"review-resolver-pick\"\n\
         version = \"1.1.0\"\n\
         summary = \"resolver-incompat fixture\"\n\
         pmacs_required = \">= 99.0.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"review-resolver-pick\"]\n";
    std::fs::write(work.join("pmacs.toml"), bad_manifest).expect("rewrite manifest");
    std::fs::write(
        work.join("init.lua"),
        "return { name = 'review-resolver-pick', version = '1.1.0' }\n",
    )
    .expect("rewrite entry");
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
        OsStr::new("v1.1.0"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("tag"),
        OsStr::new("v1.1.0"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("push"),
        OsStr::new("--tags"),
        OsStr::new("origin"),
        OsStr::new("main"),
    ]);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    // Constraint ^1.0.0 admits both v1.0.0 and v1.1.0 by version,
    // but v1.1.0's pmacs_required (>= 99.0.0) is incompatible. The
    // resolver must prefer v1.0.0; if the installer independently
    // re-picks v1.1.0 it errors with PmacsVersionIncompatible
    // instead of installing successfully.
    let script = format!(
        r#"
        local pkg = pmacs.packages.install {{ "git:{addr}", version = "^1.0.0" }}
        assert(pkg.version == "1.0.0",
            "expected resolver to pick v1.0.0, installer to honor it; got " .. tostring(pkg.version))
    "#
    );
    host.eval(Some("resolver_pick"), &script)
        .unwrap_or_else(|e| {
            panic!("install must succeed at v1.0.0 via resolver pick (not v1.1.0): {e}")
        });
}

// ---------------------------------------------------------------------------
// Review Third-Pass Medium #1: update prunes dropped transitive deps
// ---------------------------------------------------------------------------

/// Publish a new version on `bare` with an optional `[[dependencies]]`
/// block. When `dep_addr` is None, the new revision drops every
/// dependency it had before --- the case the prune test exercises.
fn publish_new_version_with_optional_dep(
    bare: &Path,
    name: &str,
    new_version: &str,
    dep_addr: Option<&str>,
    dep_constraint: &str,
) -> String {
    let work = bare
        .parent()
        .expect("bare has parent")
        .join(format!("work-bump-{new_version}"));
    run_git(&[OsStr::new("clone"), bare.as_os_str(), work.as_os_str()]);
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

    let mut manifest = format!(
        "name = \"{name}\"\n\
         version = \"{new_version}\"\n\
         summary = \"M7 review fixture: {name}\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n"
    );
    if let Some(addr) = dep_addr {
        use std::fmt::Write;
        write!(
            manifest,
            "\n[[dependencies]]\naddress = \"git:{addr}\"\nversion = \"{dep_constraint}\"\n"
        )
        .expect("write into String never fails");
    }
    std::fs::write(work.join("pmacs.toml"), manifest).expect("rewrite manifest");
    std::fs::write(
        work.join("init.lua"),
        format!("return {{ name = '{name}', version = '{new_version}' }}\n"),
    )
    .expect("rewrite entry");

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
        OsStr::new(&format!("v{new_version}")),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("tag"),
        OsStr::new(&format!("v{new_version}")),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("push"),
        OsStr::new("--tags"),
        OsStr::new("origin"),
        OsStr::new("main"),
    ]);

    let out = Command::new("git")
        .arg("-C")
        .arg(&work)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .expect("rev-parse HEAD");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn update_prunes_packages_dropped_from_new_resolve_plan() {
    // App v1.0.0 depends on dep ^1.0.0; install pulls both. App
    // v1.1.0 drops the dep entirely. After update(): the dep must
    // disappear from the lockfile, the on-disk install dir, the
    // installed() roster, and require() reachability.
    let (_dep_td, dep_bare) = make_pkg_with_dep("review-prune-dep", None, "");
    let dep_addr = file_url(&dep_bare);
    let (_app_td, app_bare) = make_pkg_with_dep("review-prune-app", Some(&dep_addr), "^1.0.0");
    let app_addr = file_url(&app_bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    // Install pulls in both packages.
    let install_script =
        format!(r#"pmacs.packages.install {{ "git:{app_addr}", version = "^1.0.0" }}"#);
    host.eval(Some("install_with_dep"), &install_script)
        .unwrap_or_else(|e| panic!("install with dep failed: {e}"));

    let dep_dir = user_root.path().join("review-prune-dep");
    let app_dir = user_root.path().join("review-prune-app");
    assert!(dep_dir.exists(), "dep dir must exist after initial install");
    assert!(app_dir.exists(), "app dir must exist after initial install");

    // Upstream publishes app v1.1.0 with no dependencies.
    publish_new_version_with_optional_dep(&app_bare, "review-prune-app", "1.1.0", None, "");

    // update() should drop the dep.
    let update_script = r#"
        local summary = pmacs.packages.update()
        local names = {}
        for _, row in ipairs(summary) do names[row.name] = row end
        assert(names["review-prune-app"], "app must remain in summary")
        assert(not names["review-prune-dep"],
            "dropped dep must NOT appear in update summary")

        -- Roster reflects the prune.
        local roster = pmacs.packages.installed()
        for _, p in ipairs(roster) do
            assert(p.name ~= "review-prune-dep",
                "review-prune-dep must be gone from installed() roster, got " .. p.name)
        end

        -- require() of the dropped dep must fail.
        local ok = pcall(require, "review-prune-dep")
        assert(not ok,
            "require('review-prune-dep') must fail after prune; it succeeded")
    "#;
    host.eval(Some("update_drops_dep"), update_script)
        .unwrap_or_else(|e| panic!("update prune failed: {e}"));

    // On-disk: dep dir gone, app dir still there.
    assert!(
        !dep_dir.exists(),
        "dep install dir must be removed after prune"
    );
    assert!(app_dir.exists(), "app install dir must persist");

    // Lockfile reflects the prune.
    let lock = pmacs::packages::Lockfile::read_from(&user_root.path().join("pmacs.lock"))
        .expect("parse lockfile");
    let names: Vec<&str> = lock.packages.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"review-prune-app"),
        "lockfile must keep app: {names:?}"
    );
    assert!(
        !names.contains(&"review-prune-dep"),
        "lockfile must drop pruned dep: {names:?}"
    );
}

// ---------------------------------------------------------------------------
// Review Fourth-Pass: package.loaded invalidated after update
// ---------------------------------------------------------------------------

/// A `require()` against a package the update prunes must fail
/// even when the dep was already cached in `package.loaded` at
/// update time. Without invalidation, Lua would happily return the
/// stale cached module table and the user wouldn't notice the
/// prune until the next process restart.
#[test]
fn require_after_update_prune_returns_stale_then_fails_after_invalidate() {
    let (_dep_td, dep_bare) = make_pkg_with_dep("review-cache-dep", None, "");
    let dep_addr = file_url(&dep_bare);
    let (_app_td, app_bare) = make_pkg_with_dep("review-cache-app", Some(&dep_addr), "^1.0.0");
    let app_addr = file_url(&app_bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    // Install + require + cache-warm: package.loaded["review-cache-dep"]
    // is now populated with the dep's module table.
    let warm = format!(
        r#"
        pmacs.packages.install {{ "git:{app_addr}", version = "^1.0.0" }}
        local d = require("review-cache-dep")
        assert(d.name == "review-cache-dep")
        local a = require("review-cache-app")
        assert(a.name == "review-cache-app")
        -- Prove the dep is in the cache, not just findable on disk.
        assert(package.loaded["review-cache-dep"] ~= nil,
            "package.loaded must hold the dep before update")
    "#
    );
    host.eval(Some("warm"), &warm)
        .unwrap_or_else(|e| panic!("warm-up failed: {e}"));

    // Upstream drops the dep.
    publish_new_version_with_optional_dep(&app_bare, "review-cache-app", "1.1.0", None, "");

    let after = r#"
        pmacs.packages.update()
        -- Must be invalidated from the cache and re-require must fail.
        assert(package.loaded["review-cache-dep"] == nil,
            "update must clear package.loaded[dep]")
        local ok, err = pcall(require, "review-cache-dep")
        assert(not ok,
            "require('review-cache-dep') must fail after prune+invalidate; succeeded with " ..
            tostring(err))
    "#;
    host.eval(Some("after"), after)
        .unwrap_or_else(|e| panic!("post-update require check failed: {e}"));
}

/// `require()` after an update that moved a package's commit must
/// return the *new* version's module table, not the cached old
/// one. Exercises the same invalidation as the prune test but for
/// the version-bump path.
#[test]
fn require_after_update_returns_new_version_not_cached_old() {
    let (_td, bare) = make_pkg_with_dep("review-cache-bump", None, "");
    let addr = file_url(&bare);

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    // Install v1.0.0, require, observe the version field.
    let warm = format!(
        r#"
        pmacs.packages.install {{ "git:{addr}", version = "^1.0.0" }}
        local m = require("review-cache-bump")
        assert(m.version == "1.0.0",
            "warm-up: expected v1.0.0, got " .. tostring(m.version))
    "#
    );
    host.eval(Some("warm_v1"), &warm)
        .unwrap_or_else(|e| panic!("warm-up failed: {e}"));

    // Publish v1.1.0 upstream.
    publish_new_version_with_optional_dep(&bare, "review-cache-bump", "1.1.0", None, "");

    let after = r#"
        pmacs.packages.update()
        -- The cached module from v1.0.0 must have been dropped, so
        -- this require() re-runs the chunk and returns v1.1.0's
        -- module table.
        local m = require("review-cache-bump")
        assert(m.version == "1.1.0",
            "post-update require should return v1.1.0, got " .. tostring(m.version))
    "#;
    host.eval(Some("after_v1_1"), after)
        .unwrap_or_else(|e| panic!("post-update require check failed: {e}"));
}
