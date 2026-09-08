// packages/loader.rs --- Require-name resolution against installed packages (T M7.7).

//! Loader (T M7.7, spec §sec:packages-future).
//!
//! Pure-Rust resolution layer that maps a Lua `require` name (e.g.
//! `"pmacs-magit"`, `"pmacs-magit.commit"`) to a concrete on-disk file
//! path within an installed package's tree, gated by the manifest's
//! `PackageManifest::exports` list.
//!
//! # Why a separate module
//!
//! The Lua-side searcher (in `lua_bindings.rs`) is short and stitches
//! mlua APIs together: load a chunk, set its environment, return it.
//! The decision logic — "given a require name and an installed
//! package roster, what file should we load (if any) and what error
//! should we emit (if not)?" — is plain Rust. Splitting it out lets
//! us unit-test every branch (entry, exported submodule, non-exported
//! submodule, package-not-installed, missing file) without standing
//! up a Lua state per case.
//!
//! # Resolution rules
//!
//! Let the require name split into `(head, tail)` on the first `.`:
//!
//! - `head == ""` → invalid require, fall through.
//! - No package with `install_basename() == head` → [`LookupOutcome::NotInstalled`].
//!   The Lua searcher returns a "not found" string; subsequent
//!   searchers (path-based, etc.) get a turn.
//! - `tail.is_empty()` (i.e. require name == basename, no dot) →
//!   [`LookupOutcome::EntryModule`] pointing at `manifest.entry`. The
//!   entry is **always** loadable; whether the basename appears in
//!   `exports` is the package author's choice but not enforced here.
//! - `tail` non-empty:
//!   - If the full require name (`head.tail`) is in `manifest.exports`
//!     → [`LookupOutcome::ExportedModule`] pointing at the conventional
//!     Lua-style path: `<install_path>/<tail with `.` → `/`>.lua`,
//!     falling back to `<install_path>/<tail with `.` → `/`>/init.lua`
//!     when the first form does not exist.
//!   - Otherwise → [`LookupOutcome::NotExported`] carrying the
//!     package name and the full sorted exports list, so the Lua
//!     searcher can produce a self-contained error message.
//!
//! Multiple installed packages can share a basename (a project-scope
//! install of the same name as a user-scope install). The Lua
//! searcher iterates the roster reverse-most-recent-first; this
//! module operates on whichever single [`InstalledPackage`] the
//! caller hands in, so collision policy is the searcher's concern,
//! not ours.

use std::path::PathBuf;

use super::installer::InstalledPackage;

/// Outcome of looking up a `require` name against one installed
/// package.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LookupOutcome {
    /// The require name matches the package's entry module. The
    /// caller loads `entry_path` and sets the package's environment.
    EntryModule {
        /// Absolute path to the entry file (`install_path` joined
        /// with `manifest.entry`).
        entry_path: PathBuf,
    },
    /// The require name matches a submodule listed in `exports`.
    ExportedModule {
        /// Absolute path to the resolved file (one of `<dir>/x.lua`
        /// or `<dir>/x/init.lua` based on which exists; if neither
        /// exists at lookup time the caller surfaces the missing
        /// file as a load error).
        file_path: PathBuf,
        /// Which file form was selected. `Direct` for `<dir>/x.lua`,
        /// `InitDir` for `<dir>/x/init.lua`. `MissingBoth` is
        /// surfaced when neither file exists; the searcher converts
        /// this into a require error naming the package and the
        /// submodule.
        kind: ResolvedKind,
    },
    /// The require name's head matches the package basename, but the
    /// full name is not declared in the package's `exports`. The
    /// caller raises a require error with the available exports.
    NotExported {
        /// Package basename.
        package: String,
        /// Full require name as the user wrote it.
        requested: String,
        /// Sorted, deduped exports list for the error message.
        exports: Vec<String>,
    },
    /// No installed package matches the require name's head. The
    /// caller falls through to the next searcher (typically the
    /// path-based one).
    NotInstalled,
}

/// Which file form an exported-module lookup matched.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ResolvedKind {
    /// `<install_path>/<rel>.lua`.
    Direct,
    /// `<install_path>/<rel>/init.lua`.
    InitDir,
    /// Neither form exists; caller surfaces a missing-file error.
    MissingBoth,
}

