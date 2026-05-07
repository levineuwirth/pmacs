// packages/lockfile.rs --- Lockfile generation, parsing, verification (T M7.6).

//! Lockfile (T M7.6, spec §sec:packages-future).
//!
//! The lockfile (`pmacs.lock`) is the matching manifest-of-fact for a
//! [`ResolvePlan`]: exact commit hashes, resolved versions, transitive
//! closure, and per-commit content hashes. Two users with the same
//! lockfile install identical bytes; a tampered or hijacked Git host
//! produces a verifiable hash mismatch.
//!
//! # Wire format
//!
//! TOML, alphabetical-by-name `[[package]]` entries. Choosing
//! alphabetical (rather than topological) on disk: lockfile diffs in
//! source-control reviews stay readable when only one package's
//! version changes. Topological order, when needed, is recomputed by
//! consumers from the per-entry `dependencies` list.
//!
//! Schema version is `1`. Bumps are reserved for incompatible format
//! changes; additive fields can land without a bump if older readers
//! tolerate `#[serde(default)]`.
//!
//! # Content hashing
//!
//! The hash is SHA-256 over the bytes of `git archive --format=tar
//! <commit>`. `git archive` is deterministic for a given commit (it
//! emits canonical tar with sorted entries and the commit timestamp
//! as mtime), so the same commit always produces the same hash on any
//! machine. The hash is encoded `sha256:<64 lowercase hex>`.
//!
//! ## Why SHA-256 over `git archive`, not Git's own SHA-1
//!
//! Git's content addressing already verifies that a fetched commit's
//! tree matches the SHA-1 it was advertised as. But SHA-1 collisions
//! are computationally feasible in 2026; an attacker who controls the
//! upstream host can serve a different tree under the same SHA-1. The
//! spec's threat model is "tampered or hijacked Git host produces a
//! verifiable mismatch," which requires a stronger hash than SHA-1.
//! SHA-256 over the materialized tree closes that gap.
//!
//! ## Why archive bytes, not a recursive directory walk
//!
//! `git archive --format=tar` already canonicalizes (sorted entries,
//! file modes, paths, contents) and is part of every Git
//! installation. Reusing it costs one extra subprocess but avoids us
//! reimplementing canonical-tree hashing in Rust, which is a footgun
//! (mode handling, symlink targets, submodules — none of these have
//! obvious right-answers).
//!
//! # Lockfile-constrained resolution
//!
//! [`UpdatePolicy`] lets callers control how a fresh resolve interacts
//! with an existing lockfile:
//!
//! - [`UpdatePolicy::Frozen`] — no upstream re-resolution; the plan
//!   comes straight from the lockfile entries. Hash verification still
//!   runs at install time. This is the default "subsequent install"
//!   path.
//! - [`UpdatePolicy::UpdateAll`] — ignore the lockfile entirely;
//!   resolve fresh against current upstream state. Maps to
//!   `pmacs.packages.update()` (no argument).
//! - [`UpdatePolicy::UpdateOne(name)`] — re-resolve `name` fresh; for
//!   every other package, prefer the lockfile's recorded version. Maps
//!   to `pmacs.packages.update("name")`. The "prefer" semantic
//!   (rather than "force") means a constraint cascade triggered by
//!   `name`'s update can still bump unrelated packages — that's the
//!   user-visible behavior they expect from a partial-update.

// `LockfileError` carries diagnostic context (full hashes, URLs,
// commits, lists of participants) by design — the user-facing
// `Display` for `ContentHashMismatch` alone needs every field. Boxing
// the error to placate `result_large_err` would only move the cost
// without changing the user-visible surface, mirroring the same
// posture the resolver module takes for `ResolveError`.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::address::Address;
use super::fetcher::{FetchError, Fetcher, RefSpec};
use super::installer::InstallPin;
use super::manifest::PackageName;
use super::resolver::{ResolvePlan, ResolvedPackage};

/// Schema version emitted by [`Lockfile::to_bytes`]. Bumped only on
/// incompatible format changes.
pub const LOCKFILE_SCHEMA_VERSION: u32 = 1;

/// Default filename for the lockfile relative to a project root.
pub const LOCKFILE_FILENAME: &str = "pmacs.lock";

// ---------------------------------------------------------------------------
// Lockfile types
// ---------------------------------------------------------------------------

