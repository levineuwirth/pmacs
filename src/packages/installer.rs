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
//! - **Standalone vs resolver-driven**. The installer has two
//!   entry points. [`Installer::install`] is standalone: it
//!   independently picks a tag for [`InstallPin::Version`] via
//!   [`best_match`] and refuses to overwrite an existing install
//!   at a different commit. [`Installer::install_at_commit`] /
//!   [`Installer::replace_at_commit`] are the resolver-driven
//!   paths: they trust the supplied commit and (for replace)
//!   stage-and-swap an existing install. The Lua surface
//!   (`pmacs.packages.install` / `update`) goes through the
//!   resolver-driven paths so the resolver's revision choice is
//!   authoritative; transitive resolution and lockfile writes
//!   live in the surrounding orchestration code.
//! - **All three pin kinds supported**. [`InstallPin::Version`]
//!   maps to `best_match` over upstream tags;
//!   [`InstallPin::Branch`] resolves to the named branch's HEAD;
//!   [`InstallPin::Commit`] resolves to a specific revision. The
//!   user surface accepts all three via the table form (`{ ...,
//!   branch = ... }` / `{ ..., commit = ... }`).
//! - **Install dir naming**: `<install_root>/<basename>/`, where
//!   `basename` is the package name's last `/`-segment. Two
//!   packages with the same basename collide on disk; for v0.1 we
//!   accept the collision (the resolver / caller can
//!   `pmacs.packages.installed()` to spot conflicts before they
//!   bite).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum InstallPin {
    /// Highest semver tag satisfying the constraint.
    Version(VersionReq),
    /// HEAD of the named branch at install time.
    Branch(String),
    /// Specific commit (full or partial SHA; the fetcher accepts
    /// either via `git rev-parse`).
    Commit(String),
    /// Working-tree symlink installed via
    /// [`pmacs.packages.install_local`] (T M8.1c). Carries the
    /// source path so the Lua-visible roster entry can show where
    /// the live tree lives. Local-pinned packages are *ephemeral*:
    /// they never enter the lockfile and aren't reproducible across
    /// machines --- they exist for the M8 dev loop ("edit a
    /// package's source on disk and reload without restarting") and
    /// nothing else.
    Local {
        /// Source path the install dir symlinks to.
        source_path: PathBuf,
    },
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
            Self::Local { .. } => "local",
        }
    }

    /// User-supplied value as a string: the constraint for
    /// [`Self::Version`], the branch name for [`Self::Branch`], the
    /// SHA for [`Self::Commit`], the source path for [`Self::Local`].
    #[must_use]
    pub fn value(&self) -> String {
        match self {
            Self::Version(req) => req.to_string(),
            Self::Branch(b) => b.clone(),
            Self::Commit(c) => c.clone(),
            Self::Local { source_path } => source_path.display().to_string(),
        }
    }
}

/// Result of [`Installer::plan_local`]: everything needed to
/// commit a working-tree symlink install, with no disk changes
/// yet performed (T M8.1c).
///
/// The plan/commit split lets the Lua binding layer interleave
/// `on_unload`-hook execution between validation and the disk
/// swap. If a hook fails, the caller can drop the plan and the
/// disk is unchanged.
#[derive(Debug, Clone)]
pub struct LocalInstallPlan {
    /// Parsed manifest from `<source_path>/pmacs.toml`.
    pub manifest: PackageManifest,
    /// Where the symlink will be placed:
    /// `<install_root>/<basename>`.
    pub install_path: PathBuf,
    /// Canonicalized source path the symlink will point at. Holds
    /// an absolute path so the symlink resolves regardless of
    /// where the editor's CWD ends up.
    pub canonical_source: PathBuf,
    /// Install basename (the last `/`-segment of `manifest.name`).
    /// Useful to the binding layer for keying registry slots
    /// (`PackageUnloadHooks`, etc.) without re-deriving from the
    /// manifest.
    pub basename: String,
    /// Scope (user / project) this install will land in.
    pub scope: InstallScope,
}