/// Look up a `require` name against a single installed package.
///
/// `pkg.install_basename()` is the head this function compares against
/// the require name's first segment. See module docs for the full
/// resolution table.
#[must_use]
pub fn lookup_in_package(name: &str, pkg: &InstalledPackage) -> LookupOutcome {
    let basename = pkg.install_basename();
    let (head, tail) = match name.split_once('.') {
        Some((h, t)) => (h, t),
        None => (name, ""),
    };
    if head != basename {
        return LookupOutcome::NotInstalled;
    }

    if tail.is_empty() {
        // Bare basename require → entry module. Always allowed; the
        // entry is the package's "main" interface and listing it in
        // exports is permitted but not required.
        return LookupOutcome::EntryModule {
            entry_path: pkg.entry_path(),
        };
    }

    // Submodule require. Must be in exports.
    if !pkg.manifest.exports.iter().any(|e| e == name) {
        let mut exports = pkg.manifest.exports.clone();
        exports.sort();
        exports.dedup();
        return LookupOutcome::NotExported {
            package: basename.to_string(),
            requested: name.to_string(),
            exports,
        };
    }

    // Resolve `tail` (which may itself contain dots) to a relative
    // path under install_path.
    let rel = tail.replace('.', "/");
    let direct = pkg.install_path.join(format!("{rel}.lua"));
    let init_dir = pkg.install_path.join(&rel).join("init.lua");

    let kind = if direct.is_file() {
        ResolvedKind::Direct
    } else if init_dir.is_file() {
        ResolvedKind::InitDir
    } else {
        ResolvedKind::MissingBoth
    };
    let file_path = match kind {
        ResolvedKind::Direct | ResolvedKind::MissingBoth => direct,
        ResolvedKind::InitDir => init_dir,
    };
    LookupOutcome::ExportedModule { file_path, kind }
}

