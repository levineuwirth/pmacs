# Framing — Distribution Stage 1: binaries on tag

**Revision 3.** Status: **implemented** on branch `distribution-stage1`,
based on `githubsucks/main` @ `4984169` (#210). Approved at revision 2.

**Revision 2 → 3** records two implementation findings, not a new design
round:

- **Layer 2 of the binary exclusion is load-bearing, and this is now
  demonstrated rather than argued** (§1.2b). The argument for it was
  hypothetical; building the branch produced the exact case it guards
  against, on the first try.
- **The glibc floor is machine-checked** (§1.6a), which is stronger than
  acceptance 7's original container test and runs on every release
  instead of once at RC time.
- **The version bump exposed a real product defect** (§1.3a): the daemon
  reported the *protocol* crate's version to every attached frontend
  under a field named `pmacs_version`. It was invisible while the two
  crates happened to share a number, and Q#D1's decision to diverge them
  is what surfaced it. Fixed here, because shipping a release whose
  daemon misreports its own version is precisely what this stage
  exists to prevent.

`.github/workflows/` contains exactly one workflow and it is test-only.
**There is no release job, no artifact upload, no tags-to-binaries path.**
Installing pmacs means `git clone` plus knowing the feature-flag matrix.

**Scope is deliberately one stage: produce binaries when a tag is
pushed, and attach them to a GitHub Release.** Channels, rollback,
update-in-place, signing, and §17's first-launch tool detection are all
**out of scope and named in §5**. This establishes a baseline, not the
arc.

**Revision 1 → 2**, all from review:

- **Q#D2 decided with a correction to revision 1's reasoning.** It is
  `ubuntu-22.04` with an explicit **glibc ≥ 2.35** floor — and revision 1
  was wrong to imply an older runner reaches RHEL 9. It does not (§1.6).
- **Q#D1 decided**, with acceptance strengthened to assert *both*
  binaries report 1.1.0 (§1.3, acceptance 4).
- **The binary-exclusion mechanism is now two-layered**, not one:
  explicit `--bin` targets *and* an explicit staged asset list (§1.2).
- **Revision 1 mis-stated `pmacs --gpu`'s failure mode**, and the
  corrected version changes *why* co-location is required rather than
  whether (§1.2a).
- **Both runners are pinned**, not just Linux (§1.8).
- **The RC is cut after merge from the merge SHA** and the final tag
  reuses that same verified SHA (§7).

---

## 0. Coherence impact (COHERENCE §20)

**Squarely §17 / §20 Priority 8: it completes journey step 1 without
adding an interaction island, a config surface, or a background-work
model.**

- **Journey step touched: step 1 (install)** — the one step no prior arc
  could reach.
- **Concern: §17 Distribution**, graded "missing — zero release
  machinery exists." This moves it to **Partial**, not Strong: binaries
  on tag is the first of the seven things §17 asks for.
- **Interaction islands added:** none.
- **Config registry adoption:** none — this stage introduces no setting.
- **Background-work attribution:** none — no worker, job, or process.
- **Why now:** §20 P8 says every other priority's value is invisible
  until this exists, and its prerequisite — a tree whose tests actually
  run — was only satisfied by #209. Before that, a release would have
  shipped from a corpus half of which CI had never compiled.

---

## 1. Ground truth (measured at `c5f7501`, 2026-08-01)

### 1.1 What exists

Nothing. One workflow, `ci.yml`, test-only; the only `release` strings
in it are `cargo test --release` flags.

### 1.2 The binary set is NOT what a glob would produce

`Cargo.toml` declares two `[[bin]]` targets, `pmacs` and `pmacs-audit`.
But **cargo also auto-discovers `src/bin/*.rs`**, so a release build
additionally produces `pmacs_fake_lsp` and `pmacs_fake_mcp` — test
fixtures whose purpose is to be spawned by acceptance suites. Plus
`pmacs-gpu` from its own package.

**Five binaries can land in `target/release`, and three must never
ship.**

| binary | ship? | why |
|---|---|---|
| `pmacs` | **yes** | the editor |
| `pmacs-gpu` | **yes** | the GPU frontend |
| `pmacs-audit` | **no** | audits pmacs source against the v1.0 lint rules — a contributor tool with no answer to "what is this for" |
| `pmacs_fake_lsp` | **never** | test fixture |
| `pmacs_fake_mcp` | **never** | test fixture |

**Exclusion is two-layered, deliberately.** Avoiding an upload glob is
not sufficient on its own:

1. **Build explicit `--bin` targets** — `--bin pmacs` and the
   `pmacs-gpu` package — so the unwanted binaries are not produced by
   the release build at all.
2. **Stage an explicit asset list** — copy named files into a staging
   directory and archive *that*, so the archive's contents are a
   decision rather than a directory's residue.

Layer 1 without layer 2 still archives whatever a cached
`target/release` happens to hold from an earlier step; layer 2 without
layer 1 relies on a list nobody re-checks when a new `src/bin/*.rs`
appears. Acceptance 2 asserts the **complete member list**, the
**executable bits**, and the **absence of all three** excluded binaries.

### 1.2b Layer 2 is load-bearing — demonstrated, not argued

Revision 2 justified the second layer with a hypothetical: "a cached
`target/release` can still hold binaries from an earlier build."
**Implementing the branch produced that case immediately.** After
running only

```sh
cargo build --release --bin pmacs --features crdt
cargo build --release -p pmacs-gpu
```

on a tree where earlier work had run `cargo test --release` for the M10
perf gates, `target/release` contained:

```
pmacs  pmacs-audit  pmacs_fake_lsp  pmacs_fake_mcp  pmacs-gpu
```

**All three forbidden binaries were present**, left by the earlier test
build, despite this build naming only two targets. `Swatinem/rust-cache`
restores exactly this kind of directory in CI, so the risk is not
theoretical there either.

An implementation that took layer 1 as sufficient and archived
`target/release` would have published a fake language server in the
first release. The three archive assertions are bite-verified: a
smuggled `pmacs_fake_lsp`, a missing `pmacs-gpu`, and a cleared
executable bit are each caught, with the honest archive passing.

### 1.6a The glibc floor is asserted, not trusted

Acceptance 7 originally proposed verifying the floor by running the
binary in containers. **The shipped check is stronger and cheaper**:
read the versioned-symbol requirements straight out of the binary and
fail the build when any exceeds the floor.

```sh
objdump -T <binary> | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV | tail -1
```

Why this beats the container test: it runs on **every** release rather
than once at RC time, it needs no network or images, and it fails at the
point of causation. Pinning `ubuntu-22.04` sets the floor but proves
nothing about the artifact — switching the job to `ubuntu-latest` would
otherwise ship binaries that fail to load with a bare `GLIBC_2.39 not
found` on a user's machine, with no clue which commit caused it. With
the assertion, that change fails in CI instead.

Bite-verified in both directions on a glibc 2.44 host, which stands in
for a mis-pinned runner: against a 2.35 floor both binaries are caught;
against a 2.44 floor both pass. Linux only — Mach-O has no equivalent
versioned-symbol scheme.

### 1.2a Why `pmacs` and `pmacs-gpu` must be co-located — corrected

Revision 1 said separating them makes `--gpu` "silently fail." **That is
wrong, and the real behaviour is better.** `gpu_binary`
(`src/main.rs:304`) prefers a **co-located** `pmacs-gpu` when that path
`is_file()`, otherwise falls back to the bare name `pmacs-gpu` for a
**PATH lookup**; if neither resolves, the error names both — *"sibling
… is absent and PATH lookup for pmacs-gpu failed"*.

So the requirement is not "otherwise it breaks quietly." It is that
**a release archive must be self-contained**: a user who unpacks it
somewhere not on `PATH` gets a working `--gpu` only if the two binaries
sit together. The fallback is a convenience for installed layouts, not a
substitute for shipping them as a unit.

### 1.3 The version is already incoherent, and a tag makes it visible

| crate | now | after |
|---|---|---|
| `pmacs` | 1.0.0 | **1.1.0** |
| `pmacs-gpu` | **0.0.1** | **1.1.0** |
| `pmacs-protocol` | 1.0.0 | **1.0.0** (unchanged) |

`pmacs-protocol` stays: it is the wire crate, carries publish-shaped
metadata, and versions on its own schedule — the protocol is v21 and
independent of the editor's release number.

Both shipped binaries report their own `env!("CARGO_PKG_VERSION")`:
`pmacs --version` at `src/main.rs:393`, and `pmacs-gpu --version` at
`pmacs-gpu/src/main.rs:667`, which prints `pmacs-gpu <ver> (protocol
v21)` — the protocol number is worth carrying into the release notes.

Tagging `v1.1.0` against the tree as it stands would publish a release
containing a `pmacs` reporting `1.0.0` and a `pmacs-gpu` reporting
`0.0.1`. **The workflow must refuse rather than publish** (acceptance
4), and acceptance asserts the *binaries'* output, not the manifests.

### 1.3a The bump exposed a defect: the daemon reported the wrong crate's version

**`InstanceIdentity::for_running_process` is defined in
`pmacs-protocol` and expanded `env!("CARGO_PKG_VERSION")` there.**
`env!` expands in the crate being *compiled*, so the field documented as
"Pmacs version string" carried the **protocol crate's** version.

This is not cosmetic. That identity reaches Lua as
`pmacs.instance.identity()` and goes on the wire in `Hello`, so every
attached frontend was told the daemon's version — and after the bump it
would have been told `1.0.0` by a `1.1.0` release.

**Nothing could have detected it before this stage.** Three tests assert
`id.pmacs_version == env!("CARGO_PKG_VERSION")` evaluated in the `pmacs`
crate, which is the correct assertion — but while both crates read
`1.0.0` they compared the same number reached by two different paths and
**could not fail**. Q#D1's decision to hold `pmacs-protocol` at 1.0.0
while moving `pmacs` is what made them discriminating, and all three
failed immediately on the bump.

The fix makes the version a **parameter**, so the `env!` expands in the
caller's crate; all three call sites are in `pmacs` and pass their own.
This is a breaking signature change to a `pub` function in
`pmacs-protocol`, which stays at 1.0.0 per Q#D1 — acceptable because the
crate is a path dependency with no external consumers, and recorded here
rather than silently absorbed.

*A test can be correct and still prove nothing, when the two things it
compares are equal for a reason unrelated to the code under test.*

### 1.4 The existing `v1.0.0` tag is stale and is not re-used

`v1.0.0` is pushed and points at `d3fa632` — the old release mirror's
head, **1,025 commits behind `main`**. There is also an `M8` milestone
tag. A `v*`-triggered workflow does not retroactively build for either,
and re-tagging would rewrite a published ref. The first real release is
**`v1.1.0`**, preceded by `v1.1.0-rc.1` (§7).

### 1.5 CRDT is not optional for a useful release

`default = ["luajit"]`; `luajit` and `lua54` are **mutually exclusive**;
`crdt` is opt-in on the root package, activated workspace-wide as
`pmacs/crdt`. README's documented build is the one to ship:

```sh
cargo build --release --workspace --features pmacs/crdt
```

**This is not a size/quality trade — a non-CRDT build refuses `--gpu`
outright.** `run_gpu` opens with a guard printing *"pmacs: --gpu
requires pmacs built with --features crdt"*. #209 established the same
thing on the wire side: `InstanceCapabilities::default` advertises
`multi_frontend` / `crdt_replica` / `semantic_render` only under the
feature. A non-CRDT release ships an editor that cannot use the GPU
frontend shipped beside it.

### 1.6 The glibc floor — decided, and revision 1's reasoning corrected

**Linux releases build on pinned `ubuntu-22.04`, with a stated support
floor of glibc ≥ 2.35.**

Revision 1 implied that moving off `ubuntu-latest` buys reach "several
distro generations" including RHEL 9. **It does not, and the arithmetic
matters:**

| target | glibc | covered by a 22.04 build? |
|---|---|---|
| Ubuntu 22.04 (jammy) | 2.35 | **yes** — this is the floor |
| Debian 12 (bookworm) | 2.36 | **yes** |
| Ubuntu 24.04 | 2.39 | yes |
| **RHEL 9** | **2.34** | **NO — below the floor** |

RHEL 9 is *older* than the floor, so it needs a lower-glibc container or
cross-build. **That is not a one-word runner change** and is parked
(§5). Stating the floor explicitly is what keeps this honest: a user on
RHEL 9 should read "not supported yet," not discover a loader error.

References: [runner images](https://github.com/actions/runner-images),
[jammy libc6](https://packages.ubuntu.com/jammy-updates/libc6),
[bookworm libc6](https://packages.debian.org/bookworm/libc6).

### 1.7 Runtime dependencies are documented but never checked

README names `/bin/sh`, `stty`, coreutils, git and tar, and tells
packagers to encode them. **Nothing checks any of them at runtime.**
Out of scope here (§5), but the release notes carry the list rather than
assume a downloader reads the README.

`pmacs-gpu` additionally needs a working Vulkan/Metal adapter; there is
no software-rasterizer fallback in a shipped binary.

### 1.8 Runner pinning, and what is NOT established

**Both runners are pinned**, not just Linux: `macos-latest` currently
resolves to macos-15 (ARM64) and **will drift** — the same silent-choice
problem as §1.6, in the other direction, where a future move could
change the minimum supported macOS without a commit to point at.

- **No release has ever been produced**, so nothing here is verified
  end-to-end. Unlike #209 there is no existing behaviour to measure
  against; the RC is the first evidence.
- **No Intel macOS** (§5). arm64 only.
- **No signing or notarization.** A downloaded macOS binary is
  Gatekeeper-quarantined and needs an explicit override. Expected for an
  unsigned baseline; it belongs in the release notes rather than in a
  claim that macOS "just works".

---

## 2. Decisions (all questions resolved at approval)

- **Q#D1 — versions.** `pmacs` and `pmacs-gpu` → 1.1.0;
  `pmacs-protocol` stays 1.0.0. Acceptance asserts both shipped binaries
  *report* 1.1.0 and the tag is `v1.1.0`.
- **Q#D2 — Linux runner.** Pinned `ubuntu-22.04`; support floor
  glibc ≥ 2.35, covering Ubuntu 22.04 and Debian 12 but **not RHEL 9**.
- **Q#D3 — `pmacs-audit`.** Does not ship.
- **Q#D4 — Intel macOS.** Not in the baseline. macOS runner pinned.
- **Q#D5 — checksums.** Ship them.
- **Q#D6 — tests on release.** Do not re-run the suite; assert the
  tagged commit is an ancestor of `main`, which catches the real mistake
  (tagging a branch) at negligible cost.

---

## 3. Bets

- **Bet 1 — the release build is the CI build with different flags** and
  will succeed first time.
- **Bet 2 — the artifact, not the build, is where this goes wrong.** The
  plausible failures are packaging-shaped: a stray binary, a split
  archive, a version mismatch, a glibc floor nobody notices.
- **Bet 3 — one RC is enough evidence.** A release either produces two
  runnable, correctly-versioned binaries at a URL or it does not.

---

## 4. Acceptance

1. Pushing a tag matching `v*` produces a **GitHub Release** with
   attached artifacts; no other trigger produces one. `v1.1.0-rc.1` is
   marked **prerelease**.
2. Each archive's **complete member list** is asserted, along with
   **executable bits**, and the **absence of `pmacs-audit`,
   `pmacs_fake_lsp` and `pmacs_fake_mcp`**. Verified by listing the
   downloaded archive, not by trusting the build command.
3. `pmacs` and `pmacs-gpu` are in the **same directory** within the
   archive, so an unpacked release is self-contained (§1.2a).
4. **Version coherence, asserted from the binaries:** the workflow
   refuses to publish when the tag disagrees with the root crate
   version, and the downloaded `pmacs --version` and `pmacs-gpu
   --version` both report **1.1.0**. Verified by a deliberate mismatch,
   not by inspection.
5. The tagged commit is an ancestor of `main`.
6. Each artifact is a **CRDT build**, verified by running the shipped
   binary rather than trusting the flag (§1.5).
7. The Linux binary honours the **glibc ≥ 2.35** floor, asserted from
   the binary's own versioned symbols on every release (§1.6a) rather
   than by a one-off container run, and the floor is stated in the
   release notes and README.
8. `SHA256SUMS` covers every published artifact and verifies against the
   downloads.
9. Release notes carry the runtime dependencies (§1.7), the glibc floor,
   the arm64-only macOS scope, and the unsigned/Gatekeeper caveat.
10. `README.md`'s install section offers the download path **before**
    from-source.

---

## 5. Parked (explicitly out of scope)

Named so they read as decisions. All are §17 requirements this stage
does not meet:

- **RHEL 9 and older glibc** — needs a container or cross-build (§1.6).
- **Intel macOS.**
- **Release channels**, stable/nightly.
- **Update-in-place and rollback.**
- **Signing and notarization.**
- **Reproducible builds.**
- **Package managers** — Homebrew, AUR, nixpkgs, distro packages.
- **Windows.** Unsupported anywhere in the tree today.
- **First-launch experience** — §17's config-directory creation and
  optional-tool detection. That is §18 onboarding work.
- **Protocol/package-API compatibility reporting.**

---

## 6. Gates

The standing `CLAUDE.md` suite applies unchanged; this stage adds no
Rust logic and no test, beyond the version bumps which the whole suite
covers. **Its real verification is the RC artifact** — acceptance 2, 3,
4, 6, 7 and 8 are all performed against a *downloaded* archive, because
that is the only thing that tests a release.

---

## 7. Branch plan

One branch, `distribution-stage1`:

1. **Bump `pmacs` and `pmacs-gpu` to 1.1.0** (Q#D1).
2. **Add `.github/workflows/release.yml`** — `on: push: tags: ['v*']`,
   pinned `ubuntu-22.04` + pinned macOS runner, explicit `--bin`
   targets, explicit staged asset list, the tag/version assertion, the
   ancestor check, and `SHA256SUMS`.
3. **Update `README.md`** so download precedes build-from-source, and
   state the glibc floor and macOS scope.
4. **Merge.**
5. **Tag `v1.1.0-rc.1` from the merge SHA**, marked prerelease. Verify
   acceptance 2–8 against the published artifacts.
6. **Tag `v1.1.0` from that same verified SHA.**

**A tag before the merge does nothing, silently.** For `on: push:
tags`, GitHub resolves the workflow file **as it exists at the tagged
commit** — and it lists a repository's workflows from the *default
branch*, so `release.yml` is not even registered until this PR merges
(verified: `gh workflow list` shows only CI while the file lives on the
branch). Tagging any commit that predates the merge therefore produces
no run, no error, and no release. **A silent no-op is the worst possible
outcome for a release step, because it is indistinguishable from "not
started yet."** Cut the RC from the merge SHA and confirm a run actually
appeared before concluding anything about it.

**Steps 5 and 6 are the point of the ordering.** A tag on a branch would
publish a release from unmerged code, so the RC necessarily follows the
merge — and it is cut *from the merge SHA*, so the final tag can reuse
the exact commit the RC verified. **If the RC finds a defect, it is
fixed in a new PR and another RC is cut; the final tag is never the
first live execution of this workflow.**
