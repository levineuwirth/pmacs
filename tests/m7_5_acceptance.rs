// tests/m7_5_acceptance.rs --- Acceptance tests for T M7.5 (resolver).

//! Drives the [`pmacs::packages::Resolver`] end-to-end against real
//! bare Git repositories (constructed in tempdirs). The four spec
//! acceptance bullets at `pmacs-tasks.tex:3244`:
//!
//! 1. Resolver succeeds on a three-package transitive dependency
//!    graph.
//! 2. Unresolvable conflicts produce errors naming the conflicting
//!    constraint paths.
//! 3. `conflicts` declarations in manifests are honored: two packages
//!    declared incompatible cannot both be installed.
//! 4. Resolution is deterministic: the same input always produces the
//!    same output.
//!
//! Plus the pin-kind sub-cases agreed during M7.5 design:
//!
//! - Top-level branch pin: plan records both the pin and the resolved
//!   commit so M7.6's lockfile can re-resolve branches on `update`.
//! - Top-level commit pin: degenerate path; manifest read straight
//!   off the commit.
//! - `pmacs_required` filter is distinct from version-constraint
//!   filter: separate error variants make "loosen your constraint"
//!   and "upgrade pmacs" remediable independently.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use semver::{Version, VersionReq};
use tempfile::TempDir;

use pmacs::packages::{
    Address, Fetcher, InstallPin, ResolveError, ResolveRequest, Resolver, Source,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// One published version of a fixture package.
#[derive(Debug, Clone)]
struct VersionSpec<'a> {
    /// Semver to publish (used for both the manifest's `version`
    /// field and the Git tag, prefixed with `v`).
    version: &'a str,
    /// Manifest's `pmacs_required` for this version.
    pmacs_required: &'a str,
    /// Each entry is `(address, version_constraint)`.
    dependencies: Vec<(String, String)>,
    /// Each entry is `(address, version_constraint)`.
    conflicts: Vec<(String, String)>,
}

impl<'a> VersionSpec<'a> {
    fn new(version: &'a str) -> Self {
        Self {
            version,
            pmacs_required: ">= 0.1.0",
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    fn with_pmacs_required(mut self, req: &'a str) -> Self {
        self.pmacs_required = req;
        self
    }

    fn with_dependency(mut self, address: &str, constraint: &str) -> Self {
        self.dependencies
            .push((address.to_string(), constraint.to_string()));
        self
    }

    fn with_conflict(mut self, address: &str, constraint: &str) -> Self {
        self.conflicts
            .push((address.to_string(), constraint.to_string()));
        self
    }
}

/// Build a bare Git repo for a package with one or more tagged
/// versions. Each `VersionSpec` becomes a commit on `main` with a
/// matching `vX.Y.Z` tag.
fn make_versioned_package(name: &str, versions: &[VersionSpec<'_>]) -> (TempDir, PathBuf) {
    assert!(
        !versions.is_empty(),
        "make_versioned_package: need at least one version",
    );

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

    for v in versions {
        let manifest = render_manifest(name, v);
        std::fs::write(work.join("pmacs.toml"), manifest).expect("write pmacs.toml");
        std::fs::write(
            work.join("init.lua"),
            format!(
                "return {{ name = '{name}', version = '{version}' }}\n",
                version = v.version,
            ),
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
            OsStr::new(v.version),
        ]);
        run_git(&[
            OsStr::new("-C"),
            work.as_os_str(),
            OsStr::new("tag"),
            OsStr::new(&format!("v{}", v.version)),
        ]);
    }
    run_git(&[
        OsStr::new("clone"),
        OsStr::new("--bare"),
        work.as_os_str(),
        bare.as_os_str(),
    ]);

    (td, bare)
}

fn render_manifest(name: &str, v: &VersionSpec<'_>) -> String {
    use std::fmt::Write as _;
    let mut out = format!(
        "name = \"{name}\"\n\
         version = \"{version}\"\n\
         summary = \"acceptance fixture\"\n\
         pmacs_required = \"{pmacs_required}\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n",
        version = v.version,
        pmacs_required = v.pmacs_required,
    );
    for (addr, constraint) in &v.dependencies {
        out.push_str("[[dependencies]]\n");
        let _ = writeln!(out, "address = \"{addr}\"");
        let _ = writeln!(out, "version = \"{constraint}\"");
    }
    for (addr, constraint) in &v.conflicts {
        out.push_str("[[conflicts]]\n");
        let _ = writeln!(out, "address = \"{addr}\"");
        let _ = writeln!(out, "version = \"{constraint}\"");
    }
    out
}

/// Build a bare repo with a single tagged version + a feature branch
/// whose head is a different commit. Returns `(tempdir, bare_path,
/// feature_branch_head_sha)`. Useful for branch / commit pin tests.
fn make_branched_package(name: &str) -> (TempDir, PathBuf, String) {
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

    // v1.0.0 on main.
    let v1 = VersionSpec::new("1.0.0");
    std::fs::write(work.join("pmacs.toml"), render_manifest(name, &v1)).expect("write manifest v1");
    std::fs::write(
        work.join("init.lua"),
        format!("return {{ name = '{name}', version = '1.0.0' }}\n"),
    )
    .expect("write init.lua v1");
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

    // Feature branch with a different commit; same name+version in
    // manifest (the branch carries an in-progress build).
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("checkout"),
        OsStr::new("-b"),
        OsStr::new("feature"),
    ]);
    std::fs::write(
        work.join("init.lua"),
        format!("return {{ name = '{name}', version = '1.0.0', flavor = 'feature' }}\n"),
    )
    .expect("write init.lua feature");
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