/// A local install whose new symlink has already been created at a
/// sibling staging path, but has not yet been published over
/// [`LocalInstallPlan::install_path`].
///
/// The Lua binding uses this to front-load the fallible symlink
/// creation before it runs the prior package's `on_unload` hooks.
/// After hooks complete, publishing is a same-directory `rename(2)`.
#[derive(Debug, Clone)]
pub struct StagedLocalInstall {
    plan: LocalInstallPlan,
    staging_path: PathBuf,
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

/// Internal options passed through [`Installer::install_with`].
/// Public callers use [`Installer::install`] /
/// [`Installer::install_at_commit`] /
/// [`Installer::replace_at_commit`] which set these flags
/// appropriately.
#[derive(Debug, Default)]
struct InstallOptions {
    /// If `Some`, treat this commit as the resolver's choice and
    /// skip the installer's own tag/branch/commit lookup. The
    /// manifest is read at this commit.
    resolved_commit: Option<String>,
    /// If `true`, an existing install at the same path with a
    /// different commit is replaced rather than rejected with
    /// [`InstallError::AlreadyInstalled`]. The replacement is
    /// staged at `<install_path>.new` and only swapped in after
    /// successful extraction.
    replace_existing: bool,
}

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
    ///
    /// `install()` is the standalone entry point: the installer
    /// independently picks the tag for `InstallPin::Version` via
    /// [`best_match`]. Resolver-driven flows
    /// (`pmacs.packages.install` / `pmacs.packages.update`) instead
    /// call [`Self::install_at_commit`] / [`Self::replace_at_commit`]
    /// so the installer honors the resolver's revision choice rather
    /// than re-deriving it.
    pub fn install(&self, spec: &InstallSpec) -> Result<InstalledPackage, InstallError> {
        self.install_with(spec, &InstallOptions::default())
    }

    /// Install at a commit pre-chosen by the resolver. The displayed
    /// `pin` field on the returned [`InstalledPackage`] still
    /// reflects `spec.pin` (so a `Version(^1.0.0)` request shows up
    /// as a version pin), but the installer skips its own tag
    /// enumeration and checks out the supplied commit directly.
    /// The displayed `tag` is synthesized from `manifest.version` for
    /// Version pins, or kept as `branch:<name>`/`commit:<short>` for
    /// the other variants.
    ///
    /// Refuses to overwrite an existing install at a different
    /// commit; for that path see [`Self::replace_at_commit`].
    pub fn install_at_commit(
        &self,
        spec: &InstallSpec,
        commit: &str,
    ) -> Result<InstalledPackage, InstallError> {
        self.install_with(
            spec,
            &InstallOptions {
                resolved_commit: Some(commit.to_string()),
                replace_existing: false,
            },
        )
    }

    /// Install or replace at a commit pre-chosen by the resolver.
    /// Differs from [`Self::install_at_commit`] only in that an
    /// existing install at the same path with a different commit is
    /// replaced rather than erroring. The replacement is staged at
    /// `<install_path>.new` and only swapped in after extraction
    /// succeeds, so a failing update leaves the prior install intact.
    ///
    /// Used by `pmacs.packages.update`, which by definition expects
    /// to overwrite a prior install when upstream has moved.
    pub fn replace_at_commit(
        &self,
        spec: &InstallSpec,
        commit: &str,
    ) -> Result<InstalledPackage, InstallError> {
        self.install_with(
            spec,
            &InstallOptions {
                resolved_commit: Some(commit.to_string()),
                replace_existing: true,
            },
        )
    }

    /// Install from a local working-tree path by symlinking it
    /// into the install root (T M8.1c). The dev-loop counterpart
    /// to [`Self::install`]: edits to files under `source_path`
    /// become live in the editor without re-running the package
    /// pipeline; `pmacs.packages.reload(name)` (M8.1d) picks them
    /// up without restarting the session.
    ///
    /// Semantics:
    ///
    /// - `source_path` must contain a readable `pmacs.toml`.
    ///   Anything else fails with [`InstallError::LocalManifestMissing`].
    /// - The install dir is `<install_root>/<basename>`. If a
    ///   symlink already lives there, it is replaced by staging a
    ///   sibling symlink and atomically renaming it into place.
    /// - If a *real* directory lives at the install path,
    ///   [`InstallError::LocalRealInstallInWay`] surfaces. The user
    ///   removes that install first (manually or via a future
    ///   uninstall API).
    /// - The returned [`InstalledPackage`] carries
    ///   [`InstallPin::Local`] so the Lua-visible roster entry
    ///   names the source. No lockfile work is done; `install_local`
    ///   is explicitly ephemeral.
    pub fn install_local(&self, source_path: &Path) -> Result<InstalledPackage, InstallError> {
        let plan = self.plan_local(source_path)?;
        self.commit_local(plan)
    }

