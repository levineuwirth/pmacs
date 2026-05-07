// builtin_packages.rs --- Bundled packages materialized at editor startup.

//! Bundled packages (T M7.11).
//!
//! pmacs ships a small set of first-party packages alongside the
//! editor binary. Through M7.10, those bundled packages were loaded
//! directly via `lua_host.eval(include_str!(...))`, bypassing the
//! manifest / exports / per-package `_ENV` machinery the M7 package
//! system gives third-party packages. T M7.11 closes the gap: the
//! bundled REPL gets a real `pmacs.toml` manifest and is loaded
//! through the same path a `pmacs.packages.install` user package
//! would take.
//!
//! ## How a bundled package is delivered
//!
//! Each [`BundledPackage`] carries its manifest TOML and entry source
//! as `&'static str` references --- compiled into the binary via
//! [`include_str!`]. At startup, the editor calls
//! [`materialize_all`] to write each package to a per-version
//! directory under the user's XDG data dir (see
//! [`bundled_runtime_dir`] for the resolution policy), then
//! constructs an
//! [`InstalledPackage`] record and pushes it onto the
//! [`InstalledPackages`](crate::lua_bindings::InstalledPackages)
//! roster. From that point the M7.7 searcher resolves
//! `require("repl")` (etc.) the same way it resolves any
//! user-installed package.
//!
//! ## Why not load embedded source directly
//!
//! The M7.7 loader reads chunk bytes from `install_path` via
//! [`std::fs::read`]. Routing the bundled chunk through that same
//! disk-read keeps the load path uniform: one mechanism, one
//! per-package `_ENV` setup, one exports check. Adding a parallel
//! "embedded source" code path would split that uniformity and
//! mean every loader-side change would need two implementations.
//!
//! ## Where the files land
//!
//! [`bundled_runtime_dir`] resolves to
//! `$XDG_DATA_HOME/pmacs/builtin-packages/v<crate-version>/`,
//! falling back to `$HOME/.local/share/pmacs/builtin-packages/v<v>/`
//! and then to a per-user, per-process subdir of the OS temp dir if
//! neither HOME nor `XDG_DATA_HOME` is set. The data-dir choice is
//! deliberate: the previous version used `$TMPDIR` directly, which
//! on a multi-user host is world-writable and exposed bundled-package
//! materialization to a shared-tempdir race. Sitting under the user's
//! own data-dir path puts the bundled tree behind the same ownership
//! and mode boundary as user-installed packages.
//!
//! The version-stamped subdir means two side-by-side pmacs versions
//! don't collide and a content-gated write skips redundant updates.
//! We do not delete the dir on editor shutdown: leaving the
//! materialized tree in place lets a later editor invocation reuse
//! the files without re-writing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use semver::{Version, VersionReq};

use crate::packages::{InstallPin, InstallScope, InstalledPackage, ManifestError, PackageManifest};

/// One bundled package's compile-time content.
#[derive(Debug, Clone, Copy)]
pub struct BundledPackage {
    /// Package basename. Must match `manifest.name` after parsing.
    pub name: &'static str,
    /// Embedded `pmacs.toml` source.
    pub manifest_toml: &'static str,
    /// Embedded `(relative-path, file-bytes)` table. The first
    /// entry's relative path must match the parsed manifest's
    /// `entry` field; subsequent entries are exported submodules.
    pub files: &'static [(&'static str, &'static str)],
}

/// All bundled packages shipped with the editor. Order is
/// presentation-only --- the loader looks them up by name.
pub const BUNDLED_PACKAGES: &[BundledPackage] = &[BundledPackage {
    name: "repl",
    manifest_toml: include_str!("../builtin/packages/repl/pmacs.toml"),
    files: &[(
        "init.lua",
        include_str!("../builtin/packages/repl/init.lua"),
    )],
}];

