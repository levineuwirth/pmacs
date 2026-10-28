// build/git_hash.rs --- the git hash the root build script embeds, and the
// files that keep it fresh (E10h, #371).

//! Included by the root `build.rs`, and by the fixture crate that
//! `tests/e10h_worktree_builds_acceptance.rs` builds in a worktree and in a
//! plain clone, so those rows run this file and not a copy of it.
//!
//! A release build embeds `git rev-parse --short HEAD` as `PMACS_GIT_HASH`
//! and names, with `cargo:rerun-if-changed`, the files a commit or a
//! checkout writes, so the hash follows HEAD without running `git` on every
//! build. Until E10h every build did, naming `.git/HEAD` and `.git/index`
//! relative to the package root. In a `git worktree`, where `.git` is a
//! file holding a `gitdir:` pointer, neither can exist, and cargo counts a
//! declared path it cannot stat as changed: every cargo invocation in a
//! worktree reran the script and rebuilt `pmacs` and everything that links
//! it. The paths are now the ones git reports for the checkout, in either
//! layout, and only those that exist are declared.
//!
//! A debug build (cargo's `PROFILE` is `debug`: every gate's, every CI
//! `Test` leg's) runs no `git` and embeds no hash, so it never rebuilds
//! because a commit was made (E10h.2 as amended, option D of four: the hash
//! in every build, in release builds only, only when supplied, or removed).
//! A `cargo test --release` is a release build and runs it, as CI's M1
//! Acceptance and M4, M5, M6 and M10 Perf Gates do.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Print the build script's directives. A caller-supplied `PMACS_GIT_HASH`
/// (a reproducible build, a packager) is honored in any profile: the crate
/// reads the caller's variable itself, so no `git` runs and no file is
/// watched, and `rerun-if-env-changed` reruns this script when it changes.
/// Otherwise a release build embeds the hash as `cargo:rustc-env` and
/// watches what moves it; any other profile does neither. Outside a git
/// checkout nothing is watched and nothing embedded.
pub fn embed() {
    println!("cargo:rerun-if-env-changed=PMACS_GIT_HASH");
    if std::env::var_os("PMACS_GIT_HASH").is_some() {
        return;
    }
    // Cargo's `PROFILE` is `release` for the release profile and every
    // profile that inherits it (`fuzz`), `debug` for the rest.
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }
    let root =
        std::env::var_os("CARGO_MANIFEST_DIR").map_or_else(|| PathBuf::from("."), PathBuf::from);
    for path in watched(&root) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if let Some(hash) = git(&root, &["rev-parse", "--short", "HEAD"]) {
        println!("cargo:rustc-env=PMACS_GIT_HASH={hash}");
    }
}

/// The files a move of HEAD writes, as git reports them for the checkout at
/// `root`, each only if it exists: HEAD itself (a checkout, a commit on a
/// detached HEAD), the index, and the ref HEAD names, which a commit on the
/// branch rewrites. A packed ref has no file of its own until the next
/// commit writes one, which nothing declared would see, so while the ref is
/// packed `packed-refs` stands in for it, with HEAD's reflog, which every
/// commit appends to, or where no reflog is kept (`core.logAllRefUpdates`
/// off) the nearest directory under `refs` above the loose file, which the
/// commit's new file changes. A reftable repository (git 2.45 and later)
/// has no loose ref, no `packed-refs` and no reflog file, and its HEAD is a
/// stub no commit rewrites: every ref update rewrites the `tables.list` of
/// the stack it lands in, the checkout's own (a worktree's HEAD) and the
/// common one (its branches), so both stand in (E10h review 1, Low 2).
pub fn watched(root: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = ["HEAD", "index"]
        .into_iter()
        .filter_map(|name| git_path(root, name))
        .collect();
    if git(root, &["rev-parse", "--show-ref-format"]).as_deref() == Some("reftable") {
        paths.extend(git_path(root, "reftable/tables.list"));
        paths.extend(
            rev_parse_path(root, &["--git-common-dir"])
                .map(|common| common.join("reftable/tables.list")),
        );
    } else if let Some(branch) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        match git_path(root, &branch) {
            Some(loose) if loose.exists() => paths.push(loose),
            loose => {
                paths.extend(git_path(root, "packed-refs"));
                match git_path(root, "logs/HEAD").filter(|reflog| reflog.exists()) {
                    Some(reflog) => paths.push(reflog),
                    None => paths.extend(loose.and_then(|loose| nearest_refs_dir(root, &loose))),
                }
            }
        }
    }
    paths.retain(|path| path.exists());
    paths.dedup();
    paths
}

/// The nearest directory above `loose` that exists and lies under the
/// repository's `refs`: where a commit on a packed branch creates the
/// branch's loose file.
fn nearest_refs_dir(root: &Path, loose: &Path) -> Option<PathBuf> {
    let refs = git_path(root, "refs")?;
    loose
        .ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(&refs))
        .find(|dir| dir.is_dir())
        .map(Path::to_path_buf)
}

/// `git rev-parse --git-path <name>`: where `name` lives for this checkout
/// (a worktree's own HEAD, index and reftable stack under the main
/// repository's `worktrees/`, its refs in the common directory).
fn git_path(root: &Path, name: &str) -> Option<PathBuf> {
    rev_parse_path(root, &["--git-path", name])
}

/// `git rev-parse <args>` answering a path, joined to `root` when git
/// answers relative to it.
fn rev_parse_path(root: &Path, args: &[&str]) -> Option<PathBuf> {
    let mut argv = vec!["rev-parse"];
    argv.extend_from_slice(args);
    let path = PathBuf::from(git(root, &argv)?);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

/// `git <args>` run at `root`, its trimmed output when it succeeds and says
/// something.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}