/// On-disk lockfile.
///
/// Construct from a [`ResolvePlan`] via [`Lockfile::from_plan`], parse
/// from bytes via [`Lockfile::parse`], emit deterministic bytes via
/// [`Lockfile::to_bytes`].
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Lockfile {
    /// Schema version. See [`LOCKFILE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Generator string, e.g. `"pmacs 0.1.0"`. Informational only;
    /// readers do not branch on it.
    pub generator: String,
    /// Package entries, sorted alphabetically by [`PackageName`].
    #[serde(rename = "package", default)]
    pub packages: Vec<LockfileEntry>,
}

/// One entry in a [`Lockfile`].
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LockfileEntry {
    /// Canonical package name from the resolved manifest.
    pub name: PackageName,
    /// Canonical Git clone URL (the result of [`Address::to_git_url`]
    /// at resolution time). Stored as a string so the lockfile is
    /// independent of any future `Address` enum-shape changes.
    pub url: String,
    /// 40-character commit hash. Always a real SHA, never a tag string.
    pub commit: String,
    /// Version parsed from the manifest at `commit`.
    pub version: Version,
    /// Content hash in `sha256:<hex>` form. Verified at install time.
    pub content_hash: ContentHash,
    /// `Some` for top-level packages, `None` for transitive deps.
    /// Tells [`UpdatePolicy::UpdateOne`] which lockfile entries
    /// originated from a top-level user request and which were pulled
    /// in transitively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_level_pin: Option<LockfilePin>,
    /// Immediate dependency names (sorted). The closure is implied by
    /// walking these edges across all entries.
    #[serde(default)]
    pub dependencies: Vec<PackageName>,
}

/// Mirror of [`InstallPin`] with a flat TOML representation.
///
/// Serializes as `{ kind = "version" | "branch" | "commit", value =
/// "<string>" }`. The wider [`InstallPin`] enum carries a parsed
/// [`VersionReq`] internally; the lockfile stores its `Display` form
/// so the file is independent of `semver` crate-version drift.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum LockfilePin {
    /// Version constraint, e.g. `"^1.0.0"`.
    Version(String),
    /// Branch name.
    Branch(String),
    /// Specific commit (the SHA the branch / commit pin originally
    /// resolved to).
    Commit(String),
}

impl LockfilePin {
    /// Convert from a runtime [`InstallPin`].
    ///
    /// Returns `None` for [`InstallPin::Local`]: `install_local`
    /// (T M8.1c) installs a working-tree symlink and is explicitly
    /// ephemeral --- it doesn't belong in the lockfile because the
    /// lockfile's contract is reproducibility across machines, and
    /// a symlink to `/srv/dev/pmacs-dired` doesn't reproduce
    /// anywhere else. Callers building lockfile entries skip the
    /// `top_level_pin` slot when this returns `None`; the entry
    /// itself isn't built at all because `install_local` doesn't
    /// go through the resolver.
    #[must_use]
    pub fn from_install_pin(pin: &InstallPin) -> Option<Self> {
        match pin {
            InstallPin::Version(req) => Some(Self::Version(req.to_string())),
            InstallPin::Branch(b) => Some(Self::Branch(b.clone())),
            InstallPin::Commit(c) => Some(Self::Commit(c.clone())),
            InstallPin::Local { .. } => None,
        }
    }

    /// Convert back to a runtime [`InstallPin`]. The stored
    /// `Version(_)` string is re-parsed; an unparseable value
    /// surfaces as [`LockfileError::InvalidVersionConstraint`].
    pub fn to_install_pin(&self) -> Result<InstallPin, LockfileError> {
        match self {
            Self::Version(s) => {
                let req = VersionReq::parse(s).map_err(|cause| {
                    LockfileError::InvalidVersionConstraint {
                        value: s.clone(),
                        cause: cause.to_string(),
                    }
                })?;
                Ok(InstallPin::Version(req))
            }
            Self::Branch(b) => Ok(InstallPin::Branch(b.clone())),
            Self::Commit(c) => Ok(InstallPin::Commit(c.clone())),
        }
    }
}

fn lockfile_save_error(path: &Path, err: crate::file_io::SaveError) -> LockfileError {
    let source = match err {
        crate::file_io::SaveError::Io(source) => source,
        crate::file_io::SaveError::NoParent(target) => io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "lockfile path has no parent directory: {}",
                target.display()
            ),
        ),
    };
    LockfileError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Content hash carried by every [`LockfileEntry`].
