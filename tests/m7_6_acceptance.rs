// tests/m7_6_acceptance.rs --- Acceptance tests for T M7.6 (lockfile).

//! Drives lockfile generation, round-trip, hash verification, and the
//! `UpdatePolicy` matrix end-to-end against real bare Git repositories
//! built in tempdirs. Spec acceptance bullets at
//! `pmacs-tasks.tex:3279-3288`:
//!
//! 1. Lockfile content is stable across regeneration of the same
//!    resolve (no spurious diffs).
//! 2. Two users on different machines, given the same lockfile,
//!    install identical commits.
//! 3. Content-hash mismatch (a tampered fetch) produces a clear error
//!    and refuses to install.
//! 4. `pmacs.packages.update()` regenerates the whole lockfile;
//!    `update("pkg")` regenerates only the named package.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use semver::VersionReq;
use tempfile::TempDir;

use pmacs::packages::{
    Address, ContentHash, Fetcher, InstallPin, Lockfile, LockfileEntry, LockfileError, PackageName,
    ResolveError, ResolveRequest, Resolver, UpdatePolicy,
};

// ---------------------------------------------------------------------------
// Test helpers (mirrors m7_5_acceptance.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VersionSpec<'a> {
    version: &'a str,
    pmacs_required: &'a str,
    dependencies: Vec<(String, String)>,
}

impl<'a> VersionSpec<'a> {
    fn new(version: &'a str) -> Self {
        Self {
            version,
            pmacs_required: ">= 0.1.0",
            dependencies: Vec::new(),
        }
    }

    fn with_dependency(mut self, address: &str, constraint: &str) -> Self {
        self.dependencies
            .push((address.to_string(), constraint.to_string()));
        self
    }
}

/// Build a bare Git repo with one or more tagged versions.
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
    // Determinism: pin commit timestamps so two runs of the same
    // fixture produce the same SHAs (and therefore the same content
    // hashes) even when wallclock time differs. Acceptance bullet 1
    // ("stable across regeneration") relies on this for the cross-
    // tempdir comparison test.
    let env_pairs = [
        ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00 +0000"),
        ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00 +0000"),
    ];

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
        run_git_env(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("add"),
                OsStr::new("."),
            ],
            &env_pairs,
        );
        run_git_env(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new(v.version),
            ],
            &env_pairs,
        );
        run_git_env(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("tag"),
                OsStr::new(&format!("v{}", v.version)),
            ],
            &env_pairs,
        );
    }
    run_git(&[
        OsStr::new("clone"),
        OsStr::new("--bare"),
        work.as_os_str(),
        bare.as_os_str(),
    ]);

    (td, bare)
}

/// Add a new tagged version on top of an existing bare-clone fixture's
/// working tree. Keeps the working tree's `td.path()` reusable so a
/// later `git push` (or re-clone) picks up the new tag.
fn add_version_to_existing(td: &TempDir, name: &str, v: &VersionSpec<'_>) {
    let work = td.path().join("work");
    let bare = td.path().join("upstream.git");
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
    let env_pairs = [
        ("GIT_AUTHOR_DATE", "2026-02-01T00:00:00 +0000"),
        ("GIT_COMMITTER_DATE", "2026-02-01T00:00:00 +0000"),
    ];
    run_git_env(
        &[
            OsStr::new("-C"),
            work.as_os_str(),
            OsStr::new("add"),
            OsStr::new("."),
        ],
        &env_pairs,
    );
    run_git_env(
        &[
            OsStr::new("-C"),
            work.as_os_str(),
            OsStr::new("commit"),
            OsStr::new("-m"),
            OsStr::new(v.version),
        ],
        &env_pairs,
    );
    run_git_env(
        &[
            OsStr::new("-C"),
            work.as_os_str(),
            OsStr::new("tag"),
            OsStr::new(&format!("v{}", v.version)),
        ],
        &env_pairs,
    );
    // Push the new commits + tags into the bare upstream so the
    // fetcher's next `git fetch` pulls them.
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("push"),
        bare.as_os_str(),
        OsStr::new("--tags"),
        OsStr::new("main"),
    ]);
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
    out
}

fn run_git(args: &[&OsStr]) {
    run_git_env(args, &[]);
}

