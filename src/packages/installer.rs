// packages/installer.rs --- Synchronous package installer.

//! Package installer (T M7.3, spec §sec:packages-future).
//!
//! Given an [`InstallSpec`] (address + version constraint) and an
//! install root, performs:
//!
//! 1. **Fetch** via [`Fetcher::fetch`] (mirror clone or update).
//! 2. **Tag enumeration** via [`Fetcher::list_tags`] and best-match
//!    selection against the user's [`VersionReq`].
//! 3. **Resolve** the chosen tag to a 40-char commit hash.
//! 4. **Manifest read** via [`Fetcher::show_blob`] so we can derive the
//!    install directory name from `manifest.name`'s basename without
//!    materializing the whole tree first.
//! 5. **Archive + extract** via [`Fetcher::archive_commit`] piped into
//!    `tar -x`. The install dir is self-contained; no `.git` clutter,
//!    no link back to the bare cache.
//!
//! v0.1 design notes:
//!
//! - **Synchronous**: errors surface back at the call site (typically
//!   a Lua traceback inside `pmacs.packages.install{...}`). The
//!   deferred-dispatcher pattern used for `pmacs.attach` does not
//!   apply --- attach is a transport handoff, install is just I/O,
//!   and a synchronous failure pins the offending `init.lua` line.
//! - **No transitive resolution**: each spec is resolved and
//!   installed independently. Dependency closure / lockfile come in
//!   T M7.5 / T M7.6.
//! - **Tag-only resolution**: we pick the highest-numbered semver tag
//!   that satisfies the constraint. Branch / commit pinning is the
//!   resolver's job in M7.5; for v0.1 the install API only takes
//!   `version` constraints, which by definition target tags.
//! - **Install dir naming**: `<install_root>/<basename>/`, where
//!   `basename` is the package name's last `/`-segment. Two installs
//!   with the same basename collide on disk; for v0.1 we accept the
//!   collision (the caller can `pmacs.packages.installed()` to spot
//!   conflicts before they bite). Proper handling lands with the
//!   M7.5 resolver.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use semver::{Version, VersionReq};
use thiserror::Error;

use super::address::{Address, AddressError};
use super::fetcher::{FetchError, Fetcher, RefSpec};
use super::manifest::{ManifestError, PackageManifest};

// ---------------------------------------------------------------------------
// InstallScope
// ---------------------------------------------------------------------------

/// Where a package should be installed.
///
/// `User` resolves to `$XDG_DATA_HOME/pmacs/packages/` (or
/// `$HOME/.local/share/pmacs/packages/` if `XDG_DATA_HOME` is unset).
/// `Project` resolves to `<project_root>/.pmacs/packages/`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum InstallScope {
    /// User-config install (per-user, persistent across projects).
    User,
    /// Project-scoped install. The project root is conventionally the
    /// editor's working directory at startup.
    Project {
        /// Project root directory; the install dir is `<root>/.pmacs/packages/`.
        project_root: PathBuf,
    },
}

impl InstallScope {
    /// Resolve to the install root directory. Does not create it.
    pub fn resolve_root(&self) -> Result<PathBuf, InstallError> {
        match self {
            Self::User => xdg_data_root(),
            Self::Project { project_root } => Ok(project_root.join(".pmacs").join("packages")),
        }
    }
}

fn xdg_data_root() -> Result<PathBuf, InstallError> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        let p: PathBuf = dir.into();
        if !p.as_os_str().is_empty() {
            return Ok(p.join("pmacs").join("packages"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p: PathBuf = home.into();
        if !p.as_os_str().is_empty() {
            return Ok(p
                .join(".local")
                .join("share")
                .join("pmacs")
                .join("packages"));
        }
    }
    Err(InstallError::NoDataHome)
}

// ---------------------------------------------------------------------------
// InstallSpec
// ---------------------------------------------------------------------------

/// What the user pinned the install to. Mutually exclusive on the
/// Lua side: a spec table may carry exactly one of `version`,
/// `branch`, or `commit`.
///
/// # Why three kinds
///
/// - [`Self::Version`] is the default, recommended path: the
///   installer picks the highest semver tag matching the constraint
///   and validates the manifest's declared version against the same
///   constraint. Lockfile reproduction (M7.6) records the resolved
///   commit so a later install at the same constraint yields the
///   same revision.
/// - [`Self::Branch`] follows a moving target. Each install
///   re-resolves the branch's HEAD; the install is *not*
///   reproducible across time. Useful for development against an
///   upstream's `main` or for a private package whose semver
///   discipline is not yet established.
/// - [`Self::Commit`] freezes the install at a specific revision.
///   Useful for pinning to a known-good state before the upstream
///   has tagged a release, or for reproducing a colleague's
///   environment exactly without semver drift.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum InstallPin {
    /// Highest semver tag satisfying the constraint.
    Version(VersionReq),
    /// HEAD of the named branch at install time.
    Branch(String),
    /// Specific commit (full or partial SHA; the fetcher accepts
    /// either via `git rev-parse`).
    Commit(String),
}