    let feature_head = git_rev_parse_head(&work);

    run_git(&[
        OsStr::new("clone"),
        OsStr::new("--bare"),
        work.as_os_str(),
        bare.as_os_str(),
    ]);
    (td, bare, feature_head)
}

fn git_rev_parse_head(repo: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("rev-parse")
        .arg("HEAD")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git rev-parse HEAD");
    assert!(
        out.status.success(),
        "git rev-parse HEAD failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout)
        .expect("rev-parse output utf-8")
        .trim()
        .to_string()
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

fn file_address(p: &Path) -> String {
    format!("git:file://{}", p.display())
}

fn make_resolver() -> (Resolver, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
    (Resolver::new(fetcher), cache)
}

fn version_pin(constraint: &str) -> InstallPin {
    InstallPin::Version(VersionReq::parse(constraint).expect("parse VersionReq"))
}

fn parse_address(s: &str) -> Address {
    Address::parse(s).expect("parse address")
}

// ---------------------------------------------------------------------------
// Acceptance bullet 1: transitive dependency graph
// ---------------------------------------------------------------------------

#[test]
fn resolver_resolves_transitive_dependency_chain() {
    // A depends on B (^1); B depends on C (^1); top-level installs A.
    // Expected plan: [C, B, A] — dependencies before dependents.
    let (_c_td, c_bare) = make_versioned_package("c-pkg", &[VersionSpec::new("1.0.0")]);
    let c_addr = file_address(&c_bare);

    let (_b_td, b_bare) = make_versioned_package(
        "b-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let b_addr = file_address(&b_bare);

    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&b_addr, "^1.0.0")],
    );
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve");

    let names: Vec<&str> = plan.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["c-pkg", "b-pkg", "a-pkg"],
        "expected topological order C -> B -> A, got {names:?}",
    );

    // Each entry has the manifest version and a 40-char commit.
    for entry in &plan.packages {
        assert_eq!(entry.version, Version::new(1, 0, 0));
        assert!(
            entry.commit.is_empty() || entry.commit.len() == 40 || entry.commit.starts_with('v'),
            "expected commit to be a 40-char SHA or tag-resolved, got {:?}",
            entry.commit,
        );
    }

    // Top-level pin is recorded on A only; B and C came in transitively.
    let a = plan
        .packages
        .iter()
        .find(|p| p.name.as_str() == "a-pkg")
        .unwrap();
    assert!(matches!(a.top_level_pin, Some(InstallPin::Version(_))));
    let b = plan
        .packages
        .iter()
        .find(|p| p.name.as_str() == "b-pkg")
        .unwrap();
    assert!(
        b.top_level_pin.is_none(),
        "transitive deps have no top_level_pin"
    );
    let c = plan
        .packages
        .iter()
        .find(|p| p.name.as_str() == "c-pkg")
        .unwrap();
    assert!(c.top_level_pin.is_none());
}