/// Errors raised during bundled-package materialization. None of
/// these are recoverable at runtime; the editor cannot start
/// without its bundled packages.
#[derive(Debug, thiserror::Error)]
pub enum BundledError {
    /// I/O failure writing or reading a bundled-package file.
    #[error("bundled-package I/O at {path}: {source}")]
    Io {
        /// Path the operation was attempted against.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// A bundled package's embedded manifest failed to parse.
    /// Compile-time content invariant: this should never fire in
    /// production, but the path is exercised so a malformed
    /// `pmacs.toml` in `builtin/packages/<name>/` fails loud.
    #[error("bundled manifest for `{name}` failed to parse: {source}")]
    Manifest {
        /// Package basename.
        name: String,
        /// Underlying parse error.
        #[source]
        source: ManifestError,
    },
    /// A bundled package's manifest's `name` field did not match
    /// the registry-table entry's `name` field. Indicates a typo in
    /// either side; surfaces at startup to fail fast.
    #[error(
        "bundled package name mismatch: registry says `{registry}`, manifest says `{manifest}`"
    )]
    NameMismatch {
        /// Registry entry's name.
        registry: String,
        /// Manifest's `name` field.
        manifest: String,
    },
}

/// Resolve the directory under which bundled-package trees are
/// materialized. Tries, in order:
///
/// 1. `$XDG_DATA_HOME/pmacs/builtin-packages/v<crate-version>/`
/// 2. `$HOME/.local/share/pmacs/builtin-packages/v<crate-version>/`
/// 3. A user-private fallback under `$TMPDIR`:
///    `$TMPDIR/pmacs-builtin-uid<U>-v<crate-version>/` (only used
///    when neither `XDG_DATA_HOME` nor `HOME` is set; the UID
///    suffix narrows the path to the calling user so a multi-user
///    box doesn't share the same world-writable directory).
#[must_use]
pub fn bundled_runtime_dir() -> PathBuf {
    let suffix = format!("v{}", env!("CARGO_PKG_VERSION"));
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        let p: PathBuf = d.into();
        if !p.as_os_str().is_empty() {
            return p.join("pmacs").join("builtin-packages").join(suffix);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p: PathBuf = home.into();
        if !p.as_os_str().is_empty() {
            return p
                .join(".local")
                .join("share")
                .join("pmacs")
                .join("builtin-packages")
                .join(suffix);
        }
    }
    // Headless / sandboxed environment with neither HOME nor
    // XDG_DATA_HOME (CI containers, build daemons). Scope the temp
    // path to the calling user to avoid world-writable shared paths.
    let uid = nix::unistd::Uid::current().as_raw();
    std::env::temp_dir().join(format!("pmacs-builtin-uid{uid}-{suffix}"))
}