impl InstallPin {
    /// Stable string discriminator used at the Lua boundary
    /// (`installed_package_to_lua`) and in error messages.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Version(_) => "version",
            Self::Branch(_) => "branch",
            Self::Commit(_) => "commit",
        }
    }

    /// User-supplied value as a string: the constraint for
    /// [`Self::Version`], the branch name for [`Self::Branch`], the
    /// SHA for [`Self::Commit`].
    #[must_use]
    pub fn value(&self) -> String {
        match self {
            Self::Version(req) => req.to_string(),
            Self::Branch(b) => b.clone(),
            Self::Commit(c) => c.clone(),
        }
    }
}

/// A normalized install request: where to fetch and how to pin the
/// revision.
#[derive(Debug, Clone)]
pub struct InstallSpec {
    /// Address (resolved via `Address::parse`).
    pub address: Address,
    /// What the user pinned the install to. See [`InstallPin`].
    pub pin: InstallPin,
}

impl InstallSpec {
    /// Parse the `<address>@<constraint>` shorthand. Examples:
    ///
    /// - `github:user/repo@^1.0.0`
    /// - `git:https://x.y/z.git@=2.3.4`
    /// - `https://x.y/z.git@~0.5`
    ///
    /// The `@` separator is always the **last** `@` in the string so
    /// addresses containing an `@` (SSH shorthand: `git@host:path`) work.
    /// Returns [`InstallError::ShorthandMissingVersion`] when the input
    /// has no `@` separator and [`InstallError::Address`] /
    /// [`InstallError::InvalidVersionReq`] for the underlying parse
    /// failures.
    ///
    /// The shorthand string form is **version-pin only**. Branch and
    /// commit pins must use the Lua table form
    /// (`{ "addr", branch = "..." }` / `{ "addr", commit = "..." }`)
    /// because there is no concise sigil that disambiguates a
    /// branch/commit value from a semver constraint without
    /// surprising users (`@main` could be a branch named "main" or
    /// a malformed semver --- ambiguous).
    pub fn parse_shorthand(s: &str) -> Result<Self, InstallError> {
        let (addr, ver) =
            s.rsplit_once('@')
                .ok_or_else(|| InstallError::ShorthandMissingVersion {
                    input: s.to_string(),
                })?;
        if addr.is_empty() || ver.is_empty() {
            return Err(InstallError::ShorthandMissingVersion {
                input: s.to_string(),
            });
        }
        let address = Address::parse(addr).map_err(InstallError::Address)?;
        let version = VersionReq::parse(ver).map_err(|e| InstallError::InvalidVersionReq {
            value: ver.to_string(),
            cause: e.to_string(),
        })?;
        Ok(Self {
            address,
            pin: InstallPin::Version(version),
        })
    }
}

// ---------------------------------------------------------------------------
// InstalledPackage
// ---------------------------------------------------------------------------

/// Result of a successful install.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    /// Parsed `pmacs.toml` from the installed snapshot.
    pub manifest: PackageManifest,
    /// On-disk directory holding the installed snapshot.
    pub install_path: PathBuf,
    /// 40-char commit hash of the installed snapshot.
    pub commit: String,
    /// A descriptor of what was installed:
    /// - For [`InstallPin::Version`]: the matched tag, e.g. `"v1.0.0"`.
    /// - For [`InstallPin::Branch`]: `"branch:<name>"`.
    /// - For [`InstallPin::Commit`]: `"commit:<short-sha>"`.
    ///
    /// Always non-empty so Lua callers can use it as a stable
    /// "what got installed" label without nil-checking.
    pub tag: String,
    /// Semver version of the installed snapshot. For
    /// [`InstallPin::Version`] this is the version parsed from the
    /// matched tag; for [`InstallPin::Branch`] / [`InstallPin::Commit`]
    /// it falls back to `manifest.version` (the package's declared
    /// version at the resolved revision).
    pub version: Version,
    /// The install scope this package was installed under.
    pub scope: InstallScope,
    /// What the user originally pinned this install to. Useful for
    /// lockfile generation (M7.6) and for surfacing to the Lua
    /// `installed()` snapshot.
    pub pin: InstallPin,
}

impl InstalledPackage {
    /// The basename used for the on-disk install dir and for the
    /// Lua `require` name. Equal to `manifest.name`'s segment after
    /// the optional `/` namespace prefix.
    #[must_use]
    pub fn install_basename(&self) -> &str {
        package_basename(self.manifest.name.as_str())
    }

    /// Absolute path of the entry Lua module (`install_path` joined
    /// with `manifest.entry`).
    #[must_use]
    pub fn entry_path(&self) -> PathBuf {
        self.install_path.join(&self.manifest.entry)
    }
}

// ---------------------------------------------------------------------------
// Installer
// ---------------------------------------------------------------------------

