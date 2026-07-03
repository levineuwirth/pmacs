# Package-manager hardening — framing + as-built

Five audit findings in `src/packages/`, taken as one sweep: one Medium
correctness issue (F-005) and four Lows (F-009–F-012) that range from a
security-adjacent cache-key upgrade to dead-code deletion. All in the
`pmacs` core crate; no wire-protocol or frontend change.

- **F-005** (Medium) — install dirs are named by package *basename*, and
  require-lookup routes by basename. Two distinct packages `owner/magit`
  and `other/magit` collapse to one install dir; most-recent-install
  silently wins.
- **F-009** (Low) — the fetch bare-mirror cache dir is keyed by 64-bit
  FNV-1a of the URL — trivially collidable, and URLs are
  attacker-adjacent.
- **F-010** (Low) — on a git subprocess timeout, `run_with_timeout`
  returns before joining the stdout/stderr drain threads (they're joined
  only on the normal path).
- **F-011** (Low) — `ResolvedPackage.commit` is documented "Full
  40-character commit hash" but `commit_for_tag()` puts a **tag string**
  there; the resolver deliberately works against commit-ishes and defers
  SHA resolution to the installer.
- **F-012** (Low) — the resolver's topological sort builds an `indegree`
  map, reasons in comments that it's backwards, and rebuilds it — leaving
  the first block dead.

## What the recon established

- **F-005 must not break the loader's *intended* override.** The loader
  documents that a project-scope install may share a basename with a
  user-scope one, and the Lua searcher picks most-recent-first *on
  purpose* (`loader.rs:43`). So the collision to reject is the one with no
  override semantics: **two distinct package names inside a single resolve
  plan** sharing a basename — they'd collide on disk with no way to tell
  which the caller meant. `into_plan` (`resolver.rs:678`) builds the plan
  and holds every name at once — the one place that can see the collision.
  The install marker (`.pmacs-install`) records only the commit, not the
  canonical name, so cross-resolve install-time detection would need a
  marker-format change — deferred.