// ---------------------------------------------------------------------------
// Acceptance bullet 2: unresolvable version conflict names paths
// ---------------------------------------------------------------------------

#[test]
fn resolver_errors_on_unresolvable_version_conflict() {
    // C publishes v1.0.0 and v2.0.0.
    // A (top-level) requires C ^1; D (top-level) requires C ^2.
    // No version of C satisfies both.
    let (_c_td, c_bare) = make_versioned_package(
        "c-pkg",
        &[VersionSpec::new("1.0.0"), VersionSpec::new("2.0.0")],
    );
    let c_addr = file_address(&c_bare);

    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let a_addr = file_address(&a_bare);

    let (_d_td, d_bare) = make_versioned_package(
        "d-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^2.0.0")],
    );
    let d_addr = file_address(&d_bare);

    let (resolver, _cache) = make_resolver();
    let err = resolver
        .resolve(&[
            ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            },
            ResolveRequest {
                address: parse_address(&d_addr),
                pin: version_pin("^1.0.0"),
            },
        ])
        .expect_err("expected unresolvable conflict");

    let msg = err.to_string();
    assert!(
        matches!(err, ResolveError::NoVersionMatchesConstraints { .. }),
        "expected NoVersionMatchesConstraints, got {err:?}",
    );
    assert!(
        msg.contains("a-pkg") && msg.contains("d-pkg"),
        "error must name both constraint sources, got: {msg}",
    );
    assert!(
        msg.contains("^1.0.0") && msg.contains("^2.0.0"),
        "error must name both constraints, got: {msg}",
    );
    assert!(
        msg.contains("v1.0.0") && msg.contains("v2.0.0"),
        "error must list available tags, got: {msg}",
    );
}

// ---------------------------------------------------------------------------
// Acceptance bullet 3: manifest conflict declaration is honored
// ---------------------------------------------------------------------------

#[test]
fn resolver_errors_on_manifest_conflict_declaration() {
    // A and B published independently. A's manifest declares
    // `conflicts: B = "*"`. Both top-level. Resolver must error
    // naming both packages and the conflict clause.
    let (_b_td, b_bare) = make_versioned_package("b-pkg", &[VersionSpec::new("1.0.0")]);
    let b_addr = file_address(&b_bare);

    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0").with_conflict(&b_addr, "*")],
    );
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let err = resolver
        .resolve(&[
            ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            },
            ResolveRequest {
                address: parse_address(&b_addr),
                pin: version_pin("^1.0.0"),
            },
        ])
        .expect_err("expected manifest conflict");

    let msg = err.to_string();
    assert!(
        matches!(err, ResolveError::ManifestConflict { .. }),
        "expected ManifestConflict, got {err:?}",
    );
    assert!(
        msg.contains("a-pkg") && msg.contains("b-pkg"),
        "error must name both packages, got: {msg}",
    );
    assert!(
        msg.contains("conflict"),
        "error must call this a conflict, got: {msg}",
    );
}

// ---------------------------------------------------------------------------
// Acceptance bullet 4: determinism
// ---------------------------------------------------------------------------

#[test]
fn resolver_resolution_is_deterministic() {
    // Build the same A → B → C graph as the transitive test, run
    // the resolver twice, assert the plans are byte-equal under JSON
    // serialization (the eventual lockfile representation).
    let (_c_td, c_bare) = make_versioned_package("c-pkg", &[VersionSpec::new("1.0.0")]);
    let c_addr = file_address(&c_bare);
    let (_b_td, b_bare) = make_versioned_package(
        "b-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let b_addr = file_address(&b_bare);
    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&b_addr, "^1.0.0")],
    );
    let a_addr = file_address(&a_bare);

    let request = || ResolveRequest {
        address: parse_address(&a_addr),
        pin: version_pin("^1.0.0"),
    };

    let (resolver_one, _cache_one) = make_resolver();
    let plan_one = resolver_one.resolve(&[request()]).expect("resolve 1");

    let (resolver_two, _cache_two) = make_resolver();
    let plan_two = resolver_two.resolve(&[request()]).expect("resolve 2");

    let json_one = serde_json::to_string_pretty(&plan_one).expect("serialize plan 1");
    let json_two = serde_json::to_string_pretty(&plan_two).expect("serialize plan 2");
    assert_eq!(
        json_one, json_two,
        "two resolves of the same input must produce byte-identical plans",
    );
}