///
/// Serialized as a single string `<algorithm>:<hex>` (e.g.
/// `"sha256:abc..."`). The algorithm field exists for future-proofing
/// — v1.0 only emits and accepts `sha256`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ContentHash {
    /// Hash algorithm name. v1.0: always `"sha256"`.
    pub algorithm: String,
    /// Lowercase hex digest.
    pub hex: String,
}

impl ContentHash {
    /// Construct a SHA-256 hash from raw bytes.
    #[must_use]
    pub fn sha256_of(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        Self {
            algorithm: "sha256".to_string(),
            hex: hex_encode(&digest),
        }
    }

    /// Render as the on-disk wire string: `"<algorithm>:<hex>"`.
    #[must_use]
    pub fn to_wire_string(&self) -> String {
        format!("{}:{}", self.algorithm, self.hex)
    }

    /// Parse a wire string back to a [`ContentHash`].
    pub fn parse(s: &str) -> Result<Self, LockfileError> {
        let (algo, hex) = s
            .split_once(':')
            .ok_or_else(|| LockfileError::MalformedContentHash {
                value: s.to_string(),
                reason: "expected `<algorithm>:<hex>`".into(),
            })?;
        if algo != "sha256" {
            return Err(LockfileError::MalformedContentHash {
                value: s.to_string(),
                reason: format!("unsupported hash algorithm `{algo}`"),
            });
        }
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(LockfileError::MalformedContentHash {
                value: s.to_string(),
                reason: "sha256 hex must be 64 lowercase hex chars".into(),
            });
        }
        // Normalize to lowercase to keep round-tripping byte-stable.
        Ok(Self {
            algorithm: algo.to_string(),
            hex: hex.to_ascii_lowercase(),
        })
    }
}

impl Serialize for ContentHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_wire_string())
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// UpdatePolicy
// ---------------------------------------------------------------------------

/// How a lockfile-aware resolve interacts with the lockfile.
///
/// See module docs for semantics.
#[derive(Debug, Clone)]
pub enum UpdatePolicy {
    /// Use the lockfile as a hard pin: each top-level request resolves
    /// to its lockfile entry's commit; transitive entries also come
    /// straight from the lockfile. Effectively zero-network
    /// (post-fetch).
    Frozen,
    /// Ignore the lockfile; fully re-resolve against current upstream.
    UpdateAll,
    /// Re-resolve only the named package (and any cascade its update
    /// triggers); for every other package, prefer the lockfile's
    /// recorded version when current constraints still allow it.
    UpdateOne(PackageName),
}

// ---------------------------------------------------------------------------
// Lockfile construction & I/O
// ---------------------------------------------------------------------------