- **F-009 has `sha2` already.** `sha2 = "0.10"` is a direct dep (M7.6
  lockfile hashing, `lockfile.rs:84`), so SHA-256 is a swap, not a new
  dependency. The cache key is a dir name; changing it re-clones once
  (it's a cache) — acceptable.
- **F-010 is reap-then-join.** After `kill()` + `wait()` the child's pipes
  close, so the drain threads finish promptly and are safe to join. The
  fix is to reap on *every* exit path and join at a single point.
- **F-011's field genuinely holds a commit-ish** and `ResolvePlan` is
  never serialized (only feeds `Lockfile::from_plan`), so a rename is
  serde-safe. Field access is concrete-typed, so a compiler-driven rename
  touches exactly the `ResolvedPackage` sites, not the other `.commit`
  fields (`ChosenTag`, the internal entry, `InstalledPackage`,
  `LockedPackage`).

## The rules

**Q#PM1 — F-005: reject same-basename distinct names in one plan.** After
`into_plan` assembles `packages`, group by `package_basename(name)`
(`installer.rs:1046`, already `pub(crate)`); if any basename maps to more
than one distinct `PackageName`, return a new
`ResolveError::BasenameCollision { basename, names }`. This blocks the
install before it can silently shadow, without touching the loader's
cross-scope override behavior. Install-time (cross-resolve) detection is
named-deferred (needs the canonical name in the marker).

**Q#PM2 — F-009: SHA-256 the cache key.** Replace `fnv1a_hex` with
`sha256_hex` (`sha2::Sha256`, full 64-char hex) for the bare-mirror dir /
lock names. Keep `normalize_url` in front (case/`.git`/trailing-slash
folding) so the *same* repo still maps to one entry; only the hash
function changes. Update the three hash unit tests.

**Q#PM3 — F-010: reap on every path, join once.** Restructure
`run_with_timeout` so the wait loop `break`s with a `Result<ExitStatus,
FetchError>` — killing + reaping the child on the timeout and `try_wait`-
error paths — then joins both drain threads unconditionally and only then
propagates the error. No detached readers survive a timeout.

**Q#PM4 — F-011: rename `commit` → `revision`, tell the truth.** Rename
the field and document it as "a commit-ish (a SHA, or a tag/branch name);
the installer/lockfile resolves it to a concrete SHA." Compiler-driven:
fix each flagged `ResolvedPackage` read (`resolver`/`lockfile`/
`lua_bindings`). The Lua-visible record key stays `"commit"` (user API
unchanged); only the Rust contract gets honest.

**Q#PM5 — F-012: delete the dead indegree block.** Remove the first
`indegree` construction (`resolver.rs:707-715`) and collapse the
three-paragraph "reset / no, backwards / cleaner" narration into one
comment stating the final algorithm (outgoing-edge counts + a `dependents`
reverse-adjacency, peeled by Kahn's). Pure cleanup — the existing
topo-sort tests are the guard.

## Categorical bets

- **Reject, don't guess (F-005).** With two same-basename packages in one
  plan and no override semantics to disambiguate, a clear error beats
  silently installing whichever lands last. A namespace-preserving layout
  is the real answer later; failing loud is the right v0.1 floor.
- **Crypto hash for an adversary-adjacent key (F-009).** URLs are
  effectively attacker-controlled; only a cryptographic digest resists a
  *deliberate* cache-path collision. `sha2` is already in the tree.
- **A rename is worth it over a doc patch (F-011).** The name is the trap
  ("trusts `.commit` as a SHA"); renaming removes it at the type level,
  and the compiler makes the change safe and exhaustive.

## Validation implication

All five are unit-testable in-crate (no GPU, no daemon): a resolve plan
with a synthesized basename collision errors (F-005); `sha256_hex` is
stable/normalized (F-009); a timeout leaves no unjoined threads and still
returns `Timeout` (F-010 — assert via the existing timeout test path);
rename is compile-checked + existing resolve/lockfile tests (F-011); topo
order unchanged (F-012, existing tests). Runs under both Lua flavors in
CI.

## As-built

Landed as framed; all five in `src/packages/` (F-011 also touches
`lua_bindings.rs` + acceptance tests via the rename). No serde/wire change.

- **F-005** (`resolver.rs`): new `ResolveError::BasenameCollision {
  basename, names }`; a free `find_basename_collision(names)` (testable,
  used by `into_plan`) groups the plan's names by `package_basename` and
  returns the first basename with >1 distinct name. Two unit tests; the
  existing `roster_lookup_picks_most_recent_on_basename_collision` loader
  test still passes, confirming the intended cross-scope override is
  untouched.
- **F-009** (`fetcher.rs`): `fnv1a_hex` → `sha256_hex` (`sha2::Sha256`,
  64-char hex) for the bare-mirror dir/lock key; `normalize_url` unchanged
  in front. Hash test asserts stability, normalization, 64-hex shape, and
  distinctness.
- **F-010** (`fetcher.rs`): `run_with_timeout` now `break`s the wait loop
  with a `Result`, reaping the child (`kill` + `wait`) on both the timeout
  and `try_wait`-error paths, then joins both drain threads at a single
  point before propagating — no detached readers survive a timeout. The
  existing `timeout_kills_long_running_command` test covers the path.
- **F-011** (`resolver.rs` + `lockfile.rs` + `lua_bindings.rs` + m7_5/m7_6/
  m7_10 tests): `ResolvedPackage.commit` → `revision`, documented as a
  commit-ish. Compiler-driven rename hit exactly the `ResolvedPackage`
  sites; the Lua-visible record key stays `"commit"`, and the other
  structs' `commit` fields (`ChosenTag`, internal entry, `InstalledPackage`,
  `LockedPackage`) were correctly left alone.
- **F-012** (`resolver.rs`): deleted the dead first `indegree` map + the
  three-paragraph "reset/backwards/cleaner" narration; one comment now
  states the actual algorithm.

Validated: `cargo fmt` clean; `clippy --all-targets` clean under **both**
Lua flavors (luajit + lua54); 1436 lib unit tests pass, incl. the new
F-005/F-009 tests, the F-010 timeout test, and the loader override test.

## Deferred (named)

- **F-005 install-time collision detection.** Catching a collision across
  *separate* resolves (install `owner/magit`, later `other/magit`) needs
  the canonical name recorded in `.pmacs-install` and an installer check —
  a marker-format change out of this batch's scope.
- **Namespace-preserving install layout.** The structural fix for F-005
  (install under `<owner>/<name>/`, require by canonical name) is a larger
  design change; rejecting is the interim floor.