/// Walk the installed-packages list in reverse order (most-recent
/// first) and return the first match. Mirrors the Lua searcher's
/// "newer install wins on collision" semantics.
#[must_use]
pub fn lookup_in_roster(name: &str, roster: &[InstalledPackage]) -> LookupOutcome {
    for pkg in roster.iter().rev() {
        let outcome = lookup_in_package(name, pkg);
        if !matches!(outcome, LookupOutcome::NotInstalled) {
            return outcome;
        }
    }
    LookupOutcome::NotInstalled
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packages::{InstallPin, InstallScope, PackageManifest, PackageName};
    use semver::{Version, VersionReq};
    use std::path::Path;
    use tempfile::TempDir;

    fn manifest(name: &str, exports: &[&str]) -> PackageManifest {
        PackageManifest {
            name: PackageName::new(name).unwrap(),
            version: Version::new(1, 0, 0),
            summary: "test".into(),
            pmacs_required: VersionReq::parse(">= 0.1.0").unwrap(),
            dependencies: vec![],
            conflicts: vec![],
            entry: PathBuf::from("init.lua"),
            exports: exports.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn make_installed(
        name: &str,
        exports: &[&str],
        files: &[(&str, &str)],
    ) -> (TempDir, InstalledPackage) {
        let td = tempfile::tempdir().unwrap();
        let install_path = td.path().join(name);
        std::fs::create_dir_all(&install_path).unwrap();
        for (rel, body) in files {
            let p = install_path.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, body).unwrap();
        }
        let pkg = InstalledPackage {
            manifest: manifest(name, exports),
            install_path,
            commit: "0".repeat(40),
            tag: "v1.0.0".into(),
            version: Version::new(1, 0, 0),
            scope: InstallScope::User,
            pin: InstallPin::Version(VersionReq::parse("^1.0.0").unwrap()),
        };
        (td, pkg)
    }

    #[test]
    fn entry_lookup_for_bare_basename() {
        let (_td, pkg) = make_installed("samplepkg", &[], &[("init.lua", "return {}")]);
        match lookup_in_package("samplepkg", &pkg) {
            LookupOutcome::EntryModule { entry_path } => {
                assert!(
                    entry_path.ends_with("samplepkg/init.lua"),
                    "got {entry_path:?}",
                );
            }
            other => panic!("expected EntryModule, got {other:?}"),
        }
    }

    #[test]
    fn exported_submodule_resolves_to_direct_file() {
        let (_td, pkg) = make_installed(
            "magit",
            &["magit.commit"],
            &[("init.lua", "return {}"), ("commit.lua", "return {}")],
        );
        match lookup_in_package("magit.commit", &pkg) {
            LookupOutcome::ExportedModule { file_path, kind } => {
                assert_eq!(kind, ResolvedKind::Direct);
                assert!(file_path.ends_with("magit/commit.lua"), "got {file_path:?}");
            }
            other => panic!("expected ExportedModule, got {other:?}"),
        }
    }

    #[test]
    fn exported_submodule_falls_back_to_init_dir() {
        let (_td, pkg) = make_installed(
            "magit",
            &["magit.commit"],
            &[("init.lua", "return {}"), ("commit/init.lua", "return {}")],
        );
        match lookup_in_package("magit.commit", &pkg) {
            LookupOutcome::ExportedModule { file_path, kind } => {
                assert_eq!(kind, ResolvedKind::InitDir);
                assert!(
                    file_path.ends_with("magit/commit/init.lua"),
                    "got {file_path:?}",
                );
            }
            other => panic!("expected ExportedModule, got {other:?}"),
        }
    }

    #[test]
    fn exported_submodule_with_neither_file_returns_missing_both() {
        let (_td, pkg) = make_installed("magit", &["magit.commit"], &[("init.lua", "return {}")]);
        match lookup_in_package("magit.commit", &pkg) {
            LookupOutcome::ExportedModule { kind, .. } => {
                assert_eq!(kind, ResolvedKind::MissingBoth);
            }
            other => panic!("expected ExportedModule, got {other:?}"),
        }
    }

    #[test]
    fn non_exported_submodule_returns_not_exported_with_list() {
        let (_td, pkg) = make_installed(
            "magit",
            &["magit.commit", "magit.diff"],
            &[("init.lua", "return {}"), ("internal.lua", "return {}")],
        );
        match lookup_in_package("magit.internal", &pkg) {
            LookupOutcome::NotExported {
                package,
                requested,
                exports,
            } => {
                assert_eq!(package, "magit");
                assert_eq!(requested, "magit.internal");
                assert_eq!(exports, vec!["magit.commit", "magit.diff"]);
            }
            other => panic!("expected NotExported, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_require_name_returns_not_installed() {
        let (_td, pkg) = make_installed("magit", &[], &[("init.lua", "return {}")]);
        assert_eq!(
            lookup_in_package("totally-different", &pkg),
            LookupOutcome::NotInstalled,
        );
    }

    #[test]
    fn deeper_dotted_path_resolves_with_slashes() {
        let (_td, pkg) = make_installed(
            "magit",
            &["magit.diff.hunk"],
            &[("init.lua", "return {}"), ("diff/hunk.lua", "return {}")],
        );
        match lookup_in_package("magit.diff.hunk", &pkg) {
            LookupOutcome::ExportedModule { file_path, kind } => {
                assert_eq!(kind, ResolvedKind::Direct);
                assert!(
                    file_path.ends_with("magit/diff/hunk.lua"),
                    "got {file_path:?}",
                );
            }
            other => panic!("expected ExportedModule, got {other:?}"),
        }
    }

    #[test]
    fn roster_lookup_picks_most_recent_on_basename_collision() {
        // Two installs with the same basename; the second one should
        // win the lookup (mirrors the Lua searcher's project-scope-
        // overrides-user-scope semantics).
        let (_td_a, mut pkg_a) = make_installed("samplepkg", &[], &[("init.lua", "return 'a'")]);
        pkg_a.scope = InstallScope::User;
        let (_td_b, mut pkg_b) = make_installed("samplepkg", &[], &[("init.lua", "return 'b'")]);
        pkg_b.scope = InstallScope::Project {
            project_root: PathBuf::from("/tmp/proj"),
        };
        let _ = (Path::new("a"), Path::new("b"));

        let roster = vec![pkg_a.clone(), pkg_b.clone()];
        match lookup_in_roster("samplepkg", &roster) {
            LookupOutcome::EntryModule { entry_path } => {
                assert_eq!(entry_path, pkg_b.entry_path());
            }
            other => panic!("expected EntryModule, got {other:?}"),
        }
    }
}