impl Lockfile {
    /// Generate a fresh lockfile from a [`ResolvePlan`].
    ///
    /// For each plan entry, resolves the chosen commit-ish to a 40-char
    /// SHA (so tag-pinned entries become commit-pinned in the lock)
    /// and computes its content hash. The entries are sorted
    /// alphabetically by [`PackageName`] for diff stability.
    pub fn from_plan(plan: &ResolvePlan, fetcher: &Fetcher) -> Result<Self, LockfileError> {
        // Map each plan entry by canonical URL so transitive `dependencies`
        // edges (which carry an address string, not a name) can be
        // canonicalized to package names for the lockfile entry.
        let mut url_to_name: BTreeMap<String, PackageName> = BTreeMap::new();
        for rp in &plan.packages {
            url_to_name.insert(rp.address.to_git_url(), rp.name.clone());
        }

        let mut entries: Vec<LockfileEntry> = Vec::with_capacity(plan.packages.len());

        for rp in &plan.packages {
            entries.push(build_entry(rp, &url_to_name, fetcher)?);
        }

        // Sort alphabetically by name for diff stability.
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Self {
            schema_version: LOCKFILE_SCHEMA_VERSION,
            generator: format!("pmacs {}", env!("CARGO_PKG_VERSION")),
            packages: entries,
        })
    }

    /// Serialize to canonical TOML bytes.
    ///
    /// Deterministic for a given `Lockfile` instance: entry order is
    /// fixed by [`Lockfile::from_plan`], dependency lists are sorted,
    /// and all maps go through `BTreeMap`. Two `from_plan` calls
    /// against the same plan produce byte-identical output.
    pub fn to_bytes(&self) -> Result<Vec<u8>, LockfileError> {
        let s = toml::to_string(self).map_err(LockfileError::Serialize)?;
        Ok(s.into_bytes())
    }

    /// Parse from on-disk TOML bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, LockfileError> {
        let text = std::str::from_utf8(bytes).map_err(|_| LockfileError::NotUtf8)?;
        let lock: Self = toml::from_str(text).map_err(LockfileError::Parse)?;
        if lock.schema_version != LOCKFILE_SCHEMA_VERSION {
            return Err(LockfileError::SchemaVersion {
                expected: LOCKFILE_SCHEMA_VERSION,
                found: lock.schema_version,
            });
        }
        Ok(lock)
    }

    /// Read a lockfile from disk.
    pub fn read_from(path: &Path) -> Result<Self, LockfileError> {
        let bytes = fs::read(path).map_err(|source| LockfileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&bytes)
    }

    /// Write to disk, creating parent dirs as needed.
    ///
    /// The final file replacement is atomic: bytes are written to a
    /// sibling temp file and renamed over `path`, so a failed write
    /// leaves the previous lockfile intact rather than truncated.
    pub fn write_to(&self, path: &Path) -> Result<(), LockfileError> {
        let bytes = self.to_bytes()?;
        Self::write_bytes_to(path, &bytes)
    }

    /// Write pre-serialized lockfile bytes to disk atomically.
    pub fn write_bytes_to(path: &Path, bytes: &[u8]) -> Result<(), LockfileError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| LockfileError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }
        crate::file_io::save_atomic(path, bytes)
            .map(|_| ())
            .map_err(|err| lockfile_save_error(path, err))?;
        Ok(())
    }

    /// Look up a lockfile entry by package name.
    #[must_use]
    pub fn entry(&self, name: &PackageName) -> Option<&LockfileEntry> {
        self.packages.iter().find(|e| &e.name == name)
    }

    /// Look up a lockfile entry by canonical clone URL.
    #[must_use]
    pub fn entry_by_url(&self, url: &str) -> Option<&LockfileEntry> {
        self.packages.iter().find(|e| e.url == url)
    }

    /// Verify that fetching `entry.commit` from the upstream and
    /// hashing the resulting tree produces `entry.content_hash`.
    ///
    /// Caller's responsibility to have already invoked [`Fetcher::fetch`]
    /// for the entry's URL; this function operates on the local cache.
    pub fn verify_entry(
        &self,
        entry: &LockfileEntry,
        fetcher: &Fetcher,
    ) -> Result<(), LockfileError> {
        let repo = fetcher
            .fetch(&entry.url)
            .map_err(|source| LockfileError::Fetch {
                url: entry.url.clone(),
                source,
            })?;
        let archive = fetcher
            .archive_commit(&repo, &entry.commit)
            .map_err(|source| LockfileError::Fetch {
                url: entry.url.clone(),
                source,
            })?;
        let observed = ContentHash::sha256_of(&archive);
        if observed == entry.content_hash {
            Ok(())
        } else {
            Err(LockfileError::ContentHashMismatch {
                name: entry.name.clone(),
                url: entry.url.clone(),
                commit: entry.commit.clone(),
                expected: entry.content_hash.clone(),
                observed,
            })
        }
    }

    /// Convert a (verified) lockfile directly into a [`ResolvePlan`],
    /// without any upstream re-resolution.
    ///
    /// Used by [`UpdatePolicy::Frozen`]. Reads each entry's manifest
    /// from the local cache (the manifest is needed downstream by the
    /// loader to compute exports and `entry`); no tag enumeration, no
    /// `pmacs_required` re-check (the resolver that produced this
    /// lockfile already verified compatibility).
    ///
    /// Each entry's content hash is verified against the upstream
    /// before its manifest is read. This is how a tampered or
    /// drift-prone cache is detected: if the bytes the upstream now
    /// serves at the recorded commit don't hash to the recorded
    /// content hash, the call fails with
    /// [`LockfileError::ContentHashMismatch`] and the install
    /// aborts. The check matches the lockfile's value proposition
    /// (reproducible installs across machines) on the install path
    /// itself, not just at lockfile-write time.
    pub fn to_resolve_plan(&self, fetcher: &Fetcher) -> Result<ResolvePlan, LockfileError> {
        let mut packages = Vec::with_capacity(self.packages.len());
        for entry in &self.packages {
            // Verify before reading the manifest. `verify_entry`
            // calls `fetcher.fetch` itself, so the cache is warm
            // for the subsequent `show_blob`.
            self.verify_entry(entry, fetcher)?;
            let repo = fetcher
                .fetch(&entry.url)
                .map_err(|source| LockfileError::Fetch {
                    url: entry.url.clone(),
                    source,
                })?;
            let bytes = fetcher
                .show_blob(&repo, &entry.commit, "pmacs.toml")
                .map_err(|source| LockfileError::Fetch {
                    url: entry.url.clone(),
                    source,
                })?;
            let text = std::str::from_utf8(&bytes).map_err(|_| LockfileError::ManifestNotUtf8 {
                url: entry.url.clone(),
                commit: entry.commit.clone(),
            })?;
            let manifest = super::manifest::PackageManifest::from_toml(text).map_err(|source| {
                LockfileError::Manifest {
                    url: entry.url.clone(),
                    commit: entry.commit.clone(),
                    message: source.to_string(),
                }
            })?;
            // The lockfile stores a canonical clone URL (the result of
            // `Address::to_git_url`). Re-parsing it via `Address::parse`
            // is wrong: `file://` and similar schemes that Address only
            // accepts behind a `git:` prefix would fail. Wrap the URL
            // directly as `Address::Url` — this preserves the
            // round-trip identity for downstream fetcher calls.
            let address = Address::Url(entry.url.clone());
            let top_level_pin = entry
                .top_level_pin
                .as_ref()
                .map(LockfilePin::to_install_pin)
                .transpose()?;

            packages.push(ResolvedPackage {
                name: entry.name.clone(),
                address,
                commit: entry.commit.clone(),
                version: entry.version.clone(),
                manifest,
                top_level_pin,
            });
        }
        // Lockfile entries are alphabetical on disk; downstream
        // consumers want topological order. Sort using each entry's
        // recorded `dependencies` list as the edges.
        let order = topo_sort(&self.packages)?;
        let mut by_name: BTreeMap<PackageName, ResolvedPackage> =
            packages.into_iter().map(|p| (p.name.clone(), p)).collect();
        let topo: Vec<ResolvedPackage> = order
            .into_iter()
            .map(|name| {
                by_name
                    .remove(&name)
                    .expect("topo_sort returns only known names")
            })
            .collect();
        Ok(ResolvePlan { packages: topo })
    }
}