// ---------------------------------------------------------------------------
// pmacs_required filter is distinct from version-constraint filter
// ---------------------------------------------------------------------------

#[test]
fn resolver_distinguishes_pmacs_required_from_user_constraint() {
    // A publishes v1.0.0 with pmacs_required = ">= 99.0.0" (this
    // pmacs is 0.1.0, so incompatible). User asks for any version.
    // Expect NoVersionMatchesPmacsRequirement, not
    // NoVersionMatchesConstraints — the user can satisfy the version
    // constraint, just not the pmacs requirement.
    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0").with_pmacs_required(">= 99.0.0")],
    );
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let err = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("*"),
        }])
        .expect_err("expected pmacs incompatibility");

    assert!(
        matches!(err, ResolveError::NoVersionMatchesPmacsRequirement { .. }),
        "expected NoVersionMatchesPmacsRequirement, got {err:?}",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("pmacs") && msg.contains("99.0.0"),
        "error should mention pmacs and the required version, got: {msg}",
    );
}

#[test]
fn resolver_errors_with_constraint_variant_when_only_user_constraint_unsatisfied() {
    // A publishes v1.0.0 only. User asks for ^99. Expect
    // NoVersionMatchesConstraints — the user constraint excludes
    // every published version.
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let err = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^99.0.0"),
        }])
        .expect_err("expected version constraint failure");

    assert!(
        matches!(err, ResolveError::NoVersionMatchesConstraints { .. }),
        "expected NoVersionMatchesConstraints, got {err:?}",
    );
}

// ---------------------------------------------------------------------------
// Branch + commit pin paths
// ---------------------------------------------------------------------------

#[test]
fn resolver_branch_pin_records_pin_and_resolved_commit() {
    let (_a_td, a_bare, feature_head) = make_branched_package("a-pkg");
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: InstallPin::Branch("feature".to_string()),
        }])
        .expect("resolve branch pin");

    assert_eq!(plan.packages.len(), 1);
    let entry = &plan.packages[0];
    assert_eq!(
        entry.commit, feature_head,
        "branch HEAD must be the resolved commit"
    );
    assert!(
        matches!(entry.top_level_pin, Some(InstallPin::Branch(ref b)) if b == "feature"),
        "plan must record the original branch pin, got {:?}",
        entry.top_level_pin,
    );
}

#[test]
fn resolver_commit_pin_uses_exact_revision() {
    let (_a_td, a_bare, feature_head) = make_branched_package("a-pkg");
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: InstallPin::Commit(feature_head.clone()),
        }])
        .expect("resolve commit pin");

    assert_eq!(plan.packages.len(), 1);
    let entry = &plan.packages[0];
    assert_eq!(entry.commit, feature_head);
    assert!(
        matches!(entry.top_level_pin, Some(InstallPin::Commit(ref c)) if c == &feature_head),
        "plan must record the original commit pin, got {:?}",
        entry.top_level_pin,
    );
}

// ---------------------------------------------------------------------------
// Diamond dependency: both paths converge on a single chosen version
// ---------------------------------------------------------------------------

