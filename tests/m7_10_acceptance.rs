// tests/m7_10_acceptance.rs --- T M7.10 three-package end-to-end test.
//
//! Acceptance suite for T M7.10 ("Three-package end-to-end test").
//! Spec acceptance bullets (`pmacs-tasks.tex:3502-3510`):
//!
//! 1. Three test packages published to a Git host (we use local
//!    bare repos, per the spec's allowance: "the test can use a
//!    local bare repo or a real host").
//! 2. User-A install produces a lockfile; user-B install with the
//!    same lockfile produces identical behavior.
//! 3. All three packages pass the audit lint with zero findings.
//! 4. Test runs in CI as part of the M7 acceptance pipeline.
//!
//! Bullet 4 is satisfied implicitly: this file is a `cargo test`
//! integration test under `tests/`, so any CI that runs
//! `cargo test --features luajit` (or `... --features lua54
//! --no-default-features`) executes it. The TRANSITION-M7.md M7.10
//! section names this contract.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use pmacs::audit::{AuditEngine, Severity};
use pmacs::lua::LuaHost;
use pmacs::lua_bindings::PackageInstallOverride;
use pmacs::packages::{
    Address, Fetcher, InstallPin, InstallScope, InstallSpec, Installer, Lockfile, ResolveRequest,
    Resolver, UpdatePolicy,
};
use semver::VersionReq;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture: three trivial packages
// ---------------------------------------------------------------------------
//
// Each package is published as a local bare git repo. The bodies
// are deliberately trivial *and* audit-clean (no fs writes, no
// process spawns, no FFI, no debug-table calls, no rawget/rawset
// against `_G`, no setfenv/getfenv). The acceptance lint check
// expects zero Error/Warning findings against all three.

const HELLO_WORLD_BODY: &str = r#"return {
  name = "hello-world",
  version = "1.0.0",
  greet = function()
    return "hello, world"
  end,
}
"#;

const FORTUNE_COOKIE_BODY: &str = r#"local fortunes = {
  "you will write good code today",
  "the next test you write will pass",
  "your future self will thank you",
}

return {
  name = "fortune-cookie",
  version = "1.0.0",
  tell = function(i)
    return fortunes[((i or 1) - 1) % #fortunes + 1]
  end,
}
"#;

// Date is hardcoded so the package stays deterministic (and audit-
// clean). `os.date` is allowed by the audit rules; we keep the
// fixture trivial regardless.
const DATE_PRINTER_BODY: &str = r#"return {
  name = "date-printer",
  version = "1.0.0",
  today = function()
    return "1970-01-01"
  end,
}
"#;

#[derive(Clone, Copy)]
struct PkgFixture {
    name: &'static str,
    body: &'static str,
}

const PKGS: &[PkgFixture] = &[
    PkgFixture {
        name: "hello-world",
        body: HELLO_WORLD_BODY,
    },
    PkgFixture {
        name: "fortune-cookie",
        body: FORTUNE_COOKIE_BODY,
    },
    PkgFixture {
        name: "date-printer",
        body: DATE_PRINTER_BODY,
    },
];

/// Build a single-package bare repo and return (`tmpdir-keepalive`,
/// path-to-bare-repo, path-to-working-tree). The working tree is
/// kept around because the audit lint runs against it directly
/// (the bare repo has no checked-out files).
fn make_pkg(pkg: &PkgFixture) -> (TempDir, PathBuf, PathBuf) {
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
         summary = \"M7.10 fixture: {name}\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n",
        name = pkg.name,
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write manifest");
    std::fs::write(work.join("init.lua"), pkg.body).expect("write entry");

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
    (td, bare, work)
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

/// Install every package in `plan` through `installer`, pinning each
/// install to the plan's recorded commit. Returns the installed
/// roster in plan order. Panics on any install failure (acceptance
/// tests treat install failure as fatal).
fn install_plan(
    installer: &Installer,
    plan: &pmacs::packages::ResolvePlan,
) -> Vec<pmacs::packages::InstalledPackage> {
    let mut installed = Vec::with_capacity(plan.packages.len());
    for rp in &plan.packages {
        let spec = InstallSpec {
            address: rp.address.clone(),
            pin: InstallPin::Commit(rp.commit.clone()),
        };
        installed.push(installer.install(&spec).expect("install"));
    }
    installed
}

fn sha256_of_file(p: &Path) -> String {
    let bytes = std::fs::read(p).expect("read for hashing");
    let mut h = Sha256::new();
    h.update(&bytes);
    let digest = h.finalize();
    digest.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        write!(&mut s, "{b:02x}").expect("hex");
        s
    })
}

// ---------------------------------------------------------------------------
// Bullet 1: three packages published; full pipeline succeeds
// ---------------------------------------------------------------------------