/// Installer: pairs a [`Fetcher`] with an [`InstallScope`].
///
/// One `Installer` per scope; `LuaHost` constructs two (user-scoped and
/// project-scoped) at startup.
#[derive(Debug, Clone)]
pub struct Installer {
    fetcher: Fetcher,
    scope: InstallScope,
    /// Test override for the install root. Production callers leave
    /// this `None`; the test harness sets it (the project forbids
    /// `unsafe_code`, so mutating `XDG_DATA_HOME` directly is not an
    /// option).
    root_override: Option<PathBuf>,
}

impl Installer {
    /// Construct an installer with an explicit fetcher and scope.
    #[must_use]
    pub fn new(fetcher: Fetcher, scope: InstallScope) -> Self {
        Self {
            fetcher,
            scope,
            root_override: None,
        }
    }

    /// Builder: override the on-disk install root (test-facing).
    /// Production code does not call this; the resolved
    /// [`InstallScope`] determines the path.
    #[must_use]
    pub fn with_install_root_override(mut self, root: PathBuf) -> Self {
        self.root_override = Some(root);
        self
    }

    /// The scope this installer targets.
    #[must_use]
    pub fn scope(&self) -> &InstallScope {
        &self.scope
    }

    /// Resolve the install root (creating it on demand).
    pub fn install_root(&self) -> Result<PathBuf, InstallError> {
        let root = match &self.root_override {
            Some(p) => p.clone(),
            None => self.scope.resolve_root()?,
        };
        fs::create_dir_all(&root).map_err(|source| InstallError::Io {
            path: root.clone(),
            source,
        })?;
        Ok(root)
    }