fn run_git_env(args: &[&OsStr], env: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("LC_ALL", "C");
    for (k, v) in env {
        cmd.env(k, v);
    }
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
// Acceptance bullet 1: lockfile content is stable across regenerations
// ---------------------------------------------------------------------------

#[test]
fn lockfile_bytes_are_stable_across_regenerations() {
    // Build a 3-package transitive graph. Generate the lockfile twice
    // from the same plan and assert byte-identical output. Then resolve
    // again from scratch (new resolver, fresh cache) and assert the
    // bytes still match — exercises the "no spurious diffs" property
    // across runs, not just within a run.
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

    let request = ResolveRequest {
        address: parse_address(&a_addr),
        pin: version_pin("^1.0.0"),
    };

    // Run 1.
    let (resolver1, _cache1) = make_resolver();
    let plan1 = resolver1
        .resolve(std::slice::from_ref(&request))
        .expect("resolve 1");
    let lock1 = Lockfile::from_plan(&plan1, resolver1.fetcher()).expect("lockfile 1");
    let bytes1 = lock1.to_bytes().expect("bytes 1");

    // Run 2 (same plan, same fetcher).
    let lock2 = Lockfile::from_plan(&plan1, resolver1.fetcher()).expect("lockfile 2");
    let bytes2 = lock2.to_bytes().expect("bytes 2");
    assert_eq!(
        bytes1, bytes2,
        "two regenerations of same plan must be byte-equal"
    );

    // Run 3 (cold cache, fresh resolver).
    let (resolver3, _cache3) = make_resolver();
    let plan3 = resolver3
        .resolve(std::slice::from_ref(&request))
        .expect("resolve 3");
    let lock3 = Lockfile::from_plan(&plan3, resolver3.fetcher()).expect("lockfile 3");
    let bytes3 = lock3.to_bytes().expect("bytes 3");
    assert_eq!(
        bytes1, bytes3,
        "fresh-resolver lockfile must equal first-resolver lockfile",
    );
}

// ---------------------------------------------------------------------------
// Acceptance bullet 2: same lockfile → identical commits across machines
// ---------------------------------------------------------------------------

#[test]
fn frozen_resolve_yields_lockfile_commits_on_a_second_machine() {
    // Simulate "user A generates lockfile, user B installs from it":
    // produce a lockfile with one resolver, throw away the fetcher
    // cache, write the lockfile bytes, then load them on a fresh
    // resolver / cache and run a Frozen resolve. The Frozen plan must
    // pin to the same commits as the original lockfile.
    let (_c_td, c_bare) = make_versioned_package("c-pkg", &[VersionSpec::new("1.0.0")]);
    let c_addr = file_address(&c_bare);
    let (_b_td, b_bare) = make_versioned_package(
        "b-pkg",
        &[VersionSpec::new("1.0.0").with_dependency(&c_addr, "^1.0.0")],
    );
    let b_addr = file_address(&b_bare);

    // User A.
    let (resolver_a, _cache_a) = make_resolver();
    let plan_a = resolver_a
        .resolve(&[ResolveRequest {
            address: parse_address(&b_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve A");
    let lock = Lockfile::from_plan(&plan_a, resolver_a.fetcher()).expect("lockfile A");
    let lock_bytes = lock.to_bytes().expect("serialize lockfile");

    // Save lockfile commits for comparison. Sort by name so the
    // comparison is stable regardless of whether the source is the
    // alphabetical lockfile or the topological plan.
    let mut commits_a: Vec<(String, String)> = lock
        .packages
        .iter()
        .map(|e| (e.name.as_str().to_string(), e.commit.clone()))
        .collect();
    commits_a.sort();

    // User B: cold cache, parse the lockfile, run Frozen.
    let parsed = Lockfile::parse(&lock_bytes).expect("parse lockfile");
    let (resolver_b, _cache_b) = make_resolver();
    let plan_b = resolver_b
        .resolve_with_policy(
            &[ResolveRequest {
                address: parse_address(&b_addr),
                pin: version_pin("^1.0.0"),
            }],
            Some(&parsed),
            &UpdatePolicy::Frozen,
        )
        .expect("frozen resolve");

    let mut commits_b: Vec<(String, String)> = plan_b
        .packages
        .iter()
        .map(|p| (p.name.as_str().to_string(), p.revision.clone()))
        .collect();
    commits_b.sort();
    assert_eq!(
        commits_a, commits_b,
        "frozen resolve on a fresh machine must pin to the lockfile's commits",
    );
}

// ---------------------------------------------------------------------------
// Acceptance bullet 3: content-hash mismatch refuses to install
// ---------------------------------------------------------------------------

#[test]
fn content_hash_mismatch_produces_clear_error() {
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve");
    let mut lock = Lockfile::from_plan(&plan, resolver.fetcher()).expect("lockfile");

    // Tamper: replace the content_hash with a known-different value.
    let original_hash = lock.packages[0].content_hash.clone();
    lock.packages[0].content_hash = ContentHash::sha256_of(b"this is not the upstream content");

    let entry = lock.packages[0].clone();
    let err = lock
        .verify_entry(&entry, resolver.fetcher())
        .expect_err("hash mismatch must error");

    match &err {
        LockfileError::ContentHashMismatch {
            name,
            expected,
            observed,
            ..
        } => {
            assert_eq!(name.as_str(), "a-pkg");
            assert_ne!(expected, observed, "expected and observed must differ");
            // Observed hash must equal the lockfile's *original* hash
            // (the upstream itself is untouched, only the lockfile was
            // tampered).
            assert_eq!(
                observed, &original_hash,
                "observed hash must match the upstream-derived hash",
            );
        }
        other => panic!("expected ContentHashMismatch, got {other:?}"),
    }

    let msg = err.to_string();
    assert!(
        msg.contains("a-pkg"),
        "error message must name the package, got: {msg}",
    );
    assert!(
        msg.contains("expected") && msg.contains("observed"),
        "error message must show both hashes, got: {msg}",
    );
    assert!(
        msg.contains("Refusing to install"),
        "error message must explicitly refuse install, got: {msg}",
    );
}

#[test]
fn frozen_resolve_detects_lockfile_content_hash_tamper() {
    // The Frozen path through `to_resolve_plan` must verify each
    // entry's content hash before reading the manifest. A tampered
    // hash (e.g., a malicious lockfile served alongside a benign
    // upstream) must abort the install with ContentHashMismatch
    // rather than silently accept whatever the upstream now ships.
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let req = ResolveRequest {
        address: parse_address(&a_addr),
        pin: version_pin("^1.0.0"),
    };
    let plan = resolver
        .resolve(std::slice::from_ref(&req))
        .expect("resolve");
    let mut lock = Lockfile::from_plan(&plan, resolver.fetcher()).expect("lockfile");

    // Tamper with the content_hash recorded in the lockfile.
    lock.packages[0].content_hash = ContentHash::sha256_of(b"tampered");

    // Frozen resolve must surface the mismatch as ContentHashMismatch.
    let (resolver2, _cache2) = make_resolver();
    let err = resolver2
        .resolve_with_policy(&[req], Some(&lock), &UpdatePolicy::Frozen)
        .expect_err("Frozen with tampered content_hash must error");

    let msg = err.to_string();
    assert!(
        msg.contains("a-pkg") || msg.contains("ContentHash") || msg.contains("hash"),
        "expected hash-mismatch error from Frozen resolve, got: {msg}",
    );
}

#[test]
fn unmodified_lockfile_verifies_against_upstream() {
    // Counterpart to the tampering test: the happy path. A lockfile
    // generated from a plan must verify against its own upstream
    // without error.
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve");
    let lock = Lockfile::from_plan(&plan, resolver.fetcher()).expect("lockfile");
    for entry in &lock.packages {
        lock.verify_entry(entry, resolver.fetcher())
            .unwrap_or_else(|e| panic!("verify failed for {}: {e}", entry.name.as_str()));
    }
}

// ---------------------------------------------------------------------------
// Acceptance bullet 4: update() vs update("pkg") regeneration semantics
// ---------------------------------------------------------------------------

#[test]
fn update_all_regenerates_full_lockfile_after_upstream_changes() {
    // Lock at v1.0.0; upstream publishes v1.1.0; UpdateAll picks v1.1.0.
    let (a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);

    let (resolver1, _cache1) = make_resolver();
    let plan1 = resolver1
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve initial");
    let lock1 = Lockfile::from_plan(&plan1, resolver1.fetcher()).expect("lockfile 1");
    let initial_version = lock1.packages[0].version.clone();
    assert_eq!(initial_version.to_string(), "1.0.0");

    // Upstream publishes v1.1.0.
    add_version_to_existing(&a_td, "a-pkg", &VersionSpec::new("1.1.0"));

    // UpdateAll on a fresh resolver should pick v1.1.0.
    let (resolver2, _cache2) = make_resolver();
    let plan2 = resolver2
        .resolve_with_policy(
            &[ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            }],
            Some(&lock1),
            &UpdatePolicy::UpdateAll,
        )
        .expect("update all resolve");
    let lock2 = Lockfile::from_plan(&plan2, resolver2.fetcher()).expect("lockfile 2");
    assert_eq!(lock2.packages[0].version.to_string(), "1.1.0");
    assert_ne!(
        lock1.packages[0].commit, lock2.packages[0].commit,
        "UpdateAll must produce a different commit when upstream advanced",
    );
}

#[test]
fn update_one_only_touches_named_package() {
    // Lock A and C at v1.0.0. Upstream publishes v1.1.0 for both.
    // UpdateOne("a-pkg") bumps A to v1.1.0; C stays at v1.0.0
    // because its constraints still allow the lockfile version (the
    // hint cascade brake).
    let (a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);
    let (c_td, c_bare) = make_versioned_package("c-pkg", &[VersionSpec::new("1.0.0")]);
    let c_addr = file_address(&c_bare);

    let (resolver1, _cache1) = make_resolver();
    let plan1 = resolver1
        .resolve(&[
            ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            },
            ResolveRequest {
                address: parse_address(&c_addr),
                pin: version_pin("^1.0.0"),
            },
        ])
        .expect("resolve initial");
    let lock1 = Lockfile::from_plan(&plan1, resolver1.fetcher()).expect("lockfile 1");
    let lock1_a_commit = lock1
        .entry(&PackageName::new("a-pkg").unwrap())
        .unwrap()
        .commit
        .clone();
    let lock1_c_commit = lock1
        .entry(&PackageName::new("c-pkg").unwrap())
        .unwrap()
        .commit
        .clone();

    // Upstream advances both packages.
    add_version_to_existing(&a_td, "a-pkg", &VersionSpec::new("1.1.0"));
    add_version_to_existing(&c_td, "c-pkg", &VersionSpec::new("1.1.0"));

    let (resolver2, _cache2) = make_resolver();
    let plan2 = resolver2
        .resolve_with_policy(
            &[
                ResolveRequest {
                    address: parse_address(&a_addr),
                    pin: version_pin("^1.0.0"),
                },
                ResolveRequest {
                    address: parse_address(&c_addr),
                    pin: version_pin("^1.0.0"),
                },
            ],
            Some(&lock1),
            &UpdatePolicy::UpdateOne(PackageName::new("a-pkg").unwrap()),
        )
        .expect("update one resolve");
    let lock2 = Lockfile::from_plan(&plan2, resolver2.fetcher()).expect("lockfile 2");

    let a2 = lock2.entry(&PackageName::new("a-pkg").unwrap()).unwrap();
    let c2 = lock2.entry(&PackageName::new("c-pkg").unwrap()).unwrap();

    assert_eq!(
        a2.version.to_string(),
        "1.1.0",
        "UpdateOne must advance the named package",
    );
    assert_ne!(
        a2.commit, lock1_a_commit,
        "named package must move to a new commit",
    );

    assert_eq!(
        c2.version.to_string(),
        "1.0.0",
        "UpdateOne must not advance unrelated packages even when upstream has newer",
    );
    assert_eq!(
        c2.commit, lock1_c_commit,
        "unrelated package must keep its lockfile commit",
    );
}

#[test]
fn update_one_errors_when_package_not_in_lockfile() {
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve");
    let lock = Lockfile::from_plan(&plan, resolver.fetcher()).expect("lockfile");

    let err = resolver
        .resolve_with_policy(
            &[ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            }],
            Some(&lock),
            &UpdatePolicy::UpdateOne(PackageName::new("nonexistent").unwrap()),
        )
        .expect_err("UpdateOne on missing package errors");
    match err {
        ResolveError::Lockfile(LockfileError::UpdateOneMissing { name }) => {
            assert_eq!(name.as_str(), "nonexistent");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// FrozenRequestMissing path
// ---------------------------------------------------------------------------

#[test]
fn frozen_resolve_errors_when_request_missing_from_lockfile() {
    // Lockfile has only A; user tries Frozen resolve including B.
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);
    let (_b_td, b_bare) = make_versioned_package("b-pkg", &[VersionSpec::new("1.0.0")]);
    let b_addr = file_address(&b_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve A only");
    let lock = Lockfile::from_plan(&plan, resolver.fetcher()).expect("lockfile A");

    let err = resolver
        .resolve_with_policy(
            &[
                ResolveRequest {
                    address: parse_address(&a_addr),
                    pin: version_pin("^1.0.0"),
                },
                ResolveRequest {
                    address: parse_address(&b_addr),
                    pin: version_pin("^1.0.0"),
                },
            ],
            Some(&lock),
            &UpdatePolicy::Frozen,
        )
        .expect_err("frozen resolve with new package errors");

    match err {
        ResolveError::FrozenRequestMissing { address } => {
            assert!(
                address
                    .to_git_url()
                    .contains(b_bare.to_string_lossy().as_ref()),
                "error must name the missing package's URL",
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn lockfile_required_when_policy_demands_one() {
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);
    let (resolver, _cache) = make_resolver();
    let err = resolver
        .resolve_with_policy(
            &[ResolveRequest {
                address: parse_address(&a_addr),
                pin: version_pin("^1.0.0"),
            }],
            None,
            &UpdatePolicy::Frozen,
        )
        .expect_err("Frozen without lockfile errors");
    assert!(matches!(err, ResolveError::LockfileRequired), "got {err:?}");
}

// ---------------------------------------------------------------------------
// Lockfile structure / serialization
// ---------------------------------------------------------------------------

#[test]
fn lockfile_records_transitive_closure_with_dependency_edges() {
    // A → B → C. Lockfile must contain all three with correct edges.
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
    let lock = Lockfile::from_plan(&plan, resolver.fetcher()).expect("lockfile");

    let names: Vec<&str> = lock.packages.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["a-pkg", "b-pkg", "c-pkg"],
        "lockfile entries must be alphabetical by name",
    );

    let a = find_entry(&lock, "a-pkg");
    let b = find_entry(&lock, "b-pkg");
    let c = find_entry(&lock, "c-pkg");

    assert_eq!(a.dependencies.len(), 1);
    assert_eq!(a.dependencies[0].as_str(), "b-pkg");
    assert_eq!(b.dependencies.len(), 1);
    assert_eq!(b.dependencies[0].as_str(), "c-pkg");
    assert!(c.dependencies.is_empty());

    // Top-level pin only on A.
    assert!(a.top_level_pin.is_some());
    assert!(b.top_level_pin.is_none());
    assert!(c.top_level_pin.is_none());

    // Every commit is a 40-char SHA.
    for e in &lock.packages {
        assert_eq!(e.commit.len(), 40, "{}: {:?}", e.name.as_str(), e.commit);
        assert!(e.commit.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

fn find_entry<'a>(lock: &'a Lockfile, name: &str) -> &'a LockfileEntry {
    lock.packages
        .iter()
        .find(|e| e.name.as_str() == name)
        .unwrap_or_else(|| panic!("no entry for {name}"))
}

#[test]
fn lockfile_to_resolve_plan_yields_topological_order() {
    // Lockfile is alphabetical on disk; to_resolve_plan must restore
    // dependency-first order via the recorded `dependencies` edges.
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
    let plan_orig = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve");
    let lock = Lockfile::from_plan(&plan_orig, resolver.fetcher()).expect("lockfile");

    let plan_via_lock = lock
        .to_resolve_plan(resolver.fetcher())
        .expect("plan via lockfile");
    let names: Vec<&str> = plan_via_lock
        .packages
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, vec!["c-pkg", "b-pkg", "a-pkg"]);
}

#[test]
fn lockfile_round_trip_via_disk_io() {
    let (_a_td, a_bare) = make_versioned_package("a-pkg", &[VersionSpec::new("1.0.0")]);
    let a_addr = file_address(&a_bare);

    let (resolver, _cache) = make_resolver();
    let plan = resolver
        .resolve(&[ResolveRequest {
            address: parse_address(&a_addr),
            pin: version_pin("^1.0.0"),
        }])
        .expect("resolve");
    let lock = Lockfile::from_plan(&plan, resolver.fetcher()).expect("lockfile");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("pmacs.lock");
    lock.write_to(&path).expect("write lockfile");
    let read_back = Lockfile::read_from(&path).expect("read lockfile");
    assert_eq!(read_back, lock);
}
