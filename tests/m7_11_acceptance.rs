// tests/m7_11_acceptance.rs --- T M7.11 REPL migration acceptance.
//
//! Acceptance suite for T M7.11. Spec acceptance bullets
//! (`pmacs-tasks.tex:3557-3572`):
//!
//! 1. Package author's guide published, including v1.0 address-scheme
//!    set and forge-alias extension path.
//! 2. REPL package source moved to `builtin/packages/repl/`, manifest
//!    present, exports declared, bootstrap loads it through the
//!    package system.
//! 3. All M6.4-M6.9 acceptance tests still pass against the migrated
//!    REPL. (Verified by their own suites; this file does not
//!    re-run them but documents the dependency.)
//! 4. T M7.9 audit lint reports zero Error-severity findings against
//!    the migrated REPL package; any Warning-level findings are
//!    classified into the M11 watchlist with a one-line rationale.
//! 5. M7 tag created. (Manual: outside test scope.)
//! 6. `TRANSITION-M7.md` published. (Outside test scope; the file
//!    exists at the repo root and contains the M7.11 section.)
//!
//! Bullets 1, 5, and 6 are doc / git-tag deliverables; the four
//! tests below cover bullets 2 and 4 plus a smoke check that the
//! migrated REPL is reachable via `require("repl")` after editor
//! init.

use std::path::Path;

use pmacs::audit::{AuditEngine, Severity};
use pmacs::builtin_packages::{BUNDLED_PACKAGES, materialize_all};
use pmacs::editor::EditorState;
use pmacs::packages::PackageManifest;

// ---------------------------------------------------------------------------
// Bullet 2a: source layout
// ---------------------------------------------------------------------------

#[test]
fn repl_package_lives_under_builtin_packages_with_manifest() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pkg_dir = root.join("builtin").join("packages").join("repl");
    assert!(
        pkg_dir.is_dir(),
        "expected migrated REPL package dir at {pkg_dir:?}"
    );
    assert!(
        pkg_dir.join("init.lua").is_file(),
        "expected entry at {:?}",
        pkg_dir.join("init.lua")
    );
    assert!(
        pkg_dir.join("pmacs.toml").is_file(),
        "expected manifest at {:?}",
        pkg_dir.join("pmacs.toml")
    );

    // The pre-M7.11 home of the source must be gone.
    let old = root.join("builtin").join("runtime").join("repl.lua");
    assert!(
        !old.exists(),
        "pre-migration repl source should be moved, but found at {old:?}"
    );
}

#[test]
fn repl_manifest_declares_exports_and_pmacs_required() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toml = std::fs::read_to_string(root.join("builtin/packages/repl/pmacs.toml"))
        .expect("read repl manifest");
    let m = PackageManifest::from_toml(&toml).expect("parse repl manifest");
    assert_eq!(m.name.as_str(), "repl");
    assert_eq!(m.entry.to_str().unwrap(), "init.lua");
    assert!(
        m.exports.iter().any(|e| e == "repl"),
        "manifest must export `repl`; got {:?}",
        m.exports
    );
    // pmacs_required must accept the running pmacs version, else
    // the package is unloadable on its own host.
    let running =
        semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is semver");
    assert!(
        m.pmacs_required.matches(&running),
        "manifest pmacs_required {} should accept running version {running}",
        m.pmacs_required
    );
}

// ---------------------------------------------------------------------------
// Bullet 2b: bootstrap loads through the package system
// ---------------------------------------------------------------------------
//
// EditorState::new() runs the M7.11 bootstrap that materializes
// the bundled REPL and pushes its InstalledPackage record into
// the roster. After that, `require("repl")` from Lua resolves
// through the M7.7 searcher (not the legacy direct eval).

#[test]
fn editor_init_makes_repl_loadable_via_require() {
    let state = EditorState::new();
    let result: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            -- pmacs.packages.load returns true on success and logs to *errors* on
            -- failure. The bundled REPL must already be loaded by the time the
            -- editor finishes its bootstrap (the editor's own setup calls
            -- pmacs.packages.load("repl") through eval), so a follow-up
            -- pmacs.packages.installed() must list it.
            local roster = pmacs.packages.installed()
            for _, p in ipairs(roster) do
                if p.name == "repl" then return true end
            end
            return false
        "#,
        )
        .eval()
        .expect("query installed roster");
    assert!(
        result,
        "bundled REPL must be in pmacs.packages.installed() after editor init"
    );
}

// ---------------------------------------------------------------------------
// Bullet 4: audit lint reports zero Error-severity findings against the
// migrated REPL; Warning findings are classified.
// ---------------------------------------------------------------------------

#[test]
fn audit_lint_against_migrated_repl_reports_no_error_findings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pkg_dir = root.join("builtin/packages/repl");
    let engine = AuditEngine::new().expect("audit engine");
    let findings = engine
        .audit_dir(&pkg_dir)
        .expect("audit migrated REPL package");

    let errors: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "migrated REPL must have zero Error-severity audit findings; got {errors:#?}"
    );

    // M11 watchlist classification (per spec acceptance bullet 4).
    // Each Warning-level finding is documented inline so a future
    // reader can see why it is permitted:
    //
    //   * `no-process-spawn-pmacs` --- the REPL *is* the
    //     process-spawning subsystem. The bundled package's manifest
    //     declares process access by virtue of being shipped with
    //     the editor; once the manifest gains a formal
    //     `permissions` field (post-M7), the audit engine will
    //     verify the declaration and stop emitting this warning.
    //
    // Any new warning class outside this rationale fails the
    // acceptance --- a real surprise should surface as a
    // classification request in M11.
    let permitted_warnings = ["no-process-spawn-pmacs"];
    for f in findings.iter().filter(|f| f.severity == Severity::Warning) {
        assert!(
            permitted_warnings.contains(&f.rule.as_str()),
            "migrated REPL warning `{}` is not classified; \
             add a rationale to tests/m7_11_acceptance.rs or fix the source.\nfinding: {f:?}",
            f.rule
        );
    }
}

// ---------------------------------------------------------------------------
// Sanity check on the bundled-package machinery itself
// ---------------------------------------------------------------------------

#[test]
fn bundled_packages_table_includes_repl() {
    let names: Vec<&str> = BUNDLED_PACKAGES.iter().map(|p| p.name).collect();
    assert!(
        names.contains(&"repl"),
        "BUNDLED_PACKAGES must include `repl`; got {names:?}"
    );
}

#[test]
fn materialize_all_produces_one_record_per_bundled_package() {
    let tmp = std::env::temp_dir().join(format!(
        "pmacs-builtin-m7_11-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let pkgs = materialize_all(&tmp).expect("materialize_all");
    assert_eq!(pkgs.len(), BUNDLED_PACKAGES.len());
    let repl = pkgs
        .iter()
        .find(|p| p.manifest.name.as_str() == "repl")
        .expect("repl record");
    assert_eq!(repl.tag, "bundled");
    assert!(repl.entry_path().exists());
    let _ = std::fs::remove_dir_all(&tmp);
}