    /// Install one package. See module docs for the step-by-step flow.
    #[allow(clippy::too_many_lines)]
    pub fn install(&self, spec: &InstallSpec) -> Result<InstalledPackage, InstallError> {
        let url = spec.address.to_git_url();
        let bare = self.fetcher.fetch(&url).map_err(InstallError::Fetch)?;

        // Resolve the user's pin to a concrete (commit, tag-descriptor)
        // pair. The descriptor is what we display to users in the
        // `tag` field of the resulting `InstalledPackage`.
        let (commit, tag_descriptor) = match &spec.pin {
            InstallPin::Version(req) => {
                let tags = self.fetcher.list_tags(&bare).map_err(InstallError::Fetch)?;
                let chosen =
                    best_match(&tags, req).ok_or_else(|| InstallError::NoMatchingVersion {
                        address: url.clone(),
                        req: req.to_string(),
                        available: tags.clone(),
                    })?;
                let commit = self
                    .fetcher
                    .resolve(&bare, &RefSpec::Tag(chosen.tag.clone()))
                    .map_err(InstallError::Fetch)?;
                (commit, chosen.tag)
            }
            InstallPin::Branch(name) => {
                let commit = self
                    .fetcher
                    .resolve(&bare, &RefSpec::Branch(name.clone()))
                    .map_err(InstallError::Fetch)?;
                (commit, format!("branch:{name}"))
            }
            InstallPin::Commit(sha) => {
                let commit = self
                    .fetcher
                    .resolve(&bare, &RefSpec::Commit(sha.clone()))
                    .map_err(InstallError::Fetch)?;
                let short = commit.get(..7).unwrap_or(commit.as_str()).to_string();
                (commit, format!("commit:{short}"))
            }
        };

        // Read the manifest at this commit so we know the install dir name.
        let manifest_bytes = self
            .fetcher
            .show_blob(&bare, &commit, "pmacs.toml")
            .map_err(|e| match e {
                FetchError::GitInvocation { stderr, .. } => InstallError::ManifestMissing {
                    address: url.clone(),
                    tag: tag_descriptor.clone(),
                    cause: stderr,
                },
                other => InstallError::Fetch(other),
            })?;
        let manifest_str =
            std::str::from_utf8(&manifest_bytes).map_err(|_| InstallError::ManifestNotUtf8 {
                address: url.clone(),
                tag: tag_descriptor.clone(),
            })?;
        let manifest = PackageManifest::from_toml(manifest_str).map_err(InstallError::Manifest)?;

        // Refuse to install a package whose `pmacs_required` constraint
        // does not match the running pmacs version. Applies to every
        // pin kind: a package's declared API requirements are
        // independent of how the user pinned the revision.
        let running_pmacs = running_pmacs_version();
        if !manifest.pmacs_required.matches(&running_pmacs) {
            return Err(InstallError::PmacsVersionIncompatible {
                address: url.clone(),
                tag: tag_descriptor.clone(),
                required: manifest.pmacs_required.to_string(),
                running: running_pmacs.to_string(),
            });
        }

        // For version pins only: cross-check that the manifest's
        // declared version satisfies the constraint. The matched tag
        // already satisfies it (we chose it that way); the strict
        // check is on the manifest, which catches packages whose tag
        // and pmacs.toml version disagree. Branch and commit pins
        // skip this check --- the user explicitly asked for that
        // revision regardless of what the manifest says.
        if let InstallPin::Version(req) = &spec.pin {
            if !req.matches(&manifest.version) {
                return Err(InstallError::ManifestVersionMismatch {
                    address: url.clone(),
                    tag: tag_descriptor.clone(),
                    manifest_version: manifest.version.to_string(),
                    req: req.to_string(),
                });
            }
        }

        // Archive + extract.
        let install_root = self.install_root()?;
        let basename = package_basename(manifest.name.as_str());
        let install_path = install_root.join(basename);

        // If the install path already exists with the same commit, treat
        // as idempotent. With a different commit, we refuse rather than
        // overwrite --- callers ask for `update` (M7.6), not silent
        // replacement.
        if install_path.exists() {
            let existing = read_install_marker(&install_path).ok();
            match existing {
                Some(prev) if prev == commit => {
                    return Ok(InstalledPackage {
                        version: manifest.version.clone(),
                        manifest,
                        install_path,
                        commit,
                        tag: tag_descriptor,
                        scope: self.scope.clone(),
                        pin: spec.pin.clone(),
                    });
                }
                _ => {
                    return Err(InstallError::AlreadyInstalled {
                        path: install_path,
                        existing_commit: existing,
                        requested_commit: commit,
                    });
                }
            }
        }

        let archive = self
            .fetcher
            .archive_commit(&bare, &commit)
            .map_err(InstallError::Fetch)?;
        fs::create_dir_all(&install_path).map_err(|source| InstallError::Io {
            path: install_path.clone(),
            source,
        })?;
        if let Err(e) = extract_tar(&archive, &install_path) {
            // Roll back partial extraction: an empty install dir is more
            // recoverable than a half-populated one.
            let _ = fs::remove_dir_all(&install_path);
            return Err(e);
        }
        write_install_marker(&install_path, &commit)?;

        Ok(InstalledPackage {
            version: manifest.version.clone(),
            manifest,
            install_path,
            commit,
            tag: tag_descriptor,
            scope: self.scope.clone(),
            pin: spec.pin.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tag selection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TagMatch {
    tag: String,
    version: Version,
}

/// Pick the highest-numbered tag satisfying `req`.
///
/// Tags are filtered through [`parse_tag_as_semver`]: tags that don't
/// parse as semver are silently skipped (release notes, internal markers,
/// etc.). Tags with a leading `v` are accepted (the `v1.2.3` convention
/// is universal); the returned `tag` field preserves the original form.
fn best_match(tags: &[String], req: &VersionReq) -> Option<TagMatch> {
    let mut best: Option<TagMatch> = None;
    for tag in tags {
        let Some(version) = parse_tag_as_semver(tag) else {
            continue;
        };
        if !req.matches(&version) {
            continue;
        }
        match &best {
            None => {
                best = Some(TagMatch {
                    tag: tag.clone(),
                    version,
                });
            }
            Some(current) if version > current.version => {
                best = Some(TagMatch {
                    tag: tag.clone(),
                    version,
                });
            }
            _ => {}
        }
    }
    best
}

fn parse_tag_as_semver(tag: &str) -> Option<Version> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(stripped).ok()
}

fn package_basename(name: &str) -> &str {
    match name.rsplit_once('/') {
        Some((_, last)) => last,
        None => name,
    }
}

// ---------------------------------------------------------------------------
// tar extraction
// ---------------------------------------------------------------------------

/// Pipe `archive` into `tar -xC <dest>` and wait for completion.
///
/// `tar` is invoked with explicit options so it works the same on GNU
/// tar and BSD tar (macOS): `-x` extracts, `-f -` reads from stdin,
/// `-C <dest>` changes directory before extracting.
fn extract_tar(archive: &[u8], dest: &Path) -> Result<(), InstallError> {
    let mut cmd = Command::new("tar");
    cmd.arg("-x").arg("-f").arg("-").arg("-C").arg(dest);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => InstallError::TarNotFound,
        _ => InstallError::TarSpawn { source: e },
    })?;
    {
        let mut stdin = child
            .stdin
            .take()
            .expect("tar stdin handle present after Stdio::piped");
        stdin
            .write_all(archive)
            .map_err(|source| InstallError::TarSpawn { source })?;
        // stdin drops here, closing the pipe so tar can finish.
    }
    let output = child
        .wait_with_output()
        .map_err(|source| InstallError::TarSpawn { source })?;
    if !output.status.success() {
        return Err(InstallError::TarFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Install marker
// ---------------------------------------------------------------------------

const MARKER_NAME: &str = ".pmacs-install";

fn write_install_marker(install_path: &Path, commit: &str) -> Result<(), InstallError> {
    let p = install_path.join(MARKER_NAME);
    fs::write(&p, format!("{commit}\n")).map_err(|source| InstallError::Io { path: p, source })
}

fn read_install_marker(install_path: &Path) -> Result<String, io::Error> {
    let p = install_path.join(MARKER_NAME);
    let s = fs::read_to_string(&p)?;
    Ok(s.trim().to_string())
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors raised during package install.
#[derive(Debug, Error)]
pub enum InstallError {
    /// `$XDG_DATA_HOME` and `$HOME` were both unset.
    #[error("cannot resolve XDG data directory: HOME and XDG_DATA_HOME are both unset")]
    NoDataHome,
    /// Underlying fetch/clone/resolve operation failed.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// Address parsing failed (typically only via `parse_shorthand`).
    #[error(transparent)]
    Address(AddressError),
    /// Manifest parsing failed.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Generic I/O error during install.
    #[error("install I/O error at `{path}`: {source}")]
    Io {
        /// The path being acted on.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// `pmacs.packages.install("addr@constraint")` lacked the `@`
    /// separator.
    #[error(
        "missing `@<version>` in shorthand `{input}`; \
         expected e.g. `github:user/repo@^1.0.0`"
    )]
    ShorthandMissingVersion {
        /// The offending input.
        input: String,
    },
    /// `parse_shorthand` got an `@<value>` that isn't a valid `VersionReq`.
    #[error("invalid version constraint `{value}`: {cause}")]
    InvalidVersionReq {
        /// The offending constraint string.
        value: String,
        /// The semver crate's parse-error message.
        cause: String,
    },
    /// No tag satisfied the user's [`VersionReq`].
    #[error(
        "no tag for `{address}` satisfies `{req}`. \
         Available tags: {available:?}"
    )]
    NoMatchingVersion {
        /// The clone URL the search ran against.
        address: String,
        /// The user's constraint, formatted for humans.
        req: String,
        /// The full tag list (filtered by the caller's eyes, not by
        /// semver-parseability).
        available: Vec<String>,
    },
    /// A repo was fetched and the tag resolved, but no `pmacs.toml`
    /// existed at that commit.
    #[error(
        "no pmacs.toml at tag `{tag}` of `{address}` \
         (this may not be a pmacs package): {cause}"
    )]
    ManifestMissing {
        /// The clone URL the search ran against.
        address: String,
        /// The tag the install would have used.
        tag: String,
        /// The git stderr message.
        cause: String,
    },
    /// `pmacs.toml` at the commit wasn't UTF-8.
    #[error("pmacs.toml at tag `{tag}` of `{address}` is not valid UTF-8")]
    ManifestNotUtf8 {
        /// The clone URL.
        address: String,
        /// The tag that was being installed.
        tag: String,
    },
    /// The manifest's `version` doesn't satisfy the user's constraint
    /// even though the tag did. Indicates a packaging bug upstream
    /// (tag and manifest disagree).
    #[error(
        "package at tag `{tag}` of `{address}` declares version \
         `{manifest_version}`, which does not satisfy `{req}` \
         (the upstream's tag and manifest disagree)"
    )]
    ManifestVersionMismatch {
        /// The clone URL.
        address: String,
        /// The tag that was matched.
        tag: String,
        /// The version declared in `pmacs.toml`.
        manifest_version: String,
        /// The user's constraint.
        req: String,
    },
    /// A different version of this package is already installed at the
    /// target path. Caller must `update` rather than `install`.
    #[error(
        "package already installed at `{path}` \
         (existing commit: {existing_commit:?}, requested: {requested_commit}); \
         use `pmacs.packages.update(\"...\")` to change versions"
    )]
    AlreadyInstalled {
        /// On-disk install dir.
        path: PathBuf,
        /// Commit hash recorded in the install marker, if readable.
        existing_commit: Option<String>,
        /// Commit hash that the new install would have used.
        requested_commit: String,
    },
    /// `tar` is not on PATH.
    #[error(
        "the `tar` binary was not found on PATH. \
         Pmacs's package install uses `git archive | tar -x` to \
         materialize a snapshot; install GNU tar or bsdtar."
    )]
    TarNotFound,
    /// `tar` failed to spawn or its stdin write failed.
    #[error("could not spawn or feed tar: {source}")]
    TarSpawn {
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// `tar` exited with a non-zero status.
    #[error("tar extraction failed: {stderr}")]
    TarFailed {
        /// Captured stderr from `tar`.
        stderr: String,
    },
    /// The package's `pmacs_required` constraint does not match the
    /// running pmacs version. The manifest field is a hard contract
    /// (the package may rely on APIs not present in this pmacs), so
    /// we refuse the install rather than letting the failure surface
    /// later as an opaque Lua runtime error.
    #[error(
        "package at tag `{tag}` of `{address}` requires pmacs `{required}`, \
         but this pmacs is `{running}`. \
         Upgrade pmacs, or pin a package version compatible with `{running}`."
    )]
    PmacsVersionIncompatible {
        /// The clone URL.
        address: String,
        /// The tag the install was about to use.
        tag: String,
        /// The constraint declared by the package's `pmacs_required`.
        required: String,
        /// The running pmacs version (`env!("CARGO_PKG_VERSION")`).
        running: String,
    },
}