#[test]
fn three_packages_published_resolve_and_install() {
    let fixtures: Vec<_> = PKGS.iter().map(make_pkg).collect();

    let cache = tempfile::tempdir().expect("cache");
    let install_root = tempfile::tempdir().expect("install-root");
    let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());

    let requests: Vec<ResolveRequest> = fixtures
        .iter()
        .map(|(_td, bare, _work)| ResolveRequest {
            address: Address::parse(&format!("git:{}", file_url(bare))).expect("address"),
            pin: InstallPin::Version(VersionReq::parse("^1.0.0").expect("req")),
        })
        .collect();

    let resolver = Resolver::new(fetcher);
    let plan = resolver.resolve(&requests).expect("resolve");
    assert_eq!(plan.packages.len(), 3, "expected three resolved packages");

    let names: Vec<&str> = plan.packages.iter().map(|p| p.name.as_str()).collect();
    for pkg in PKGS {
        assert!(
            names.contains(&pkg.name),
            "missing {} in plan: {names:?}",
            pkg.name
        );
    }

    let installer = Installer::new(
        Fetcher::with_cache_dir(cache.path().to_path_buf()),
        InstallScope::User,
    )
    .with_install_root_override(install_root.path().to_path_buf());

    for rp in &plan.packages {
        let spec = InstallSpec {
            address: rp.address.clone(),
            pin: InstallPin::Version(VersionReq::parse(&format!("={}", rp.version)).unwrap()),
        };
        let installed = installer.install(&spec).expect("install");
        assert_eq!(installed.manifest.name.as_str(), rp.name.as_str());
        assert!(
            installed.entry_path().exists(),
            "entry {:?} not on disk after install",
            installed.entry_path()
        );
    }
}

// ---------------------------------------------------------------------------
// Bullet 2: User-A install → lockfile → User-B install with that lockfile
// produces identical state
// ---------------------------------------------------------------------------

#[test]
fn user_a_lockfile_drives_user_b_to_identical_install_state() {
    // Both users see the same upstreams (bare repos in a third
    // tempdir we keep alive for the duration of the test).
    let fixtures: Vec<_> = PKGS.iter().map(make_pkg).collect();

    let requests: Vec<ResolveRequest> = fixtures
        .iter()
        .map(|(_td, bare, _work)| ResolveRequest {
            address: Address::parse(&format!("git:{}", file_url(bare))).expect("address"),
            pin: InstallPin::Version(VersionReq::parse("^1.0.0").expect("req")),
        })
        .collect();

    // ----- User-A: fresh install on machine A ------------------------------
    let cache_a = tempfile::tempdir().expect("cache-a");
    let install_a = tempfile::tempdir().expect("install-a");
    let project_a = tempfile::tempdir().expect("project-a");

    let resolver_a = Resolver::new(Fetcher::with_cache_dir(cache_a.path().to_path_buf()));
    let plan_a = resolver_a.resolve(&requests).expect("user-A resolve");
    let lock_a = Lockfile::from_plan(&plan_a, resolver_a.fetcher()).expect("user-A lockfile");
    let lock_a_path = project_a.path().join("pmacs.lock");
    lock_a
        .write_to(&lock_a_path)
        .expect("write user-A lockfile");

    let installer_a = Installer::new(
        Fetcher::with_cache_dir(cache_a.path().to_path_buf()),
        InstallScope::User,
    )
    .with_install_root_override(install_a.path().to_path_buf());

    let installed_a = install_plan(&installer_a, &plan_a);

    // ----- User-B: simulate a different machine ----------------------------
    // Distinct cache dir + distinct install root, BUT: the same
    // lockfile bytes copied verbatim from project-A. This is the
    // moral of "user-B opens user-A's repo and runs install".
    let cache_b = tempfile::tempdir().expect("cache-b");
    let install_b = tempfile::tempdir().expect("install-b");
    let project_b = tempfile::tempdir().expect("project-b");

    let lock_b_path = project_b.path().join("pmacs.lock");
    std::fs::copy(&lock_a_path, &lock_b_path).expect("copy lockfile A → B");

    // Read from disk on user-B's side, then drive a Frozen-policy
    // resolve. This exercises the same code path a real second user
    // would run: open their cwd, find pmacs.lock, install per it.
    let lock_b = Lockfile::read_from(&lock_b_path).expect("read user-B lockfile");
    let resolver_b = Resolver::new(Fetcher::with_cache_dir(cache_b.path().to_path_buf()));
    let plan_b = resolver_b
        .resolve_with_policy(&requests, Some(&lock_b), &UpdatePolicy::Frozen)
        .expect("user-B frozen resolve");

    let installer_b = Installer::new(
        Fetcher::with_cache_dir(cache_b.path().to_path_buf()),
        InstallScope::User,
    )
    .with_install_root_override(install_b.path().to_path_buf());

    let installed_b = install_plan(&installer_b, &plan_b);

    // ----- Identity assertions --------------------------------------------
    // 1. Lockfile bytes round-trip identically.
    let bytes_a = std::fs::read(&lock_a_path).expect("re-read user-A lockfile");
    let bytes_b = std::fs::read(&lock_b_path).expect("re-read user-B lockfile");
    assert_eq!(
        bytes_a, bytes_b,
        "user-A and user-B lockfile bytes must be identical"
    );

    // 2. Same set of packages, each pinned to the same commit and
    //    content hash.
    assert_eq!(installed_a.len(), 3);
    assert_eq!(installed_b.len(), 3);

    let by_name_a: HashMap<&str, &pmacs::packages::InstalledPackage> = installed_a
        .iter()
        .map(|p| (p.manifest.name.as_str(), p))
        .collect();
    let by_name_b: HashMap<&str, &pmacs::packages::InstalledPackage> = installed_b
        .iter()
        .map(|p| (p.manifest.name.as_str(), p))
        .collect();
    for pkg in PKGS {
        let a = by_name_a
            .get(pkg.name)
            .unwrap_or_else(|| panic!("user-A missing {}", pkg.name));
        let b = by_name_b
            .get(pkg.name)
            .unwrap_or_else(|| panic!("user-B missing {}", pkg.name));
        assert_eq!(a.commit, b.commit, "{} commit drift A→B", pkg.name);
        assert_eq!(
            a.manifest.version, b.manifest.version,
            "{} version drift A→B",
            pkg.name
        );

        // 3. The installed entry file (init.lua on disk) is byte-
        //    identical between A and B. Stronger than the lockfile's
        //    own content_hash check, because it asserts the
        //    *checked-out* tree on each machine matches.
        let hash_a = sha256_of_file(&a.entry_path());
        let hash_b = sha256_of_file(&b.entry_path());
        assert_eq!(
            hash_a,
            hash_b,
            "{} entry file hash drift A→B (paths: {:?} vs {:?})",
            pkg.name,
            a.entry_path(),
            b.entry_path()
        );
    }

    // 4. Lockfile entries match per-package commit and content hash.
    for pkg in PKGS {
        let a_entry = lock_a
            .packages
            .iter()
            .find(|e| e.name.as_str() == pkg.name)
            .unwrap_or_else(|| panic!("lockfile missing {}", pkg.name));
        let b_entry = lock_b
            .packages
            .iter()
            .find(|e| e.name.as_str() == pkg.name)
            .unwrap_or_else(|| panic!("lockfile B missing {}", pkg.name));
        assert_eq!(a_entry.commit, b_entry.commit);
        assert_eq!(a_entry.content_hash, b_entry.content_hash);
    }
}