fn build_entry(
    rp: &ResolvedPackage,
    url_to_name: &BTreeMap<String, PackageName>,
    fetcher: &Fetcher,
) -> Result<LockfileEntry, LockfileError> {
    let url = rp.address.to_git_url();

    let repo = fetcher.fetch(&url).map_err(|source| LockfileError::Fetch {
        url: url.clone(),
        source,
    })?;

    // Resolve the chosen commit-ish (which may be a tag string for
    // version pins) to a 40-char SHA.
    let sha = fetcher
        .resolve(&repo, &RefSpec::Commit(rp.commit.clone()))
        .map_err(|source| LockfileError::Fetch {
            url: url.clone(),
            source,
        })?;

    // Hash the canonical archive at the resolved SHA.
    let archive = fetcher
        .archive_commit(&repo, &sha)
        .map_err(|source| LockfileError::Fetch {
            url: url.clone(),
            source,
        })?;
    let content_hash = ContentHash::sha256_of(&archive);

    // Immediate dependency names, sorted. Resolve manifest dep
    // addresses to canonical clone URLs, then look up the chosen name.
    let mut deps: BTreeSet<PackageName> = BTreeSet::new();
    for d in &rp.manifest.dependencies {
        // Stale manifest with malformed address: the resolver would
        // have errored earlier, so silently skip rather than fail
        // lockfile generation on a defensive path.
        let Ok(dep_addr) = Address::parse(&d.address) else {
            continue;
        };
        let dep_url = dep_addr.to_git_url();
        if let Some(name) = url_to_name.get(&dep_url) {
            deps.insert(name.clone());
        }
    }

    Ok(LockfileEntry {
        name: rp.name.clone(),
        url,
        commit: sha,
        version: rp.version.clone(),
        content_hash,
        top_level_pin: rp
            .top_level_pin
            .as_ref()
            .and_then(LockfilePin::from_install_pin),
        dependencies: deps.into_iter().collect(),
    })
}