// ---------------------------------------------------------------------------
// Running-pmacs-version probe
// ---------------------------------------------------------------------------

/// Return the running pmacs version as a [`semver::Version`].
///
/// The string comes from Cargo's `CARGO_PKG_VERSION` build-time env
/// var (so it tracks the workspace's `Cargo.toml` automatically).
/// `Cargo` enforces semver-shaped versions, so the parse cannot fail
/// in a release build; an `expect` here is a build-system invariant
/// guard, not user-reachable.
#[must_use]
pub fn running_pmacs_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("CARGO_PKG_VERSION must be valid semver (Cargo guarantees this)")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    // -- helpers ------------------------------------------------------------

    fn run_git(args: &[&OsStr], cwd: Option<&Path>) {
        let mut cmd = StdCommand::new("git");
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        for a in args {
            cmd.arg(a);
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("LC_ALL", "C");
        let out = cmd.output().unwrap_or_else(|e| panic!("git spawn: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Build a sample-package bare repo with two tagged versions and a
    /// `pmacs.toml` at each. Returns `(tempdir, bare_path)`.
    fn make_sample_package() -> (TempDir, PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let work = td.path().join("work");
        let bare = td.path().join("upstream.git");

        init_git_workdir(&work);
        commit_version(&work, "samplepkg", "1.0.0", "v1.0.0");
        commit_version(&work, "samplepkg", "1.1.0", "v1.1.0");
        clone_bare(&work, &bare);

        (td, bare)
    }

    fn init_git_workdir(work: &Path) {
        run_git(
            &[
                OsStr::new("init"),
                OsStr::new("--initial-branch=main"),
                work.as_os_str(),
            ],
            None,
        );
        run_git_at(work, &["config", "user.email", "test@example.com"]);
        run_git_at(work, &["config", "user.name", "Tester"]);
    }

    fn commit_version(work: &Path, name: &str, version: &str, tag: &str) {
        write_manifest(work, name, version);
        std::fs::write(
            work.join("init.lua"),
            format!("return {{ version = '{version}' }}\n"),
        )
        .unwrap();
        run_git_at(work, &["add", "."]);
        run_git_at(work, &["commit", "-m", tag]);
        run_git_at(work, &["tag", tag]);
    }

    fn clone_bare(work: &Path, bare: &Path) {
        run_git(
            &[
                OsStr::new("clone"),
                OsStr::new("--bare"),
                work.as_os_str(),
                bare.as_os_str(),
            ],
            None,
        );
    }

    /// `git -C <work> <args...>` shorthand: prepends `-C` and the work
    /// path so each call site stays one line.
    fn run_git_at(work: &Path, args: &[&str]) {
        let mut full: Vec<&OsStr> = vec![OsStr::new("-C"), work.as_os_str()];
        full.extend(args.iter().map(OsStr::new));
        run_git(&full, None);
    }

    fn write_manifest(work: &Path, name: &str, version: &str) {
        let toml = format!(
            r#"name = "{name}"
version = "{version}"
summary = "test package"
pmacs_required = ">= 0.1.0"
entry = "init.lua"
exports = ["samplepkg"]
"#
        );
        std::fs::write(work.join("pmacs.toml"), toml).unwrap();
    }

    fn file_url(p: &Path) -> String {
        format!("file://{}", p.display())
    }

    fn make_installer(install_root: &Path) -> (TempDir, Installer) {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let scope = InstallScope::Project {
            project_root: install_root.to_path_buf(),
        };
        let installer = Installer::new(fetcher, scope);
        (cache, installer)
    }

    // -- shorthand ----------------------------------------------------------

    #[test]
    fn shorthand_parses_github_address_with_caret_constraint() {
        let s = InstallSpec::parse_shorthand("github:user/repo@^1.0.0").unwrap();
        assert!(matches!(s.address, Address::Github { .. }));
        match &s.pin {
            InstallPin::Version(req) => assert_eq!(req.to_string(), "^1.0.0"),
            other => panic!("expected Version pin, got {other:?}"),
        }
    }

    #[test]
    fn shorthand_handles_address_containing_at_sign() {
        // SSH shorthand: `git:git@host:path`. The `@` in `git@host`
        // must not be confused with the version separator.
        let s = InstallSpec::parse_shorthand("git:git@host:path/repo.git@=1.2.3").unwrap();
        match &s.pin {
            InstallPin::Version(req) => assert_eq!(req.to_string(), "=1.2.3"),
            other => panic!("expected Version pin, got {other:?}"),
        }
        if let Address::Url(u) = s.address {
            assert_eq!(u, "git@host:path/repo.git");
        } else {
            panic!("expected Url, got {:?}", s.address);
        }
    }

    #[test]
    fn shorthand_without_at_separator_errors() {
        let err = InstallSpec::parse_shorthand("github:user/repo").unwrap_err();
        assert!(matches!(err, InstallError::ShorthandMissingVersion { .. }));
    }

    #[test]
    fn shorthand_rejects_empty_version_after_at() {
        let err = InstallSpec::parse_shorthand("github:user/repo@").unwrap_err();
        assert!(matches!(err, InstallError::ShorthandMissingVersion { .. }));
    }

    #[test]
    fn shorthand_rejects_invalid_constraint() {
        let err = InstallSpec::parse_shorthand("github:user/repo@not-a-version").unwrap_err();
        assert!(matches!(err, InstallError::InvalidVersionReq { .. }));
    }

    // -- best_match ---------------------------------------------------------

    #[test]
    fn best_match_picks_highest_satisfying_caret() {
        let tags = vec![
            "v0.9.0".into(),
            "v1.0.0".into(),
            "v1.1.0".into(),
            "v2.0.0".into(),
        ];
        let req = VersionReq::parse("^1.0").unwrap();
        let m = best_match(&tags, &req).unwrap();
        assert_eq!(m.tag, "v1.1.0");
    }

    #[test]
    fn best_match_skips_non_semver_tags() {
        let tags = vec![
            "release-notes".into(),
            "v1.0.0".into(),
            "milestone-1".into(),
        ];
        let req = VersionReq::parse("*").unwrap();
        let m = best_match(&tags, &req).unwrap();
        assert_eq!(m.tag, "v1.0.0");
    }

    #[test]
    fn best_match_returns_none_when_no_tag_satisfies() {
        let tags = vec!["v1.0.0".into(), "v1.1.0".into()];
        let req = VersionReq::parse(">=2.0").unwrap();
        assert!(best_match(&tags, &req).is_none());
    }

    #[test]
    fn best_match_accepts_unprefixed_tags() {
        let tags = vec!["1.0.0".into(), "1.1.0".into()];
        let req = VersionReq::parse("^1.0").unwrap();
        let m = best_match(&tags, &req).unwrap();
        assert_eq!(m.tag, "1.1.0");
    }

    // -- package_basename ---------------------------------------------------

    #[test]
    fn package_basename_unwraps_namespace() {
        assert_eq!(package_basename("magit"), "magit");
        assert_eq!(package_basename("user/magit"), "magit");
    }

    // -- end-to-end install -------------------------------------------------

    #[test]
    fn install_picks_highest_tag_and_extracts_to_install_root() {
        let (_pkg_td, bare) = make_sample_package();
        let install_td = tempfile::tempdir().unwrap();
        let (_cache_td, installer) = make_installer(install_td.path());

        let spec = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse("^1.0").unwrap()),
        };
        let installed = installer.install(&spec).unwrap();

        // Picked the higher tag (1.1.0 over 1.0.0).
        assert_eq!(installed.tag, "v1.1.0");
        assert_eq!(installed.version.to_string(), "1.1.0");

        // Install dir uses the package basename.
        assert!(
            installed
                .install_path
                .ends_with(".pmacs/packages/samplepkg")
        );

        // Entry module exists at the expected path.
        let entry = installed.entry_path();
        assert!(
            entry.exists(),
            "entry file `{}` should exist",
            entry.display()
        );
        let entry_text = std::fs::read_to_string(&entry).unwrap();
        assert!(entry_text.contains("1.1.0"));

        // Manifest snapshot is present (extraction included pmacs.toml).
        assert!(installed.install_path.join("pmacs.toml").exists());

        // Marker file records the commit.
        let marker = installed.install_path.join(".pmacs-install");
        let recorded = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(recorded.trim(), installed.commit);
    }

    #[test]
    fn install_resolves_exact_pin_to_lower_version() {
        let (_pkg_td, bare) = make_sample_package();
        let install_td = tempfile::tempdir().unwrap();
        let (_cache_td, installer) = make_installer(install_td.path());

        let spec = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse("=1.0.0").unwrap()),
        };
        let installed = installer.install(&spec).unwrap();
        assert_eq!(installed.tag, "v1.0.0");
        assert_eq!(installed.version.to_string(), "1.0.0");
    }

    #[test]
    fn install_no_matching_tag_errors_with_listing() {
        let (_pkg_td, bare) = make_sample_package();
        let install_td = tempfile::tempdir().unwrap();
        let (_cache_td, installer) = make_installer(install_td.path());

        let spec = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse(">=2.0.0").unwrap()),
        };
        let err = installer.install(&spec).unwrap_err();
        match err {
            InstallError::NoMatchingVersion { available, .. } => {
                assert!(available.iter().any(|t| t == "v1.0.0"));
                assert!(available.iter().any(|t| t == "v1.1.0"));
            }
            other => panic!("expected NoMatchingVersion, got {other:?}"),
        }
    }

    #[test]
    fn install_idempotent_for_same_commit() {
        let (_pkg_td, bare) = make_sample_package();
        let install_td = tempfile::tempdir().unwrap();
        let (_cache_td, installer) = make_installer(install_td.path());

        let spec = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse("=1.0.0").unwrap()),
        };
        let first = installer.install(&spec).unwrap();
        // Drop a sentinel; idempotent re-install should not blow it away.
        let sentinel = first.install_path.join("PMACS_TEST_SENTINEL");
        std::fs::write(&sentinel, b"x").unwrap();
        let second = installer.install(&spec).unwrap();
        assert_eq!(first.commit, second.commit);
        assert!(
            sentinel.exists(),
            "idempotent re-install must not re-extract"
        );
    }

    #[test]
    fn install_rejects_overwrite_when_commit_differs() {
        let (_pkg_td, bare) = make_sample_package();
        let install_td = tempfile::tempdir().unwrap();
        let (_cache_td, installer) = make_installer(install_td.path());

        // First install at 1.0.0.
        let spec_v1 = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse("=1.0.0").unwrap()),
        };
        installer.install(&spec_v1).unwrap();

        // Second install at 1.1.0 to the same install path: refuse.
        let spec_v2 = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse("=1.1.0").unwrap()),
        };
        let err = installer.install(&spec_v2).unwrap_err();
        assert!(matches!(err, InstallError::AlreadyInstalled { .. }));
    }

    #[test]
    fn project_and_user_scopes_have_distinct_install_roots() {
        // Sanity check: same Fetcher under two scopes resolves two
        // different roots, so user-config and project installs cannot
        // collide.
        let cache = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());

        let proj_inst = Installer::new(
            fetcher.clone(),
            InstallScope::Project {
                project_root: proj.path().to_path_buf(),
            },
        );
        let proj_root = proj_inst.scope.resolve_root().unwrap();
        assert!(proj_root.starts_with(proj.path()));

        // The user-scope path depends on the test's environment
        // (XDG_DATA_HOME / HOME). We just verify it differs from the
        // project root, not its specific layout.
        let user_inst = Installer::new(fetcher, InstallScope::User);
        if let Ok(user_root) = user_inst.scope.resolve_root() {
            assert_ne!(user_root, proj_root);
        }
    }

    /// Build a sample-package bare repo whose `pmacs_required` is
    /// pinned to `pmacs_required`. Used by the version-incompat test
    /// to mint a manifest the running pmacs cannot satisfy.
    fn make_pinned_pmacs_package(pmacs_required: &str) -> (TempDir, PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let work = td.path().join("work");
        let bare = td.path().join("upstream.git");

        init_git_workdir(&work);
        let toml = format!(
            r#"name = "samplepkg"
version = "1.0.0"
summary = "test package"
pmacs_required = "{pmacs_required}"
entry = "init.lua"
exports = ["samplepkg"]
"#
        );
        std::fs::write(work.join("pmacs.toml"), toml).unwrap();
        std::fs::write(work.join("init.lua"), "return { version = '1.0.0' }\n").unwrap();
        run_git_at(&work, &["add", "."]);
        run_git_at(&work, &["commit", "-m", "v1.0.0"]);
        run_git_at(&work, &["tag", "v1.0.0"]);
        clone_bare(&work, &bare);

        (td, bare)
    }

    #[test]
    fn install_rejects_unsatisfiable_pmacs_required() {
        // `pmacs_required = ">= 99.0.0"` cannot be satisfied by any
        // 0.x or 1.x pmacs build, so this is a stable
        // running-version-independent assertion: the install path
        // surfaces a typed `PmacsVersionIncompatible` error rather
        // than letting the package's API mismatch bite at runtime.
        let (_pkg, bare) = make_pinned_pmacs_package(">= 99.0.0");
        let install_td = tempfile::tempdir().unwrap();
        let (_cache, installer) = make_installer(install_td.path());

        let spec = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse("^1.0.0").unwrap()),
        };

        match installer.install(&spec).unwrap_err() {
            InstallError::PmacsVersionIncompatible {
                required, running, ..
            } => {
                assert!(
                    required.contains("99"),
                    "error must echo the failing constraint: required={required}"
                );
                assert_eq!(running, env!("CARGO_PKG_VERSION"));
            }
            other => panic!("expected PmacsVersionIncompatible, got {other:?}"),
        }
    }

    #[test]
    fn install_accepts_satisfiable_pmacs_required() {
        // The other side of the gate: a constraint that *is*
        // satisfied by the running build proceeds normally. Pin the
        // constraint to the running version exactly so the check is
        // tight (matches `=X.Y.Z`).
        let (_pkg, bare) = make_pinned_pmacs_package(&format!("={}", env!("CARGO_PKG_VERSION")));
        let install_td = tempfile::tempdir().unwrap();
        let (_cache, installer) = make_installer(install_td.path());

        let spec = InstallSpec {
            address: Address::Url(file_url(&bare)),
            pin: InstallPin::Version(VersionReq::parse("^1.0.0").unwrap()),
        };
        installer
            .install(&spec)
            .expect("matching pmacs_required must install");
    }

    #[test]
    fn running_pmacs_version_matches_cargo_pkg_version() {
        // Self-test for the helper. If Cargo ever changes the env-var
        // shape or someone introduces a non-semver suffix, this fires
        // before the install path does.
        let v = running_pmacs_version();
        assert_eq!(v.to_string(), env!("CARGO_PKG_VERSION"));
    }
}