#[test]
fn resolver_resolves_diamond_dependency() {
    // C publishes v1.0.0 and v1.1.0. B and D both depend on C ^1;
    // A depends on both B and D. Top-level: A. Expected: C@1.1.0
    // (highest matching), B and D reuse the same C entry.
    let (_c_td, c_bare) = make_versioned_package(
        "c-pkg",
        &[VersionSpec::new("1.0.0"), VersionSpec::new("1.1.0")],
    );
    let c_addr = file_address(&c_bare);

    let (_b_td, b_bare) = make_versioned_package(
        "b-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let b_addr = file_address(&b_bare);

    let (_d_td, d_bare) = make_versioned_package(
        "d-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let d_addr = file_address(&d_bare);

    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0")
            .with_dependency(&b_addr, "^1.0.0")
            .with_dependency(&d_addr, "^1.0.0")],
    );
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve diamond");

    let c_entries: Vec<_> = plan
        .packages
        .iter()
        .filter(|p| p.name.as_str() == "c-pkg")
        .collect();
    assert_eq!(c_entries.len(), 1, "C must appear exactly once in the plan");
    assert_eq!(c_entries[0].version, Version::new(1, 1, 0));

    // Topological invariant: C precedes both B and D, which precede A.
    let pos = |name: &str| {
        plan.packages
            .iter()
            .position(|p| p.name.as_str() == name)
            .unwrap()
    };
    assert!(pos("c-pkg") < pos("b-pkg"));
    assert!(pos("c-pkg") < pos("d-pkg"));
    assert!(pos("b-pkg") < pos("a-pkg"));
    assert!(pos("d-pkg") < pos("a-pkg"));
}

// ---------------------------------------------------------------------------
// Constraint narrowing: transitive dep narrows top-level choice
// ---------------------------------------------------------------------------

#[test]
fn resolver_narrows_top_level_choice_via_transitive_constraint() {
    // C publishes v1.0.0 and v2.0.0. A depends on C ^1 (excludes v2).
    // Top-level: C @ "*" (would normally pick v2.0.0) and A @ ^1.
    // After resolution, C must be v1.0.0 — A's transitive constraint
    // dragged the top-level choice down.
    let (_c_td, c_bare) = make_versioned_package(
        "c-pkg",
        &[VersionSpec::new("1.0.0"), VersionSpec::new("2.0.0")],
    );
    let c_addr = file_address(&c_bare);

    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[
            ResolveRequest {
                address: parse_address(&c_addr),
                pin: version_pin("*"),
            },
            ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            },
        ])
        .expect("resolve narrowing");

    let c = plan
        .packages
        .iter()
        .find(|p| p.name.as_str() == "c-pkg")
        .expect("c-pkg in plan");
    assert_eq!(
        c.version,
        Version::new(1, 0, 0),
        "transitive ^1 must override top-level *",
    );
}

// ---------------------------------------------------------------------------
// Source enum constraint paths land in the constraints error
// ---------------------------------------------------------------------------

#[test]
fn resolver_constraint_error_attributes_each_path() {
    // Single overconstrained name (C). Two transitive constraints,
    // both incompatible with each other. The error's constraint list
    // should include both Source::DependencyOf entries.
    let (_c_td, c_bare) = make_versioned_package(
        "c-pkg",
        &[VersionSpec::new("1.0.0"), VersionSpec::new("2.0.0")],
    );
    let c_addr = file_address(&c_bare);

    let (_a_td, a_bare) = make_versioned_package(
        "a-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let a_addr = file_address(&a_bare);

    let (_b_td, b_bare) = make_versioned_package(
        "b-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^2.0.0")],
    );
    let b_addr = file_address(&b_bare);

    let (resolver, _cache) = make_resolver();
    let err = resolver
        .resolve(&[
            ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            },
            ResolveRequest {
                address: parse_address(&b_addr),
                pin: version_pin("^1.0.0"),
            },
        ])
        .expect_err("expected version conflict on c-pkg");

    let ResolveError::NoVersionMatchesConstraints { constraints, .. } = &err else {
        panic!("expected NoVersionMatchesConstraints, got {err:?}");
    };

    let has_source_for = |name: &str| {
        constraints.iter().any(|(source, _req)| match source {
            Source::DependencyOf { name: pkg_name, .. } => pkg_name.as_str() == name,
            Source::TopLevel => false,
        })
    };
    assert!(
        has_source_for("a-pkg"),
        "constraints must include Source::DependencyOf {{ name: \"a-pkg\", .. }}",
    );
    assert!(
        has_source_for("b-pkg"),
        "constraints must include Source::DependencyOf {{ name: \"b-pkg\", .. }}",
    );
}