    /// Validate `source_path` and compute where its symlink should
    /// land, **without making any disk changes**. The returned
    /// [`LocalInstallPlan`] is consumed by [`Self::commit_local`],
    /// which performs the symlink swap.
    ///
    /// The plan/commit split exists so the Lua binding layer can
    /// run prior-install `on_unload` hooks between the two steps:
    /// if any hook fails, the disk symlink hasn't moved, so disk
    /// and runtime state remain in sync. Without the split, a
    /// failing hook leaves the symlink at the new source while the
    /// roster / `package.loaded` / per-package env still track the
    /// old one --- a desync the user can only resolve by
    /// restarting.
    pub fn plan_local(&self, source_path: &Path) -> Result<LocalInstallPlan, InstallError> {
        // Manifest must exist and parse. A friendly error here
        // beats a surprising error later when the searcher tries
        // to load a non-existent entry.
        let manifest_path = source_path.join("pmacs.toml");
        let manifest_str = std::fs::read_to_string(&manifest_path).map_err(|e| {
            InstallError::LocalManifestMissing {
                source_path: source_path.to_path_buf(),
                cause: e.to_string(),
            }
        })?;
        let manifest = PackageManifest::from_toml(&manifest_str).map_err(|e| {
            InstallError::LocalManifestMissing {
                source_path: source_path.to_path_buf(),
                cause: e.to_string(),
            }
        })?;

        // pmacs_required check, identical to the fetched-install
        // path. install_local doesn't bypass any compatibility
        // gate; the dev-loop story doesn't extend to "ignore the
        // version constraint."
        let running_pmacs = running_pmacs_version();
        if !manifest.pmacs_required.matches(&running_pmacs) {
            return Err(InstallError::PmacsVersionIncompatible {
                address: source_path.display().to_string(),
                tag: format!("local:{}", source_path.display()),
                required: manifest.pmacs_required.to_string(),
                running: running_pmacs.to_string(),
            });
        }

        let install_root = self.install_root()?;
        let basename = package_basename(manifest.name.as_str()).to_string();
        let install_path = install_root.join(&basename);

        // Probe the install path. We don't mutate it here --- the
        // commit step does. We do reject the real-directory case
        // up front so the Lua binding layer can refuse before
        // running any unload hooks (a hook running and then the
        // commit failing because of a real-dir collision would be
        // worse than refusing immediately).
        match std::fs::symlink_metadata(&install_path) {
            Ok(meta) if meta.file_type().is_symlink() => { /* ok, we'll replace */ }
            Ok(_) => {
                return Err(InstallError::LocalRealInstallInWay { install_path });
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => { /* ok, we'll create */ }
            Err(source) => {
                return Err(InstallError::Io {
                    path: install_path.clone(),
                    source,
                });
            }
        }

        // Canonicalize the source so the symlink points at an
        // absolute path. Without this, a relative source resolves
        // against the install dir's parent rather than the user's
        // CWD, and the user's CWD is the contract here.
        let canonical_source =
            std::fs::canonicalize(source_path).map_err(|source| InstallError::Io {
                path: source_path.to_path_buf(),
                source,
            })?;

        Ok(LocalInstallPlan {
            manifest,
            install_path,
            canonical_source,
            basename,
            scope: self.scope.clone(),
        })
    }

    /// Commit a [`LocalInstallPlan`] in one step: stage a new symlink
    /// at a sibling temp path, then atomically rename it over
    /// `plan.install_path`. After this returns, the plan's bytes are
    /// live on disk; the caller is responsible for cache invalidation.
    ///
    /// Callers that need to interleave package teardown hooks between
    /// staging and publishing should use [`Self::stage_local`] followed
    /// by [`Self::publish_local`].
    ///
    /// **Atomicity.** The replacement uses `rename(2)` to swap the
    /// staged symlink over the existing one. On the same
    /// filesystem (which it is by construction --- the staging
    /// path is in the same directory as `install_path`),
    /// `rename(2)` is atomic with respect to other observers: at
    /// any moment, `install_path` either holds the old symlink or
    /// the new one, never neither. This is the upgrade from the
    /// prior remove-then-create shape, where a `symlink(2)` failure
    /// after the `unlink(2)` left the install path missing while
    /// the runtime still tracked the old install.
    ///
    /// Re-checks the install path's symlink-vs-real-dir state at
    /// commit time: belt-and-braces against a TOCTOU between plan
    /// and commit (the dev-loop is single-user, so a real
    /// race is unlikely, but a `LocalRealInstallInWay` returned
    /// here keeps the contract symmetric with [`Self::plan_local`]).
    pub fn commit_local(&self, plan: LocalInstallPlan) -> Result<InstalledPackage, InstallError> {
        let staged = self.stage_local(plan)?;
        self.publish_local(staged)
    }

    /// Stage a [`LocalInstallPlan`] by creating the new symlink at a
    /// hidden sibling path, but do not publish it over the live
    /// install path yet.
    ///
    /// This performs the fallible symlink-creation work before the
    /// binding layer runs `on_unload` hooks. If staging fails, the old
    /// package is still live and no teardown hooks have fired.
    pub fn stage_local(&self, plan: LocalInstallPlan) -> Result<StagedLocalInstall, InstallError> {
        // Re-check the install path. A real dir surfacing here
        // would indicate either a TOCTOU race or a bug in the
        // plan/commit caller; either way refuse rather than
        // silently overwrite. Symlinks and missing paths are both
        // valid commit destinations; the atomic rename below
        // handles both shapes uniformly.
        match std::fs::symlink_metadata(&plan.install_path) {
            Ok(meta) if meta.file_type().is_symlink() => { /* ok, atomic swap */ }
            Ok(_) => {
                return Err(InstallError::LocalRealInstallInWay {
                    install_path: plan.install_path,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => { /* ok, fresh create */ }
            Err(source) => {
                return Err(InstallError::Io {
                    path: plan.install_path.clone(),
                    source,
                });
            }
        }

        // Stage the new symlink at a sibling path. Same directory
        // as the install path means rename(2) is atomic. The
        // sentinel-prefix (`.<basename>.swap.tmp`) is hidden in
        // ls(1) output and namespaced so concurrent commits for
        // different basenames don't collide. A leftover from a
        // prior crashed commit would be unlinked here before we
        // re-stage.
        let staging_path = plan
            .install_path
            .with_file_name(format!(".{}.swap.tmp", plan.basename));
        if let Err(e) = std::fs::remove_file(&staging_path) {
            if e.kind() != io::ErrorKind::NotFound {
                return Err(InstallError::Io {
                    path: staging_path,
                    source: e,
                });
            }
        }
        symlink_create(&plan.canonical_source, &staging_path)?;

        Ok(StagedLocalInstall { plan, staging_path })
    }

    /// Best-effort cleanup for a staged local install that will not be
    /// published, typically because an `on_unload` hook failed.
    pub fn discard_staged_local(&self, staged: StagedLocalInstall) {
        let _ = std::fs::remove_file(staged.staging_path);
    }

    /// Publish a staged local install with a same-directory atomic
    /// rename, returning the Lua-visible package record.
    pub fn publish_local(
        &self,
        staged: StagedLocalInstall,
    ) -> Result<InstalledPackage, InstallError> {
        let StagedLocalInstall { plan, staging_path } = staged;

        // Atomic swap. rename(2) replaces install_path
        // (whether or not it currently exists) in a single
        // observable step. On failure the staged symlink is
        // unlinked so we don't leave a dangling .swap.tmp file
        // behind; the original install_path is untouched.
        if let Err(source) = std::fs::rename(&staging_path, &plan.install_path) {
            let _ = std::fs::remove_file(&staging_path);
            return Err(InstallError::Io {
                path: plan.install_path.clone(),
                source,
            });
        }

        Ok(InstalledPackage {
            version: plan.manifest.version.clone(),
            manifest: plan.manifest,
            install_path: plan.install_path,
            // No commit. The synthetic `local` token marks this as
            // an ephemeral install in the Lua-visible roster
            // (callers compare the `pin.kind` field, not commit).
            commit: "local".to_string(),
            tag: format!("local:{}", plan.canonical_source.display()),
            scope: plan.scope,
            pin: InstallPin::Local {
                source_path: plan.canonical_source,
            },
        })
    }

    /// Unified install flow used by [`Self::install`],
    /// [`Self::install_at_commit`], and [`Self::replace_at_commit`].
    /// The three differ only in `opts`.
    #[allow(clippy::too_many_lines)]
    fn install_with(
        &self,
        spec: &InstallSpec,
        opts: &InstallOptions,
    ) -> Result<InstalledPackage, InstallError> {
        // Reject Local pins early: the fetched-install path needs a
        // clone URL, and Local pins don't have one. install_local()
        // owns the working-tree symlink path. T M8.1c.
        if let InstallPin::Local { source_path } = &spec.pin {
            return Err(InstallError::LocalPinNotSupported {
                source_path: source_path.clone(),
            });
        }
        let url = spec.address.to_git_url();
        let bare = self.fetcher.fetch(&url).map_err(InstallError::Fetch)?;

        // Resolve the user's pin to a concrete (commit, tag-descriptor)
        // pair. When the resolver has supplied a commit, we use it
        // directly: this keeps the installer aligned with the
        // resolver's choice for InstallPin::Version (where re-running
        // best_match() could otherwise diverge if upstream tagged a
        // newer version that the resolver rejected for compatibility
        // reasons). The descriptor for Version pins is synthesized
        // from manifest.version after we read the manifest.
        let (commit, tag_descriptor) = if let Some(forced) = opts.resolved_commit.as_deref() {
            let commit = self
                .fetcher
                .resolve(&bare, &RefSpec::Commit(forced.to_string()))
                .map_err(InstallError::Fetch)?;
            let descriptor = match &spec.pin {
                // Replaced post-manifest-read below.
                InstallPin::Version(_) => String::new(),
                InstallPin::Branch(name) => format!("branch:{name}"),
                InstallPin::Commit(_) => {
                    let short = commit.get(..7).unwrap_or(commit.as_str()).to_string();
                    format!("commit:{short}")
                }
                InstallPin::Local { .. } => {
                    unreachable!("Local pins refused at install_with entry")
                }
            };
            (commit, descriptor)
        } else {
            match &spec.pin {
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
                InstallPin::Local { .. } => {
                    unreachable!("Local pins refused at install_with entry")
                }
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

        // Synthesize the Version-pin descriptor now that we know the
        // manifest version. Mirrors the conventional `v{X.Y.Z}` form
        // produced by the standalone tag-matching path.
        let tag_descriptor =
            if opts.resolved_commit.is_some() && matches!(spec.pin, InstallPin::Version(_)) {
                format!("v{}", manifest.version)
            } else {
                tag_descriptor
            };

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

        // If the install path already exists with the same commit,
        // treat as idempotent. With a different commit, behavior
        // depends on `opts.replace_existing`: the standalone install
        // path refuses (`pmacs.packages.install` callers reach this
        // when re-running install with a moved upstream and should
        // be told to use `update`); the resolver-driven update path
        // proceeds to a staged replacement.
        let mut needs_replace = false;
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
                _ if opts.replace_existing => {
                    needs_replace = true;
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

        // Staging path: when replacing, extract to a sibling dir and
        // only rename into place after success, so a failing replace
        // leaves the prior install untouched. Same-filesystem rename
        // makes the swap visible atomically; a crash between removing
        // the old dir and renaming the staged dir leaves the staged
        // dir in place, which is recoverable on next run.
        let extract_target = if needs_replace {
            let staged = install_path.with_extension("new");
            // A leftover staging dir from a prior crash would
            // confuse `create_dir_all` semantics; clear it first.
            if staged.exists() {
                fs::remove_dir_all(&staged).map_err(|source| InstallError::Io {
                    path: staged.clone(),
                    source,
                })?;
            }
            staged
        } else {
            install_path.clone()
        };

        fs::create_dir_all(&extract_target).map_err(|source| InstallError::Io {
            path: extract_target.clone(),
            source,
        })?;
        if let Err(e) = extract_tar(&archive, &extract_target) {
            // Roll back partial extraction. For the staging path
            // this leaves the prior install untouched; for the
            // direct path this leaves the install root clean.
            let _ = fs::remove_dir_all(&extract_target);
            return Err(e);
        }
        write_install_marker(&extract_target, &commit)?;

        if needs_replace {
            // Swap with rollback: rename the old install aside,
            // rename the staged dir into place, then remove the
            // backup. If the second rename fails, restore from the
            // backup so the prior install survives the failed
            // update. The two renames are individually atomic on the
            // same filesystem; the only window where neither dir
            // sits at `install_path` is between them, and a crash in
            // that window leaves both `.old` and `.new` siblings
            // for manual recovery.
            let backup = install_path.with_extension("old");
            // Clear any leftover backup from a prior crash.
            if backup.exists() {
                fs::remove_dir_all(&backup).map_err(|source| InstallError::Io {
                    path: backup.clone(),
                    source,
                })?;
            }
            fs::rename(&install_path, &backup).map_err(|source| InstallError::Io {
                path: install_path.clone(),
                source,
            })?;
            if let Err(source) = fs::rename(&extract_target, &install_path) {
                // Restore. If even the restore fails, surface the
                // original error --- the operator now needs to
                // manually swap `<path>.old` back into place, but
                // we've at least preserved the bytes.
                let _ = fs::rename(&backup, &install_path);
                return Err(InstallError::Io {
                    path: install_path.clone(),
                    source,
                });
            }
            // Both renames succeeded --- safe to drop the backup.
            // A failure here leaves `<path>.old` behind (best-
            // effort): the new install is correct on disk, just a
            // disk-space leak.
            let _ = fs::remove_dir_all(&backup);
        }

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

/// Strip a `<owner>/` namespace prefix from a manifest name and
/// return the trailing segment used for the on-disk install dir
/// and for `require()` lookup. `"magit"` → `"magit"`,
/// `"user/magit"` → `"magit"`. The `pub(crate)` exposure lets
/// `lua_bindings::do_update` derive the basename for a lockfile
/// entry without re-implementing the rule.
pub(crate) fn package_basename(name: &str) -> &str {
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

/// Create a symlink at `link` pointing at `target`. Unix-only in
/// v0.1; pmacs doesn't ship Windows builds and `std::os::unix`'s
/// symlink semantics are what dired/wdired need (the link is the
/// thing being managed; the target is data).
fn symlink_create(target: &Path, link: &Path) -> Result<(), InstallError> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).map_err(|source| InstallError::Io {
            path: link.to_path_buf(),
            source,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (target, link);
        Err(InstallError::Io {
            path: link.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::Unsupported,
                "install_local requires Unix symlink support",
            ),
        })
    }
}

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
    /// [`Installer::install`] / [`Installer::install_at_commit`] /
    /// [`Installer::replace_at_commit`] received an
    /// [`InstallPin::Local`]. Local pins must go through
    /// [`Installer::install_local`] (T M8.1c); routing them to the
    /// fetched-install path would require a clone URL that doesn't
    /// exist for working-tree installs.
    #[error(
        "InstallPin::Local cannot be installed via the fetched-install path; \
         use Installer::install_local for source path `{source_path}`"
    )]
    LocalPinNotSupported {
        /// The source path the Local pin named.
        source_path: PathBuf,
    },
    /// [`Installer::install_local`] was given a path that doesn't
    /// contain a readable `pmacs.toml`. The package layout
    /// requirements are documented in the package author guide; the
    /// user typically forgot to write the manifest or pointed at
    /// the wrong directory.
    #[error("install_local: no readable pmacs.toml at `{source_path}`: {cause}")]
    LocalManifestMissing {
        /// The source path the user passed.
        source_path: PathBuf,
        /// The underlying I/O or parse error message.
        cause: String,
    },
    /// [`Installer::install_local`] was asked to install at a name
    /// that already has a real (non-symlink) install. The user must
    /// uninstall the fetched copy first. We refuse rather than
    /// silently replace because losing a fetched-install tree is a
    /// real risk (it might contain manual edits the user made
    /// before discovering `install_local`).
    #[error(
        "install_local: `{install_path}` is a real install, not a symlink; \
         remove it first, then re-run install_local"
    )]
    LocalRealInstallInWay {
        /// The install dir that's blocking the new symlink.
        install_path: PathBuf,
    },
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
