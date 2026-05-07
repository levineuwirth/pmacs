// packages/resolver.rs --- Dependency resolver (T M7.5).

//! Take a top-level set of `(address, pin)` pairs and produce a flat
//! plan: one chosen `(commit, version, manifest)` per package, ordered
//! so dependencies precede dependents.
//!
//! This module is the spine of the M7.5 → M7.6 → M7.7 chain: M7.6 turns
//! a `ResolvePlan` into a `pmacs.lock` file, M7.7 walks a plan to wire
//! installs into Lua's `require`. The resolver itself is Rust-only;
//! Lua exposure waits for M7.7+ when the lockfile feedback loop is
//! settled.
//!
//! # Algorithm
//!
//! Iterate-to-fixed-point with **monotonic-version-decrease**. Each
//! iteration either lowers some chosen package's version (because a
//! new constraint excluded the prior choice) or terminates.
//!
//! ## Why this terminates
//!
//! Constraints on a package name are **additive**: once a constraint
//! is recorded, it stays in the constraint set even if the manifest
//! that introduced it is later replaced. Adding constraints can only
//! narrow the satisfying-tag set; the resolver always picks the
//! highest tag in the satisfying set. Since the per-package version
//! set is finite, each chosen-version change strictly decreases some
//! version, and the sum of chosen versions across all packages is a
//! monotonically-decreasing well-founded measure. Termination is
//! guaranteed.
//!
//! ## Tradeoff: stale constraints can over-constrain
//!
//! Additive accumulation means a constraint introduced by a manifest
//! that has since been replaced still applies. In a constraint cycle
//! where two packages alternately push each other down, the algorithm
//! collects both sets of constraints and may report an unsatisfiable
//! conflict on a graph that a backtracking SAT solver would resolve.
//! That's a deliberate tradeoff: the resolver stays simple,
//! comprehensible, and predictable; pathological cycles surface as
//! clear errors rather than silent oscillation. v1.0 doesn't ship a
//! SAT solver; v2.0 may revisit if real-world graphs surface false
//! positives.
//!
//! # Two-pass version filtering
//!
//! For version-pinned packages (top-level or transitive), the
//! resolver picks the highest tag that satisfies **both** the user /
//! transitive constraint **and** the manifest's `pmacs_required`
//! against the running pmacs version. The two filters surface as
//! distinct errors so users can tell "loosen your constraint" from
//! "upgrade pmacs" apart:
//!
//! - [`ResolveError::NoVersionMatchesConstraints`] — no tag matches
//!   the version constraint set.
//! - [`ResolveError::NoVersionMatchesPmacsRequirement`] — tags match
//!   the version constraints, but none of them are pmacs-compatible.
//!
//! Manifest fetches are lazy: tags are walked highest-first, and the
//! first tag whose manifest passes both filters wins. The common
//! case (latest tag is compatible) costs one manifest fetch.
//!
//! # Determinism
//!
//! All iteration order goes through `BTreeMap` / sorted vectors. Tag
//! selection is `Version::cmp` descending. Two resolves of the same
//! input produce byte-identical plans modulo upstream Git state.

// Resolver errors carry rich diagnostic context (constraint paths,
// available tag lists, conflict declarations) by design — the user-
// facing `Display` is the entire reason this enum is wide. Boxing the
// error to placate `result_large_err` would only move the cost
// without changing the user-visible surface, so the lint is allowed
// for this module.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::PathBuf;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::address::{Address, AddressError};
use super::fetcher::{FetchError, Fetcher, RefSpec};
use super::installer::{InstallPin, running_pmacs_version};
use super::lockfile::{Lockfile, LockfileError, UpdatePolicy};
use super::manifest::{ManifestError, PackageManifest, PackageName};

// ---------------------------------------------------------------------------
// Public input types
// ---------------------------------------------------------------------------

/// One top-level entry to feed the resolver.
#[derive(Debug, Clone)]
pub struct ResolveRequest {
    /// Where to fetch the package from.
    pub address: Address,
    /// How the user pinned the install: by version constraint, branch
    /// name, or specific commit.
    pub pin: InstallPin,
}

// ---------------------------------------------------------------------------
// Public output types
// ---------------------------------------------------------------------------

/// Where a recorded constraint came from. Used to format conflict
/// errors so the user can see the chain of declarations that
/// produced the constraint set.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Source {
    /// A top-level user request (passed directly to
    /// [`Resolver::resolve`]).
    TopLevel,
    /// A `dependencies` entry inside another package's manifest.
    DependencyOf {
        /// The package whose manifest declared this dependency.
        name: PackageName,
        /// The version of that package at which the dependency was
        /// observed.
        version: Version,
    },
}

impl Source {
    fn display(&self) -> String {
        match self {
            Self::TopLevel => "(top-level)".to_string(),
            Self::DependencyOf { name, version } => format!("{} @ {}", name.as_str(), version),
        }
    }
}

/// One entry in a successful resolve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedPackage {
    /// Canonical package name from the resolved manifest.
    pub name: PackageName,
    /// Address used to fetch this package. For transitive deps this
    /// is whatever string the parent manifest recorded.
    pub address: Address,
    /// Full 40-character commit hash this package resolves to.
    pub commit: String,
    /// Version parsed from the manifest at `commit`.
    pub version: Version,
    /// Manifest at `commit`, captured during resolution.
    pub manifest: PackageManifest,
    /// `Some(pin)` if this entry came directly from a top-level
    /// [`ResolveRequest`]; `None` if it was pulled in transitively
    /// via another package's `dependencies`. M7.6's lockfile uses
    /// this to know which entries should re-resolve as branches on
    /// `update`: top-level branch pins move with upstream, but
    /// transitive deps are derived from manifests and don't carry
    /// branch semantics.
    pub top_level_pin: Option<InstallPin>,
}

/// Output of a successful resolve.
///
/// `packages` is in **topological order**: every entry's manifest
/// `dependencies` reference packages that appear earlier in the
/// vector. M7.6 / M7.7 walk it in order to install / load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePlan {
    /// Packages in topological order.
    pub packages: Vec<ResolvedPackage>,
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