/// Kahn's topological sort over lockfile entries' `dependencies`
/// lists. Deterministic via `BTreeSet` ready set.
fn topo_sort(entries: &[LockfileEntry]) -> Result<Vec<PackageName>, LockfileError> {
    let names: BTreeSet<PackageName> = entries.iter().map(|e| e.name.clone()).collect();
    let mut indegree: BTreeMap<PackageName, usize> = entries
        .iter()
        .map(|e| {
            let count = e.dependencies.iter().filter(|d| names.contains(*d)).count();
            (e.name.clone(), count)
        })
        .collect();

    let mut dependents: BTreeMap<PackageName, BTreeSet<PackageName>> = BTreeMap::new();
    for e in entries {
        for d in &e.dependencies {
            if names.contains(d) {
                dependents
                    .entry(d.clone())
                    .or_default()
                    .insert(e.name.clone());
            }
        }
    }

    let mut ready: BTreeSet<PackageName> = indegree
        .iter()
        .filter(|&(_, d)| *d == 0)
        .map(|(n, _)| n.clone())
        .collect();
    let mut order: Vec<PackageName> = Vec::with_capacity(entries.len());
    let mut q: VecDeque<PackageName> = ready.iter().cloned().collect();
    ready.clear();

    while let Some(next) = q.pop_front() {
        order.push(next.clone());
        if let Some(succs) = dependents.get(&next) {
            // Buffer succs into ready (BTreeSet) for deterministic order.
            for x in succs {
                if let Some(d) = indegree.get_mut(x) {
                    *d = d.saturating_sub(1);
                    if *d == 0 {
                        ready.insert(x.clone());
                    }
                }
            }
        }
        // Drain ready into queue in alphabetical order.
        for next_ready in std::mem::take(&mut ready) {
            q.push_back(next_ready);
        }
    }

    if order.len() != entries.len() {
        let cycle: Vec<PackageName> = entries
            .iter()
            .map(|e| e.name.clone())
            .filter(|n| !order.contains(n))
            .collect();
        return Err(LockfileError::DependencyCycle {
            participants: cycle,
        });
    }
    Ok(order)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by lockfile generation, parsing, and verification.
#[derive(Debug, Error)]
pub enum LockfileError {
    /// File I/O failed.
    #[error("lockfile I/O at {}: {source}", path.display())]
    Io {
        /// Path being read or written.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Lockfile bytes were not valid UTF-8.
    #[error("lockfile is not valid UTF-8")]
    NotUtf8,
    /// TOML parse failed.
    #[error("lockfile parse error: {0}")]
    Parse(toml::de::Error),
    /// TOML serialize failed.
    #[error("lockfile serialize error: {0}")]
    Serialize(toml::ser::Error),
    /// Lockfile schema version is incompatible with this pmacs.
    #[error(
        "lockfile schema version {found} is incompatible with this pmacs (expects {expected}). \
         Upgrade pmacs, or regenerate the lockfile with `pmacs.packages.update()`."
    )]
    SchemaVersion {
        /// Schema version this build expects.
        expected: u32,
        /// Schema version found in the file.
        found: u32,
    },
    /// A `content_hash` field was not in the form `<algorithm>:<hex>`.
    #[error("malformed content_hash `{value}`: {reason}")]
    MalformedContentHash {
        /// The offending string.
        value: String,
        /// Why parsing failed.
        reason: String,
    },
    /// A `top_level_pin` of `kind = "version"` had a `value` that does
    /// not parse as a [`VersionReq`].
    #[error("invalid version constraint `{value}`: {cause}")]
    InvalidVersionConstraint {
        /// The offending value.
        value: String,
        /// Underlying parse error.
        cause: String,
    },
    /// Fetcher (clone, archive, rev-parse) failed during lockfile
    /// generation or verification.
    #[error("fetch error for {url}: {source}")]
    Fetch {
        /// Clone URL being fetched.
        url: String,
        /// Underlying fetch error.
        #[source]
        source: FetchError,
    },
    /// Computed content hash does not match the lockfile's recorded
    /// hash. Tampered upstream or stale lockfile.
    #[error(
        "content_hash mismatch for `{name_str}` ({url}@{commit_short}):\n\
         expected: {expected_wire}\n\
         observed: {observed_wire}\n\
         The upstream's content at this commit differs from what was recorded \
         when the lockfile was generated. Either the upstream has been \
         tampered with, or your local cache is corrupt. Refusing to install. \
         If this is intentional (e.g. you regenerated the upstream), run \
         `pmacs.packages.update(\"{name_str}\")` to refresh the lockfile.",
        name_str = name.as_str(),
        commit_short = &commit[..commit.len().min(12)],
        expected_wire = expected.to_wire_string(),
        observed_wire = observed.to_wire_string(),
    )]
    ContentHashMismatch {
        /// Package whose content didn't match.
        name: PackageName,
        /// URL that was fetched.
        url: String,
        /// Commit hash that was hashed.
        commit: String,
        /// Hash recorded in the lockfile.
        expected: ContentHash,
        /// Hash computed against the (possibly tampered) upstream.
        observed: ContentHash,
    },
    /// Manifest at a lockfile entry's commit failed to parse.
    #[error("manifest at {url}@{commit}: {message}")]
    Manifest {
        /// URL.
        url: String,
        /// Commit.
        commit: String,
        /// Underlying parse error (stringified to keep this error type
        /// free of cycles back into resolver / manifest internals).
        message: String,
    },
    /// Manifest at a lockfile entry's commit was not valid UTF-8.
    #[error("manifest at {url}@{commit}: not valid UTF-8")]
    ManifestNotUtf8 {
        /// URL.
        url: String,
        /// Commit.
        commit: String,
    },
    /// `to_resolve_plan` found a cycle in the recorded dependency
    /// edges. Indicates a hand-corrupted lockfile.
    #[error(
        "lockfile dependency graph contains a cycle: {participants:?}. \
         Regenerate with `pmacs.packages.update()`."
    )]
    DependencyCycle {
        /// Names involved in the cycle.
        participants: Vec<PackageName>,
    },
    /// A lockfile entry referenced by an `UpdatePolicy::UpdateOne`
    /// is absent.
    #[error(
        "package `{}` is not in the lockfile; cannot update it. \
         Use `pmacs.packages.update()` to regenerate the full lockfile.",
        name.as_str(),
    )]
    UpdateOneMissing {
        /// The name that was passed to `UpdateOne`.
        name: PackageName,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_sha256_known_vector() {
        // RFC 6234 §8.5: SHA-256 of "abc"
        let h = ContentHash::sha256_of(b"abc");
        assert_eq!(h.algorithm, "sha256");
        assert_eq!(
            h.hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        );
        assert_eq!(
            h.to_wire_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        );
    }

    #[test]
    fn content_hash_round_trip_via_wire() {
        let h = ContentHash::sha256_of(b"hello");
        let parsed = ContentHash::parse(&h.to_wire_string()).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn content_hash_parse_rejects_unsupported_algorithm() {
        let err =
            ContentHash::parse("md5:0123456789abcdef0123456789abcdef").expect_err("md5 rejected");
        match err {
            LockfileError::MalformedContentHash { reason, .. } => {
                assert!(reason.contains("md5"), "reason = {reason}");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn content_hash_parse_rejects_short_hex() {
        let err = ContentHash::parse("sha256:abc").expect_err("short hex rejected");
        assert!(matches!(err, LockfileError::MalformedContentHash { .. }));
    }

    #[test]
    fn content_hash_parse_rejects_non_hex_chars() {
        let err = ContentHash::parse(&format!("sha256:{}", "z".repeat(64))).expect_err("non-hex");
        assert!(matches!(err, LockfileError::MalformedContentHash { .. }));
    }

    #[test]
    fn content_hash_parse_normalizes_uppercase() {
        let upper = format!("sha256:{}", "A".repeat(64));
        let parsed = ContentHash::parse(&upper).unwrap();
        assert_eq!(parsed.hex, "a".repeat(64));
    }

    #[test]
    fn lockfile_pin_round_trip() {
        for pin in [
            InstallPin::Version(VersionReq::parse("^1.0.0").unwrap()),
            InstallPin::Branch("main".into()),
            InstallPin::Commit("abc123".into()),
        ] {
            let lp = LockfilePin::from_install_pin(&pin)
                .expect("non-Local pins always convert to LockfilePin");
            let back = lp.to_install_pin().unwrap();
            assert_eq!(back, pin);
        }
    }

    #[test]
    fn lockfile_pin_invalid_version_constraint_errors() {
        let bad = LockfilePin::Version("not-a-constraint".into());
        let err = bad.to_install_pin().expect_err("bad VersionReq rejected");
        assert!(
            matches!(err, LockfileError::InvalidVersionConstraint { .. }),
            "got {err:?}",
        );
    }

    #[test]
    fn lockfile_serializes_alphabetically_by_name() {
        // Synthesize a lockfile by hand (no fetcher needed) and
        // verify the serialized form lists packages in name order.
        let pkg_b = LockfileEntry {
            name: PackageName::new("b-pkg").unwrap(),
            url: "https://example.com/b.git".into(),
            commit: "0".repeat(40),
            version: Version::new(1, 0, 0),
            content_hash: ContentHash::sha256_of(b"b"),
            top_level_pin: None,
            dependencies: vec![],
        };
        let pkg_a = LockfileEntry {
            name: PackageName::new("a-pkg").unwrap(),
            url: "https://example.com/a.git".into(),
            commit: "1".repeat(40),
            version: Version::new(1, 0, 0),
            content_hash: ContentHash::sha256_of(b"a"),
            top_level_pin: Some(LockfilePin::Version("^1".into())),
            dependencies: vec![PackageName::new("b-pkg").unwrap()],
        };
        let mut lock = Lockfile {
            schema_version: LOCKFILE_SCHEMA_VERSION,
            generator: "pmacs test".into(),
            packages: vec![pkg_b, pkg_a],
        };
        // Manual sort to mirror what `from_plan` does.
        lock.packages.sort_by(|a, b| a.name.cmp(&b.name));
        let text = String::from_utf8(lock.to_bytes().unwrap()).unwrap();
        let pos_a = text.find("name = \"a-pkg\"").expect("a-pkg present");
        let pos_b = text.find("name = \"b-pkg\"").expect("b-pkg present");
        assert!(pos_a < pos_b, "expected a-pkg before b-pkg in:\n{text}");
    }

    #[test]
    fn lockfile_round_trip_via_bytes() {
        let entry = LockfileEntry {
            name: PackageName::new("a-pkg").unwrap(),
            url: "https://example.com/a.git".into(),
            commit: "1".repeat(40),
            version: Version::new(1, 2, 3),
            content_hash: ContentHash::sha256_of(b"hello"),
            top_level_pin: Some(LockfilePin::Version("^1".into())),
            dependencies: vec![PackageName::new("b-pkg").unwrap()],
        };
        let lock = Lockfile {
            schema_version: LOCKFILE_SCHEMA_VERSION,
            generator: "pmacs test".into(),
            packages: vec![entry],
        };
        let bytes = lock.to_bytes().unwrap();
        let parsed = Lockfile::parse(&bytes).unwrap();
        assert_eq!(parsed, lock);
    }

    #[test]
    fn lockfile_parse_rejects_wrong_schema_version() {
        let bytes = Lockfile {
            schema_version: 99,
            generator: "x".into(),
            packages: vec![],
        }
        .to_bytes()
        .unwrap();
        let err = Lockfile::parse(&bytes).expect_err("wrong version rejected");
        assert!(
            matches!(
                err,
                LockfileError::SchemaVersion {
                    expected: 1,
                    found: 99
                }
            ),
            "got {err:?}",
        );
    }

    #[test]
    fn topo_sort_orders_dependencies_first() {
        // a depends on b, b depends on c.
        let mk = |name: &str, deps: &[&str]| LockfileEntry {
            name: PackageName::new(name).unwrap(),
            url: format!("https://example.com/{name}.git"),
            commit: "0".repeat(40),
            version: Version::new(1, 0, 0),
            content_hash: ContentHash::sha256_of(name.as_bytes()),
            top_level_pin: None,
            dependencies: deps.iter().map(|d| PackageName::new(*d).unwrap()).collect(),
        };
        let entries = vec![mk("a", &["b"]), mk("b", &["c"]), mk("c", &[])];
        let order = topo_sort(&entries).unwrap();
        let names: Vec<&str> = order.iter().map(PackageName::as_str).collect();
        assert_eq!(names, vec!["c", "b", "a"]);
    }

    #[test]
    fn topo_sort_detects_cycle() {
        let mk = |name: &str, deps: &[&str]| LockfileEntry {
            name: PackageName::new(name).unwrap(),
            url: format!("https://example.com/{name}.git"),
            commit: "0".repeat(40),
            version: Version::new(1, 0, 0),
            content_hash: ContentHash::sha256_of(name.as_bytes()),
            top_level_pin: None,
            dependencies: deps.iter().map(|d| PackageName::new(*d).unwrap()).collect(),
        };
        // a → b → a
        let entries = vec![mk("a", &["b"]), mk("b", &["a"])];
        let err = topo_sort(&entries).expect_err("cycle detected");
        match err {
            LockfileError::DependencyCycle { participants } => {
                let names: Vec<&str> = participants.iter().map(PackageName::as_str).collect();
                assert_eq!(names.len(), 2);
                assert!(names.contains(&"a"));
                assert!(names.contains(&"b"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