// ---------------------------------------------------------------------------
// Bullet 3: three packages pass the audit lint with zero findings
// ---------------------------------------------------------------------------

#[test]
fn three_packages_pass_audit_lint_with_zero_findings() {
    let fixtures: Vec<_> = PKGS.iter().map(make_pkg).collect();

    let engine = AuditEngine::new().expect("audit engine");
    for ((pkg, _td, _bare, work), pkg_fixture) in fixtures
        .iter()
        .map(|(td, bare, work)| (td, td, bare, work))
        .zip(PKGS.iter())
    {
        let _ = pkg;
        // Lint the working tree (the bare repo has no checked-out
        // files; the package source the audit cares about is the
        // working tree at the tagged commit).
        let findings = engine
            .audit_dir(work)
            .unwrap_or_else(|e| panic!("audit_dir({work:?}) failed: {e}"));
        let bad: Vec<_> = findings
            .iter()
            .filter(|f| matches!(f.severity, Severity::Error | Severity::Warning))
            .collect();
        assert!(
            bad.is_empty(),
            "package `{}` should be audit-clean; got {:#?}",
            pkg_fixture.name,
            bad
        );
    }
}

// ---------------------------------------------------------------------------
// "Identical behavior" — exercised through the Lua loader.
// Both users observe the same return value when they require()
// each package and call the entry function. The Rust-side test
// above proved on-disk identity; this one proves the loader and
// Lua VM agree on what those bytes mean.
// ---------------------------------------------------------------------------

#[test]
fn three_packages_load_via_lua_require_and_return_expected_values() {
    let fixtures: Vec<_> = PKGS.iter().map(make_pkg).collect();

    let cache = tempfile::tempdir().expect("cache");
    let user_root = tempfile::tempdir().expect("user-root");
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );

    let urls: Vec<String> = fixtures
        .iter()
        .map(|(_td, bare, _work)| file_url(bare))
        .collect();

    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{u_hello}", version = "^1.0.0" }}
        pmacs.packages.install {{ "git:{u_fortune}", version = "^1.0.0" }}
        pmacs.packages.install {{ "git:{u_date}", version = "^1.0.0" }}

        local hello = require("hello-world")
        assert(hello.greet() == "hello, world",
            "hello-world.greet() returned " .. tostring(hello.greet()))

        local fortune = require("fortune-cookie")
        assert(fortune.tell(1) == "you will write good code today",
            "fortune-cookie.tell(1) returned " .. tostring(fortune.tell(1)))

        local date = require("date-printer")
        assert(date.today() == "1970-01-01",
            "date-printer.today() returned " .. tostring(date.today()))

        return "ok"
    "#,
        u_hello = urls[0],
        u_fortune = urls[1],
        u_date = urls[2],
    );
    host.eval(Some("m7_10_acceptance"), &script)
        .unwrap_or_else(|e| panic!("three-package require/run failed: {e}"));
    assert!(host.errors().is_empty(), "errors: {:?}", host.errors());
}