/// Materialize every entry in [`BUNDLED_PACKAGES`] under `parent`,
/// returning one [`InstalledPackage`] per package in registry
/// order. Idempotent: per-file content gate skips writes when the
/// existing bytes already match. Each package lands at
/// `parent/<name>/`.
///
/// `parent` must already exist (or be createable); this function
/// creates it on demand.
pub fn materialize_all(parent: &Path) -> Result<Vec<InstalledPackage>, BundledError> {
    fs::create_dir_all(parent).map_err(|source| BundledError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut out = Vec::with_capacity(BUNDLED_PACKAGES.len());
    for bp in BUNDLED_PACKAGES {
        out.push(materialize_one(parent, bp)?);
    }
    Ok(out)
}

fn materialize_one(parent: &Path, bp: &BundledPackage) -> Result<InstalledPackage, BundledError> {
    let install_path = parent.join(bp.name);
    fs::create_dir_all(&install_path).map_err(|source| BundledError::Io {
        path: install_path.clone(),
        source,
    })?;

    // Manifest first --- write it to disk in addition to parsing it,
    // so a third-party tool inspecting the bundled tree on disk sees
    // a fully-formed package directory.
    let manifest_path = install_path.join("pmacs.toml");
    write_if_changed(&manifest_path, bp.manifest_toml.as_bytes())?;
    let manifest =
        PackageManifest::from_toml(bp.manifest_toml).map_err(|source| BundledError::Manifest {
            name: bp.name.to_string(),
            source,
        })?;
    if manifest.name.as_str() != bp.name {
        return Err(BundledError::NameMismatch {
            registry: bp.name.to_string(),
            manifest: manifest.name.as_str().to_string(),
        });
    }

    // Then every other file the package declared. Subdirs are
    // created on demand so an export at `submod/init.lua` works.
    for (rel, body) in bp.files {
        let p = install_path.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).map_err(|source| BundledError::Io {
                path: d.to_path_buf(),
                source,
            })?;
        }
        write_if_changed(&p, body.as_bytes())?;
    }

    // Build the synthetic InstalledPackage record. The version /
    // pin / tag fields use the manifest's declared version verbatim
    // --- bundled packages don't have a Git history to draw a tag
    // and commit from, so we synthesize stable values that the rest
    // of the system treats consistently:
    //
    //   * `commit`   --- 40-char placeholder; the bundled package's
    //                    "identity" is its source bytes, which the
    //                    on-disk path captures.
    //   * `tag`      --- "bundled" (descriptive label visible to the
    //                    Lua introspection surface).
    //   * `pin`      --- exact-version pin matching the manifest.
    //   * `scope`    --- `User`, since the bundled tree shares the
    //                    same lookup precedence as user-installed
    //                    packages. (Adding a `Bundled` scope variant
    //                    would propagate through several enums; the
    //                    `tag = "bundled"` field already lets Lua
    //                    callers tell the two apart.)
    let version: Version = manifest.version.clone();
    let pin = InstallPin::Version(VersionReq::parse(&format!("={version}")).unwrap_or(
        // exact-version syntax should always parse; fall back to
        // any-match if a future semver-crate version regresses.
        VersionReq::STAR,
    ));
    Ok(InstalledPackage {
        manifest,
        install_path,
        commit: "0".repeat(40),
        tag: "bundled".to_string(),
        version,
        scope: InstallScope::User,
        pin,
    })
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), BundledError> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) | Err(_) => {}
    }
    fs::write(path, bytes).map_err(|source| BundledError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_repl_manifest_parses() {
        let m = BUNDLED_PACKAGES
            .iter()
            .find(|p| p.name == "repl")
            .expect("repl bundled");
        let parsed = PackageManifest::from_toml(m.manifest_toml).expect("repl manifest parses");
        assert_eq!(parsed.name.as_str(), "repl");
        assert_eq!(parsed.entry.to_str().unwrap(), "init.lua");
        assert!(parsed.exports.iter().any(|e| e == "repl"));
    }

    #[test]
    fn materialize_all_writes_files_and_returns_installed_packages() {
        let tmp = std::env::temp_dir().join(format!(
            "pmacs-builtin-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let pkgs = materialize_all(&tmp).expect("materialize");
        assert_eq!(pkgs.len(), BUNDLED_PACKAGES.len());

        let repl = pkgs
            .iter()
            .find(|p| p.manifest.name.as_str() == "repl")
            .expect("repl present");
        assert!(repl.entry_path().exists(), "repl entry on disk");
        assert!(
            repl.install_path.join("pmacs.toml").exists(),
            "repl manifest on disk"
        );
        assert_eq!(repl.tag, "bundled");

        // Idempotent re-materialization.
        let pkgs2 = materialize_all(&tmp).expect("re-materialize");
        assert_eq!(pkgs2.len(), pkgs.len());

        // Best-effort cleanup; failure is non-fatal.
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_if_changed_skips_when_bytes_match() {
        let tmp = std::env::temp_dir().join(format!(
            "pmacs-builtin-test-write-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&tmp).unwrap();
        let p = tmp.join("x");
        fs::write(&p, b"v1").unwrap();
        let mtime1 = fs::metadata(&p).unwrap().modified().ok();
        // small sleep to ensure the mtime would advance if we did write again
        std::thread::sleep(std::time::Duration::from_millis(15));
        write_if_changed(&p, b"v1").unwrap();
        let mtime2 = fs::metadata(&p).unwrap().modified().ok();
        assert_eq!(mtime1, mtime2, "matching bytes should not retrigger write");

        write_if_changed(&p, b"v2").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"v2");

        let _ = fs::remove_dir_all(&tmp);
    }
}