/// The resolver itself. Borrows a [`Fetcher`] for the duration of a
/// resolve.
#[derive(Debug)]
pub struct Resolver {
    fetcher: Fetcher,
}

impl Resolver {
    /// Construct a resolver over the given fetcher's cache.
    #[must_use]
    pub fn new(fetcher: Fetcher) -> Self {
        Self { fetcher }
    }

    /// Borrow the underlying fetcher (useful for callers that want to
    /// reuse the same cache for downstream installs).
    #[must_use]
    pub fn fetcher(&self) -> &Fetcher {
        &self.fetcher
    }

    /// Resolve a top-level request set into a flat install plan.
    ///
    /// See module-level docs for the algorithm and termination
    /// argument.
    pub fn resolve(&self, requests: &[ResolveRequest]) -> Result<ResolvePlan, ResolveError> {
        let pmacs_version = running_pmacs_version();
        let mut state = ResolverState::new(&self.fetcher, &pmacs_version);

        for req in requests {
            state.add_top_level(req)?;
        }

        state.iterate_to_fixpoint()?;
        state.check_conflicts()?;
        state.into_plan()
    }

    /// Resolve under an explicit lockfile / [`UpdatePolicy`].
    ///
    /// - [`UpdatePolicy::Frozen`] short-circuits to
    ///   [`Lockfile::to_resolve_plan`] — no upstream re-resolution. The
    ///   `requests` set must be a subset of the lockfile by canonical
    ///   URL; entries in `requests` not present in the lockfile produce
    ///   [`ResolveError::FrozenRequestMissing`] (the user must update
    ///   the lockfile or remove the new entry).
    /// - [`UpdatePolicy::UpdateAll`] ignores the lockfile and delegates
    ///   to [`Resolver::resolve`].
    /// - [`UpdatePolicy::UpdateOne`] runs a fresh resolve with the
    ///   lockfile's recorded versions as **preferences** for every
    ///   package except the named target. A preferred version is
    ///   chosen if it satisfies the current constraint set; otherwise
    ///   the resolver falls back to highest-version selection
    ///   (cascade behavior).
    ///
    /// `lockfile` is required for `Frozen` and `UpdateOne`. Passing
    /// `None` with either policy returns
    /// [`ResolveError::LockfileRequired`].
    pub fn resolve_with_policy(
        &self,
        requests: &[ResolveRequest],
        lockfile: Option<&Lockfile>,
        policy: &UpdatePolicy,
    ) -> Result<ResolvePlan, ResolveError> {
        match (policy, lockfile) {
            (UpdatePolicy::UpdateAll, _) => self.resolve(requests),
            (UpdatePolicy::Frozen, Some(lock)) => {
                // Validate that every top-level request appears in the lockfile.
                for req in requests {
                    let url = req.address.to_git_url();
                    if lock.entry_by_url(&url).is_none() {
                        return Err(ResolveError::FrozenRequestMissing {
                            address: req.address.clone(),
                        });
                    }
                }
                lock.to_resolve_plan(&self.fetcher)
                    .map_err(ResolveError::Lockfile)
            }
            (UpdatePolicy::UpdateOne(name), Some(lock)) => {
                if lock.entry(name).is_none() {
                    return Err(ResolveError::Lockfile(LockfileError::UpdateOneMissing {
                        name: name.clone(),
                    }));
                }
                let mut prefer: BTreeMap<String, Version> = BTreeMap::new();
                for entry in &lock.packages {
                    if &entry.name == name {
                        continue;
                    }
                    prefer.insert(entry.url.clone(), entry.version.clone());
                }
                let pmacs_version = running_pmacs_version();
                let mut state = ResolverState::new(&self.fetcher, &pmacs_version);
                state.prefer_versions = prefer;
                for req in requests {
                    state.add_top_level(req)?;
                }
                state.iterate_to_fixpoint()?;
                state.check_conflicts()?;
                state.into_plan()
            }
            (UpdatePolicy::Frozen | UpdatePolicy::UpdateOne(_), None) => {
                Err(ResolveError::LockfileRequired)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ResolverState (internal)
// ---------------------------------------------------------------------------

/// All mutable state for a single `resolve()` invocation.
struct ResolverState<'a> {
    fetcher: &'a Fetcher,
    pmacs_version: &'a Version,

    /// Each package by canonical name plus its current resolution.
    chosen: BTreeMap<PackageName, ChosenEntry>,
    /// Constraints accumulated across the resolve. **Additive**:
    /// constraints stay in this map even when the manifest that
    /// introduced them is replaced. See module docs for why.
    constraints: BTreeMap<PackageName, Vec<(Source, VersionReq)>>,
    /// Conflict declarations stashed during resolution; checked once
    /// after fixed-point convergence. Like `constraints`, this is
    /// additive — declarations from replaced manifests still count.
    conflicts: Vec<ConflictDecl>,

    /// Repo cache: address-string → on-disk fetched repo path.
    /// Keyed by the post-`Address::to_git_url()` clone URL so
    /// equivalent address spellings (`github:foo/bar` vs.
    /// `https://github.com/foo/bar.git`) share fetched data.
    repo_paths: BTreeMap<String, PathBuf>,
    /// Manifest cache: `(git_url, commit)` → manifest. Avoids
    /// re-running `git show` for the same blob; load-bearing on
    /// diamond-dependency graphs where multiple constraint paths
    /// reach the same commit.
    manifests: BTreeMap<(String, String), PackageManifest>,

    /// Pending dep edges discovered but not yet processed.
    pending: VecDeque<PendingEdge>,

    /// Lockfile-supplied version hints. When `pick_best_version` finds
    /// a candidate matching the hinted version (and the candidate
    /// satisfies the constraint set), it is preferred over the
    /// highest-version candidate. Empty for fresh resolves; populated
    /// from the lockfile for [`UpdatePolicy::UpdateOne`]. Keyed by
    /// canonical clone URL because the resolver learns canonical names
    /// only after picking a version.
    prefer_versions: BTreeMap<String, Version>,
}

/// One pending dependency edge waiting to be merged into the chosen
/// set. Carries everything needed to resolve a name without
/// re-touching its parent manifest.
#[derive(Debug, Clone)]
struct PendingEdge {
    /// Address-string from the parent's `dependencies` entry.
    /// Re-parsed lazily (the manifest stored it as a `String`).
    address: String,
    /// Constraint contributed by the parent manifest.
    constraint: VersionReq,
    /// Which manifest declared this constraint (for error attribution).
    source: Source,
}

#[derive(Debug, Clone)]
struct ChosenEntry {
    address: Address,
    commit: String,
    version: Version,
    manifest: PackageManifest,
    top_level_pin: Option<InstallPin>,
}

#[derive(Debug, Clone)]
struct ConflictDecl {
    declaring_name: PackageName,
    declaring_version: Version,
    target_address: String,
    target_constraint: VersionReq,
}

impl<'a> ResolverState<'a> {
    fn new(fetcher: &'a Fetcher, pmacs_version: &'a Version) -> Self {
        Self {
            fetcher,
            pmacs_version,
            chosen: BTreeMap::new(),
            constraints: BTreeMap::new(),
            conflicts: Vec::new(),
            repo_paths: BTreeMap::new(),
            manifests: BTreeMap::new(),
            pending: VecDeque::new(),
            prefer_versions: BTreeMap::new(),
        }
    }

    // -- Top-level seeding ----------------------------------------------

    fn add_top_level(&mut self, req: &ResolveRequest) -> Result<(), ResolveError> {
        let url = req.address.to_git_url();
        self.ensure_repo(&url)?;
        let repo = self.repo_paths[&url].clone();

        let (commit, manifest) = match &req.pin {
            InstallPin::Version(constraint) => {
                let candidates = self.list_tags(&repo, &url, &Source::TopLevel, &req.address)?;
                let chosen = self.pick_best_version(
                    &repo,
                    &url,
                    &req.address,
                    &candidates,
                    &[(Source::TopLevel, constraint.clone())],
                )?;
                let manifest = self.read_manifest(&url, &repo, &chosen.commit)?.clone();
                (chosen.commit, manifest)
            }
            InstallPin::Branch(b) => {
                let commit = self
                    .fetcher
                    .resolve(&repo, &RefSpec::Branch(b.clone()))
                    .map_err(|source| ResolveError::Fetch {
                        address: req.address.clone(),
                        source,
                    })?;
                let manifest = self.read_manifest(&url, &repo, &commit)?.clone();
                self.check_pinned_pmacs_required(&req.address, &manifest, &commit, "branch", b)?;
                (commit, manifest)
            }
            InstallPin::Commit(c) => {
                let commit = self
                    .fetcher
                    .resolve(&repo, &RefSpec::Commit(c.clone()))
                    .map_err(|source| ResolveError::Fetch {
                        address: req.address.clone(),
                        source,
                    })?;
                let manifest = self.read_manifest(&url, &repo, &commit)?.clone();
                self.check_pinned_pmacs_required(&req.address, &manifest, &commit, "commit", c)?;
                (commit, manifest)
            }
            // Local pins are ephemeral working-tree symlinks
            // installed via install_local (T M8.1c). They never go
            // through the resolver because they have no clone URL
            // and no resolve story --- the source path is the
            // canonical bytes. add_top_level is unreachable from
            // any caller that admits Local pins.
            InstallPin::Local { .. } => return Err(ResolveError::LocalPinNotResolvable),
        };

        let name = manifest.name.clone();
        let version = manifest.version.clone();

        if self.chosen.contains_key(&name) {
            return Err(ResolveError::DuplicateTopLevel { name });
        }

        if let InstallPin::Version(constraint) = &req.pin {
            self.constraints
                .entry(name.clone())
                .or_default()
                .push((Source::TopLevel, constraint.clone()));
        }

        self.enqueue_dependencies(&name, &version, &manifest);
        self.stash_conflicts(&name, &version, &manifest);

        self.chosen.insert(
            name,
            ChosenEntry {
                address: req.address.clone(),
                commit,
                version,
                manifest,
                top_level_pin: Some(req.pin.clone()),
            },
        );
        Ok(())
    }

    // -- Iteration ------------------------------------------------------

    fn iterate_to_fixpoint(&mut self) -> Result<(), ResolveError> {
        // Termination invariant: each iteration either drains a
        // pending edge (finite supply, monotonically grows only when
        // a chosen version *decreases*) or strictly lowers some
        // chosen package's version. Both measures are well-founded.
        loop {
            while let Some(edge) = self.pending.pop_front() {
                self.process_pending(&edge)?;
            }

            let names: Vec<PackageName> = self.chosen.keys().cloned().collect();
            let mut changed = false;
            for name in names {
                if self.recheck_chosen(&name)? {
                    changed = true;
                }
            }

            if !changed && self.pending.is_empty() {
                break;
            }
        }
        Ok(())
    }

    fn process_pending(&mut self, edge: &PendingEdge) -> Result<(), ResolveError> {
        let address =
            Address::parse(&edge.address).map_err(|source| ResolveError::AddressInDependency {
                address: edge.address.clone(),
                source,
                declared_by: edge.source.clone(),
            })?;
        let url = address.to_git_url();
        self.ensure_repo(&url)?;

        // Reuse any existing chosen entry whose URL matches: the dep
        // edge may simply be a re-statement of a constraint we
        // already know, or a new constraint on an already-chosen
        // package. Either way we go through `recheck_chosen` once we
        // know the canonical name.
        //
        // Determining the name requires reading a manifest. Pick the
        // best-version tag under the *new* constraint alone first,
        // read its manifest to learn the name, then merge with
        // existing constraints + recheck.
        let repo = self.repo_paths[&url].clone();
        let candidates = self.list_tags(&repo, &url, &edge.source, &address)?;
        let probe = self.pick_best_version(
            &repo,
            &url,
            &address,
            &candidates,
            &[(edge.source.clone(), edge.constraint.clone())],
        )?;
        let probe_manifest = self.read_manifest(&url, &repo, &probe.commit)?.clone();
        let name = probe_manifest.name.clone();

        // Record the constraint under the canonical name.
        self.constraints
            .entry(name.clone())
            .or_default()
            .push((edge.source.clone(), edge.constraint.clone()));

        if self.chosen.contains_key(&name) {
            // Already chosen: re-resolve under the now-larger
            // constraint set. `recheck_chosen` handles the
            // monotonic-decrease step.
            self.recheck_chosen(&name)?;
        } else {
            // First time seeing this name: install the probe choice.
            // It already satisfies the new constraint and pmacs_required.
            let version = probe_manifest.version.clone();
            self.enqueue_dependencies(&name, &version, &probe_manifest);
            self.stash_conflicts(&name, &version, &probe_manifest);
            self.chosen.insert(
                name,
                ChosenEntry {
                    address,
                    commit: probe.commit,
                    version,
                    manifest: probe_manifest,
                    top_level_pin: None,
                },
            );
        }
        Ok(())
    }

    /// Re-resolve `name` under its current constraint set. If the
    /// best-version drops below the currently-chosen version, replace
    /// the chosen entry, enqueue the new manifest's deps, and stash
    /// its conflicts. Returns true if anything changed.
    fn recheck_chosen(&mut self, name: &PackageName) -> Result<bool, ResolveError> {
        let entry = match self.chosen.get(name) {
            Some(e) => e.clone(),
            None => return Ok(false),
        };

        // Top-level branch / commit pins are not subject to
        // constraint-driven re-resolution: the user fixed the
        // revision explicitly. New transitive constraints either
        // already match or they conflict — the conflict surfaces in
        // `check_conflicts` (for manifest conflicts) or in the
        // version-mismatch check below.
        if let Some(InstallPin::Branch(_) | InstallPin::Commit(_)) = &entry.top_level_pin {
            // Validate that all current constraints still match the
            // pinned version. If not, surface a typed error so the
            // user knows the explicit pin disagrees with a transitive
            // constraint.
            let constraints = self.constraints.get(name).map_or(&[][..], Vec::as_slice);
            for (source, req) in constraints {
                if matches!(source, Source::TopLevel) {
                    continue;
                }
                if !req.matches(&entry.version) {
                    return Err(ResolveError::PinnedRevisionViolatesConstraint {
                        name: name.clone(),
                        version: entry.version.clone(),
                        via: source.clone(),
                        constraint: req.clone(),
                    });
                }
            }
            return Ok(false);
        }

        let url = entry.address.to_git_url();
        let repo = self.repo_paths[&url].clone();
        let candidates = self.list_tags(&repo, &url, &Source::TopLevel, &entry.address)?;

        // Use *all* current constraints on this name (additive).
        let constraints = self.constraints.get(name).cloned().unwrap_or_default();
        let constraints = if constraints.is_empty() {
            // Top-level Version pin only — fall back to "any" so we
            // pick the highest tag that's pmacs-compatible.
            vec![(Source::TopLevel, VersionReq::STAR)]
        } else {
            constraints
        };

        let chosen =
            self.pick_best_version(&repo, &url, &entry.address, &candidates, &constraints)?;
        if chosen.version >= entry.version {
            // No-op: re-resolution picked the same or higher version.
            // (Higher would only happen via "no constraint change"
            // re-checks; we guard with >= so the loop is stable.)
            return Ok(false);
        }

        // Strictly lower: update chosen, enqueue the new manifest's
        // deps, stash its conflicts.
        let new_manifest = self.read_manifest(&url, &repo, &chosen.commit)?.clone();
        let new_version = new_manifest.version.clone();

        self.enqueue_dependencies(name, &new_version, &new_manifest);
        self.stash_conflicts(name, &new_version, &new_manifest);

        self.chosen.insert(
            name.clone(),
            ChosenEntry {
                address: entry.address.clone(),
                commit: chosen.commit,
                version: new_version,
                manifest: new_manifest,
                top_level_pin: entry.top_level_pin.clone(),
            },
        );
        Ok(true)
    }

    // -- Conflict checking ---------------------------------------------

    fn check_conflicts(&self) -> Result<(), ResolveError> {
        // Build name → version index from chosen.
        let chosen_versions: BTreeMap<&PackageName, &Version> =
            self.chosen.iter().map(|(n, e)| (n, &e.version)).collect();

        // Map address → name (canonical). Useful for resolving each
        // conflict declaration's target_address to a chosen name.
        let mut url_to_name: BTreeMap<String, &PackageName> = BTreeMap::new();
        for (name, entry) in &self.chosen {
            url_to_name.insert(entry.address.to_git_url(), name);
        }

        for decl in &self.conflicts {
            // Prefer the declaring entry's *current* version: a
            // conflict declaration from a no-longer-chosen version is
            // stale (additive accumulation tradeoff). Skip if the
            // declaring (name, version) pair isn't currently chosen.
            match self.chosen.get(&decl.declaring_name) {
                Some(entry) if entry.version == decl.declaring_version => {}
                _ => continue,
            }

            let target_url = match Address::parse(&decl.target_address) {
                Ok(a) => a.to_git_url(),
                Err(_) => continue, // Malformed conflict address — silently skip.
            };
            let Some(target_name) = url_to_name.get(&target_url) else {
                continue;
            };
            let Some(target_version) = chosen_versions.get(target_name) else {
                continue;
            };

            if decl.target_constraint.matches(target_version) {
                return Err(ResolveError::ManifestConflict {
                    declaring_name: decl.declaring_name.clone(),
                    declaring_version: decl.declaring_version.clone(),
                    target_name: (*target_name).clone(),
                    target_version: (*target_version).clone(),
                    target_constraint: decl.target_constraint.clone(),
                });
            }
        }
        Ok(())
    }

    // -- Plan emission --------------------------------------------------

    fn into_plan(self) -> Result<ResolvePlan, ResolveError> {
        // Build a name → entry map plus name → dep-name set for topo
        // sort. Determinism: BTreeMap iteration + dep-name BTreeSet.
        let entries = self.chosen;

        // Map address-url → name once so each manifest's deps can
        // resolve to chosen names.
        let mut url_to_name: BTreeMap<String, PackageName> = BTreeMap::new();
        for (name, entry) in &entries {
            url_to_name.insert(entry.address.to_git_url(), name.clone());
        }

        let mut deps_of: BTreeMap<PackageName, BTreeSet<PackageName>> = BTreeMap::new();
        for (name, entry) in &entries {
            let mut set: BTreeSet<PackageName> = BTreeSet::new();
            for dep in &entry.manifest.dependencies {
                let dep_url = match Address::parse(&dep.address) {
                    Ok(a) => a.to_git_url(),
                    Err(_) => continue,
                };
                if let Some(dep_name) = url_to_name.get(&dep_url) {
                    set.insert(dep_name.clone());
                }
            }
            deps_of.insert(name.clone(), set);
        }

        // Kahn's topological sort. `BTreeMap` + `BTreeSet` keep
        // tie-breaking deterministic (alphabetical by package name).
        let mut indegree: BTreeMap<PackageName, usize> =
            entries.keys().map(|n| (n.clone(), 0)).collect();
        for deps in deps_of.values() {
            for d in deps {
                if let Some(slot) = indegree.get_mut(d) {
                    *slot += 1;
                }
            }
        }

        // Note: edge direction. We treat "X depends on Y" as an edge
        // from X to Y. Topological order with Kahn's algorithm needs
        // dependencies *before* dependents, which means we process
        // nodes whose in-degree (number of dependents) is zero —
        // i.e., leaves of the depender graph, which are the most
        // fundamental dependencies. We then peel layers outward.
        //
        // Reframe: indegree[Y] = number of X such that X depends on Y.
        // No, that's backwards. Indegree[Y] = number of edges pointing
        // *into* Y. If "X depends on Y" is X → Y, then indegree[Y]
        // counts dependents. We want to emit Y first when nothing
        // points to it from a not-yet-emitted node. Reset.
        //
        // Cleaner: indegree[X] = count of X's outgoing edges still
        // pending = number of X's deps not yet emitted. Start with
        // indegree[X] = |deps_of[X]|; each time we emit Y, decrement
        // indegree of every X that depends on Y. Emit X when its
        // indegree hits 0.
        let mut indegree: BTreeMap<PackageName, usize> = deps_of
            .iter()
            .map(|(n, deps)| (n.clone(), deps.len()))
            .collect();

        // Reverse adjacency: dependents[Y] = set of X where X depends on Y.
        let mut dependents: BTreeMap<PackageName, BTreeSet<PackageName>> = BTreeMap::new();
        for (x, deps) in &deps_of {
            for y in deps {
                dependents.entry(y.clone()).or_default().insert(x.clone());
            }
        }

        let mut ready: BTreeSet<PackageName> = indegree
            .iter()
            .filter(|&(_, d)| *d == 0)
            .map(|(n, _)| n.clone())
            .collect();
        let mut order: Vec<PackageName> = Vec::with_capacity(entries.len());

        while let Some(next) = ready.iter().next().cloned() {
            ready.remove(&next);
            order.push(next.clone());
            if let Some(succs) = dependents.get(&next) {
                for x in succs {
                    if let Some(d) = indegree.get_mut(x) {
                        *d = d.saturating_sub(1);
                        if *d == 0 {
                            ready.insert(x.clone());
                        }
                    }
                }
            }
        }

        if order.len() != entries.len() {
            // Should be impossible given monotonic-version-decrease
            // termination, but guarded for safety: a cycle in the
            // dependency graph after fixed-point. Surface as a typed
            // error rather than panic.
            let cycle: Vec<PackageName> = entries
                .keys()
                .filter(|n| !order.contains(n))
                .cloned()
                .collect();
            return Err(ResolveError::DependencyCycle {
                participants: cycle,
            });
        }

        let mut entries = entries;
        let packages: Vec<ResolvedPackage> = order
            .into_iter()
            .map(|name| {
                let entry = entries.remove(&name).expect("name in order is in entries");
                ResolvedPackage {
                    name,
                    address: entry.address,
                    commit: entry.commit,
                    version: entry.version,
                    manifest: entry.manifest,
                    top_level_pin: entry.top_level_pin,
                }
            })
            .collect();

        Ok(ResolvePlan { packages })
    }

    // -- Helpers --------------------------------------------------------

    fn ensure_repo(&mut self, url: &str) -> Result<(), ResolveError> {
        if self.repo_paths.contains_key(url) {
            return Ok(());
        }
        let path = self
            .fetcher
            .fetch(url)
            .map_err(|source| ResolveError::Fetch {
                address: Address::Url(url.to_string()),
                source,
            })?;
        self.repo_paths.insert(url.to_string(), path);
        Ok(())
    }

    fn list_tags(
        &self,
        repo: &std::path::Path,
        url: &str,
        source: &Source,
        address: &Address,
    ) -> Result<Vec<TagCandidate>, ResolveError> {
        let raw = self
            .fetcher
            .list_tags(repo)
            .map_err(|source_err| ResolveError::Fetch {
                address: address.clone(),
                source: source_err,
            })?;

        // Filter to parseable semver tags and sort descending.
        // Tags that don't parse as semver are dropped silently —
        // pmacs's semver discipline says the user's version
        // constraint matches against semver-shaped tags only. The
        // installer applies the same convention.
        let _ = url;
        let _ = source;
        let mut tagged: Vec<TagCandidate> = raw
            .into_iter()
            .filter_map(|t| {
                let v = parse_tag_as_version(&t)?;
                Some(TagCandidate { tag: t, version: v })
            })
            .collect();
        tagged.sort_by(|a, b| b.version.cmp(&a.version));
        Ok(tagged)
    }

    fn pick_best_version(
        &mut self,
        repo: &std::path::Path,
        url: &str,
        address: &Address,
        candidates: &[TagCandidate],
        constraints: &[(Source, VersionReq)],
    ) -> Result<ChosenTag, ResolveError> {
        if candidates.is_empty() {
            return Err(ResolveError::NoTagsAvailable {
                address: address.clone(),
                sources: constraints
                    .iter()
                    .map(|(s, r)| (s.clone(), r.clone()))
                    .collect(),
            });
        }

        // Phase 1: filter by constraint intersection. This eliminates
        // tags that can't satisfy any version constraint regardless
        // of pmacs_required, so we don't fetch their manifests.
        let after_user: Vec<&TagCandidate> = candidates
            .iter()
            .filter(|c| constraints.iter().all(|(_, req)| req.matches(&c.version)))
            .collect();
        if after_user.is_empty() {
            return Err(ResolveError::NoVersionMatchesConstraints {
                address: address.clone(),
                constraints: constraints.to_vec(),
                available: candidates.iter().map(|c| c.tag.clone()).collect(),
            });
        }

        // Phase 2a: lockfile hint. If a hint version is in the
        // user-filtered candidate set and its manifest is
        // pmacs-compatible, prefer it over the highest-version
        // selection. This is the `UpdatePolicy::UpdateOne` cascade
        // brake — non-target packages stay at lockfile versions
        // unless current constraints force them to move.
        if let Some(hint) = self.prefer_versions.get(url).cloned() {
            if let Some(cand) = after_user.iter().find(|c| c.version == hint) {
                let manifest = self
                    .read_manifest(url, repo, &cand.commit_for_tag())?
                    .clone();
                if manifest.pmacs_required.matches(self.pmacs_version) {
                    return Ok(ChosenTag {
                        tag: cand.tag.clone(),
                        commit: cand.commit_for_tag(),
                        version: cand.version.clone(),
                    });
                }
                // Hinted version is no longer pmacs-compatible (e.g.
                // user upgraded pmacs and the locked version requires
                // an older one). Fall through to highest-version
                // selection.
            }
        }

        // Phase 2b: walk highest-first, fetch manifest lazily, return
        // first that satisfies pmacs_required. Lazy fetch matters: in
        // the common case the latest tag is compatible and we fetch
        // exactly one manifest.
        let mut incompatible: Vec<(String, VersionReq)> = Vec::new();
        for cand in after_user {
            let manifest = self
                .read_manifest(url, repo, &cand.commit_for_tag())?
                .clone();
            if manifest.pmacs_required.matches(self.pmacs_version) {
                return Ok(ChosenTag {
                    tag: cand.tag.clone(),
                    commit: cand.commit_for_tag(),
                    version: cand.version.clone(),
                });
            }
            incompatible.push((cand.tag.clone(), manifest.pmacs_required.clone()));
        }
        Err(ResolveError::NoVersionMatchesPmacsRequirement {
            address: address.clone(),
            running: self.pmacs_version.clone(),
            constraints: constraints.to_vec(),
            incompatible,
        })
    }

    fn read_manifest(
        &mut self,
        url: &str,
        repo: &std::path::Path,
        commit: &str,
    ) -> Result<&PackageManifest, ResolveError> {
        let key = (url.to_string(), commit.to_string());
        if !self.manifests.contains_key(&key) {
            let bytes = self
                .fetcher
                .show_blob(repo, commit, "pmacs.toml")
                .map_err(|source| ResolveError::Fetch {
                    address: Address::Url(url.to_string()),
                    source,
                })?;
            let text = std::str::from_utf8(&bytes).map_err(|_| ResolveError::ManifestNotUtf8 {
                url: url.to_string(),
                commit: commit.to_string(),
            })?;
            let manifest =
                PackageManifest::from_toml(text).map_err(|source| ResolveError::Manifest {
                    url: url.to_string(),
                    commit: commit.to_string(),
                    source,
                })?;
            self.manifests.insert(key.clone(), manifest);
        }
        Ok(&self.manifests[&key])
    }

    fn check_pinned_pmacs_required(
        &self,
        address: &Address,
        manifest: &PackageManifest,
        commit: &str,
        pin_kind: &'static str,
        pin_value: &str,
    ) -> Result<(), ResolveError> {
        if manifest.pmacs_required.matches(self.pmacs_version) {
            return Ok(());
        }
        Err(ResolveError::PinnedRevisionIncompatibleWithPmacs {
            address: address.clone(),
            commit: commit.to_string(),
            pin_kind,
            pin_value: pin_value.to_string(),
            required: manifest.pmacs_required.clone(),
            running: self.pmacs_version.clone(),
        })
    }

    fn enqueue_dependencies(
        &mut self,
        name: &PackageName,
        version: &Version,
        manifest: &PackageManifest,
    ) {
        for dep in &manifest.dependencies {
            self.pending.push_back(PendingEdge {
                address: dep.address.clone(),
                constraint: dep.version.clone(),
                source: Source::DependencyOf {
                    name: name.clone(),
                    version: version.clone(),
                },
            });
        }
    }

    fn stash_conflicts(
        &mut self,
        name: &PackageName,
        version: &Version,
        manifest: &PackageManifest,
    ) {
        for conflict in &manifest.conflicts {
            self.conflicts.push(ConflictDecl {
                declaring_name: name.clone(),
                declaring_version: version.clone(),
                target_address: conflict.address.clone(),
                target_constraint: conflict.version.clone(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Internal value types
// ---------------------------------------------------------------------------

/// A semver-shaped tag that survived parsing, carrying its parsed
/// version. Tags are pre-sorted descending by version.
#[derive(Debug, Clone)]
struct TagCandidate {
    tag: String,
    version: Version,
}

impl TagCandidate {
    /// Tags are commit-ish in git: `git show <tag>:path` works
    /// uniformly. Resolution to a 40-char SHA is deferred until the
    /// installer materializes the snapshot — the resolver works
    /// against tags throughout, and the lockfile records the commit
    /// the installer eventually materializes.
    fn commit_for_tag(&self) -> String {
        self.tag.clone()
    }
}

#[derive(Debug, Clone)]
struct ChosenTag {
    #[allow(dead_code)]
    tag: String,
    commit: String,
    version: Version,
}

/// Parse a Git tag as a semver version. Tolerates a `v` prefix
/// (`v1.2.3` → `1.2.3`); rejects anything that isn't otherwise
/// valid semver.
fn parse_tag_as_version(tag: &str) -> Option<Version> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(stripped).ok()
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by [`Resolver::resolve`].
///
/// `Display` for each variant produces a self-contained, user-facing
/// message — read in a CI log without context, the user can see what
/// to do. Lua wrappers (M7.7+) surface these strings directly without
/// reformatting.
#[derive(Debug, Error)]
pub enum ResolveError {
    /// Two top-level requests resolved to the same canonical name.
    /// Pick one.
    #[error(
        "two top-level package requests resolved to the same name `{name}`. \
         Each top-level address must produce a distinct package name; \
         remove one of the duplicate requests.",
        name = name.as_str(),
    )]
    DuplicateTopLevel {
        /// The conflicting name.
        name: PackageName,
    },

    /// A `ResolveRequest` carried [`InstallPin::Local`].
    /// `install_local` (T M8.1c) installs working-tree symlinks and
    /// never goes through the resolver; this error is a
    /// programming-error safeguard for callers that build a request
    /// from a roster entry without filtering Local pins.
    #[error(
        "InstallPin::Local cannot be resolved; use pmacs.packages.install_local \
         for working-tree installs"
    )]
    LocalPinNotResolvable,

    /// `dependencies[*].address` from a manifest failed to parse.
    #[error(
        "manifest of {declared_by} declared dependency on `{address}`, but the \
         address failed to parse: {source}",
        declared_by = source_display(declared_by),
    )]
    AddressInDependency {
        /// The bad address string.
        address: String,
        /// Underlying parse error.
        #[source]
        source: AddressError,
        /// Which manifest declared the dependency.
        declared_by: Source,
    },

    /// Fetcher (clone, fetch, rev-parse, archive, list-tags) failed.
    #[error("fetch error for {address_display}: {source}", address_display = address.to_git_url())]
    Fetch {
        /// Address that was being fetched.
        address: Address,
        /// Underlying fetch error.
        #[source]
        source: FetchError,
    },

    /// Reading or parsing `pmacs.toml` at a specific commit failed.
    #[error("manifest at {url}@{commit}: {source}")]
    Manifest {
        /// Repository URL.
        url: String,
        /// Commit hash.
        commit: String,
        /// Parse / IO error.
        #[source]
        source: ManifestError,
    },

    /// `pmacs.toml` blob at the specified commit was not valid UTF-8.
    /// Surfaced separately from [`Self::Manifest`] because the repair
    /// is "fix the upstream commit," not "fix the manifest schema."
    #[error(
        "manifest at {url}@{commit}: pmacs.toml is not valid UTF-8 at this revision. \
         Pin to a different revision or contact the upstream maintainer."
    )]
    ManifestNotUtf8 {
        /// Repository URL.
        url: String,
        /// Commit hash.
        commit: String,
    },

    /// The repository has no semver-shaped tags. Surfaced when the
    /// resolver was asked to resolve a version constraint against an
    /// empty tag set.
    #[error(
        "no semver-shaped tags published at {address_display}. {sources_msg} \
         Either publish a tag, or pin to a branch (`branch = \"main\"`) \
         or specific commit (`commit = \"<sha>\"`).",
        address_display = address.to_git_url(),
        sources_msg = format_constraint_paths(sources),
    )]
    NoTagsAvailable {
        /// The address with no usable tags.
        address: Address,
        /// All constraint paths that asked for a version of this package.
        sources: Vec<(Source, VersionReq)>,
    },

    /// Tags exist but none satisfies the union of version
    /// constraints. Distinct from [`Self::NoVersionMatchesPmacsRequirement`]
    /// so the user can tell "loosen your constraint" from "upgrade
    /// pmacs" apart.
    #[error(
        "no version of {address_display} satisfies all constraints:\n{paths}\n\
         available tags: {tags}",
        address_display = address.to_git_url(),
        paths = format_constraint_paths(constraints),
        tags = format_tag_list(available),
    )]
    NoVersionMatchesConstraints {
        /// The over-constrained address.
        address: Address,
        /// All constraint paths on this package.
        constraints: Vec<(Source, VersionReq)>,
        /// Tags available at the address (after semver-parse filter).
        available: Vec<String>,
    },

    /// Tags satisfy the user / transitive constraints, but none of
    /// them are compatible with the running pmacs version.
    #[error(
        "no version of {address_display} is compatible with pmacs {running}. \
         {constraint_msg}\
         versions matching version constraints but incompatible with pmacs:\n{incompat}",
        address_display = address.to_git_url(),
        constraint_msg = format_constraint_msg_when_present(constraints),
        incompat = format_incompatible_list(incompatible),
    )]
    NoVersionMatchesPmacsRequirement {
        /// The address with no pmacs-compatible version.
        address: Address,
        /// The running pmacs version.
        running: Version,
        /// Constraint set that the candidates satisfied.
        constraints: Vec<(Source, VersionReq)>,
        /// Each candidate that matched version constraints but had a
        /// `pmacs_required` that excluded this pmacs.
        incompatible: Vec<(String, VersionReq)>,
    },

    /// User pinned a top-level package to a specific branch or commit
    /// whose manifest's `pmacs_required` excludes the running pmacs.
    #[error(
        "{address_display} pinned to {pin_kind} `{pin_value}` (commit {commit_short}) \
         requires pmacs {required}, but this pmacs is {running}. \
         Upgrade pmacs, or pin to a different {pin_kind}.",
        address_display = address.to_git_url(),
        commit_short = short_sha(commit),
    )]
    PinnedRevisionIncompatibleWithPmacs {
        /// The pinned address.
        address: Address,
        /// The full commit the pin resolved to.
        commit: String,
        /// `"branch"` or `"commit"`.
        pin_kind: &'static str,
        /// The user's pin value.
        pin_value: String,
        /// The package's `pmacs_required`.
        required: VersionReq,
        /// The running pmacs version.
        running: Version,
    },

    /// User pinned a package to a branch or commit, and a transitive
    /// dependency in the resolved graph adds a constraint that the
    /// pinned version doesn't satisfy.
    #[error(
        "{name} pinned to version {version}, but {via} requires `{constraint}`. \
         Either change the pin, or update the dependent to accept this version.",
        name = name.as_str(),
        via = via.display(),
    )]
    PinnedRevisionViolatesConstraint {
        /// The pinned package.
        name: PackageName,
        /// The pinned version.
        version: Version,
        /// Where the conflicting constraint came from.
        via: Source,
        /// The constraint the pinned version doesn't satisfy.
        constraint: VersionReq,
    },

    /// A manifest's `conflicts` declaration matched a chosen package.
    #[error(
        "package conflict: {declaring_name} @ {declaring_version} declares conflict \
         with {target_name}, and {target_name} resolved to {target_version} \
         (matches conflict clause `{target_constraint}`). \
         Drop one of these packages, or pin them to non-conflicting versions.",
        declaring_name = declaring_name.as_str(),
        target_name = target_name.as_str(),
    )]
    ManifestConflict {
        /// Package whose manifest declared the conflict.
        declaring_name: PackageName,
        /// Version of that package at which the conflict was declared.
        declaring_version: Version,
        /// Package targeted by the conflict declaration.
        target_name: PackageName,
        /// Resolved version of the target package.
        target_version: Version,
        /// Constraint clause from the conflict declaration.
        target_constraint: VersionReq,
    },

    /// Topological sort detected a cycle. Should be unreachable given
    /// the additive monotonic-decrease termination, but guarded
    /// rather than panicking.
    #[error(
        "dependency cycle among packages: {participants:?}. \
         The resolver cannot order these packages for installation."
    )]
    DependencyCycle {
        /// Participants in the cycle.
        participants: Vec<PackageName>,
    },

    /// `resolve_with_policy` was called with [`UpdatePolicy::Frozen`]
    /// but `requests` includes an address with no matching lockfile
    /// entry.
    #[error(
        "package {address_display} is not in the lockfile. \
         A frozen install cannot add new packages. Either run \
         `pmacs.packages.update()` to regenerate the lockfile with this \
         package included, or remove the package from your install list.",
        address_display = address.to_git_url(),
    )]
    FrozenRequestMissing {
        /// The address that has no lockfile entry.
        address: Address,
    },

    /// `resolve_with_policy` was called with a policy that requires a
    /// lockfile, but `lockfile` was `None`.
    #[error(
        "this update policy requires a lockfile, but none was provided. \
         Either pass an existing lockfile, or use `UpdatePolicy::UpdateAll` \
         for a fresh resolve."
    )]
    LockfileRequired,

    /// A lockfile-aware path produced an underlying [`LockfileError`].
    #[error("lockfile: {0}")]
    Lockfile(#[from] LockfileError),
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

/// Format a constraint-source list as a bulleted block, one per line.
fn format_constraint_paths(constraints: &[(Source, VersionReq)]) -> String {
    if constraints.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (source, req) in constraints {
        let _ = writeln!(out, "  - required by {} as `{}`", source.display(), req);
    }
    // Trim trailing newline for cleaner Display interpolation.
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn format_constraint_msg_when_present(constraints: &[(Source, VersionReq)]) -> String {
    if constraints.is_empty() {
        String::new()
    } else {
        format!(
            "Constraints requiring this package:\n{}\n",
            format_constraint_paths(constraints),
        )
    }
}

/// Format a tag list with truncation when long. Long tag lists make
/// error messages unreadable; truncating with the count preserves the
/// useful information.
const TAG_LIST_DISPLAY_CAP: usize = 10;

fn format_tag_list(tags: &[String]) -> String {
    let total = tags.len();
    if total == 0 {
        return "(none)".to_string();
    }
    if total <= TAG_LIST_DISPLAY_CAP {
        let body: Vec<String> = tags.iter().map(|t| format!("\"{t}\"")).collect();
        return format!("[{}]", body.join(", "));
    }
    // Show the first few and the last few, plus the total count.
    let show_each_end = TAG_LIST_DISPLAY_CAP / 2;
    let head: Vec<String> = tags
        .iter()
        .take(show_each_end)
        .map(|t| format!("\"{t}\""))
        .collect();
    let tail: Vec<String> = tags
        .iter()
        .skip(total - show_each_end)
        .map(|t| format!("\"{t}\""))
        .collect();
    format!(
        "[{}, ..., {}] ({total} total)",
        head.join(", "),
        tail.join(", "),
    )
}

fn format_incompatible_list(incompatible: &[(String, VersionReq)]) -> String {
    if incompatible.is_empty() {
        return "(none)".to_string();
    }
    let mut out = String::new();
    for (tag, required) in incompatible {
        let _ = writeln!(out, "  - {tag} requires pmacs `{required}`");
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn source_display(source: &Source) -> String {
    source.display()
}

fn short_sha(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tag_strips_v_prefix() {
        let v = parse_tag_as_version("v1.2.3").expect("parse");
        assert_eq!(v, Version::new(1, 2, 3));
    }

    #[test]
    fn parse_tag_accepts_bare_version() {
        let v = parse_tag_as_version("1.2.3").expect("parse");
        assert_eq!(v, Version::new(1, 2, 3));
    }

    #[test]
    fn parse_tag_rejects_non_semver() {
        // semver is strict: `1.2` (no patch) is not valid.
        assert!(parse_tag_as_version("v1.2").is_none());
        assert!(parse_tag_as_version("nightly").is_none());
        assert!(parse_tag_as_version("v-foo").is_none());
    }

    #[test]
    fn format_tag_list_under_cap_lists_all() {
        let tags = vec![
            "v1.0.0".to_string(),
            "v1.1.0".to_string(),
            "v2.0.0".to_string(),
        ];
        let out = format_tag_list(&tags);
        assert!(out.contains("v1.0.0"));
        assert!(out.contains("v2.0.0"));
        assert!(!out.contains("total"));
    }

    #[test]
    fn format_tag_list_over_cap_truncates_with_count() {
        let tags: Vec<String> = (0..47).map(|i| format!("v0.{i}.0")).collect();
        let out = format_tag_list(&tags);
        assert!(out.contains("47 total"));
        assert!(out.contains("..."));
        // First and last fragments present.
        assert!(out.contains("v0.0.0"));
        assert!(out.contains("v0.46.0"));
    }

    #[test]
    fn format_tag_list_empty_says_none() {
        assert_eq!(format_tag_list(&[]), "(none)");
    }

    #[test]
    fn source_display_top_level() {
        assert_eq!(Source::TopLevel.display(), "(top-level)");
    }

    #[test]
    fn source_display_dependency_of() {
        let s = Source::DependencyOf {
            name: PackageName::new("foo").expect("name"),
            version: Version::new(1, 2, 3),
        };
        assert_eq!(s.display(), "foo @ 1.2.3");
    }

    #[test]
    fn short_sha_truncates_to_seven() {
        assert_eq!(short_sha("0123456789abcdef"), "0123456");
        assert_eq!(short_sha("abc"), "abc");
    }
}
