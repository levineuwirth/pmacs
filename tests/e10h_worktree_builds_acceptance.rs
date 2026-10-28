// e10h_worktree_builds_acceptance.rs --- the root build script's hash and
// the files it watches, built by cargo in a worktree and a plain clone
// (E10h, #371).

//! In a `git worktree` `.git` is a file, and until E10h the root build
//! script declared `.git/HEAD` and `.git/index` relative to the package
//! root; cargo counts a declared path it cannot stat as changed, so every
//! cargo call in a worktree reran the script and rebuilt `pmacs` and every
//! binary that links it (#371). Since E10h.2 only a release build runs
//! `git`, and it declares the files git reports for the checkout, those
//! that exist; a supplied `PMACS_GIT_HASH` is honored in any profile and
//! watches nothing (`build/git_hash.rs`).
//!
//! These rows read cargo's own verdict rather than the script's output: a
//! fixture crate whose build script is `build/git_hash.rs` itself, included
//! by path as the root `build.rs` includes it, is committed to a scratch
//! repository and built in a worktree of it and in a plain clone of it. Its
//! binary prints the hash it was compiled with. `cargo build -v` names a
//! unit it did not rebuild `Fresh` and one it did `Dirty`, with the reason.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The fixture's package name, as cargo prints it in `Fresh`, `Dirty` and
/// `Compiling` lines.
const FIXTURE: &str = "e10h-fixture";

/// A scratch repository holding the fixture crate, committed once.
struct Lab {
    dir: tempfile::TempDir,
}

/// One checkout of the lab's repository, with a target directory of its own.
struct Checkout {
    root: PathBuf,
    target: PathBuf,
}

/// `git` with no user or system configuration (the machine's may sign
/// commits or name a hook), an identity of its own, and none of the
/// variables that would point it at another repository.
fn git_command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "e10h")
        .env("GIT_AUTHOR_EMAIL", "e10h@example.invalid")
        .env("GIT_COMMITTER_NAME", "e10h")
        .env("GIT_COMMITTER_EMAIL", "e10h@example.invalid");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_CEILING_DIRECTORIES",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_command(dir).args(args).output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

impl Lab {
    /// The fixture crate, committed on `main` in `<lab>/origin`.
    fn new() -> Self {
        Self::with_repository(&[], &[])
    }

    /// The same, in a repository `git init` makes with `init` added to
    /// its arguments (`--ref-format=reftable`), and `config` set in it
    /// before the fixture's commit (`core.logAllRefUpdates=false`).
    fn with_repository(init: &[&str], config: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let origin = dir.path().join("origin");
        std::fs::create_dir_all(origin.join("src")).expect("mkdir");
        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("build/git_hash.rs");
        let module = module.to_str().expect("a UTF-8 checkout path");
        std::fs::write(
            origin.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{FIXTURE}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
                 build = \"build.rs\"\n\n[workspace]\n"
            ),
        )
        .expect("write Cargo.toml");
        std::fs::write(
            origin.join("build.rs"),
            format!(
                "#[path = {module:?}]\nmod git_hash;\n\nfn main() {{\n    git_hash::embed();\n}}\n"
            ),
        )
        .expect("write build.rs");
        std::fs::write(
            origin.join("src/main.rs"),
            "fn main() {\n    print!(\"{}\", option_env!(\"PMACS_GIT_HASH\").unwrap_or(\"none\"));\n}\n",
        )
        .expect("write main.rs");
        let mut args = vec!["init", "-q", "-b", "main"];
        args.extend_from_slice(init);
        git(&origin, &args);
        for (key, value) in config {
            git(&origin, &["config", key, value]);
        }
        git(&origin, &["add", "Cargo.toml", "build.rs", "src/main.rs"]);
        git(&origin, &["commit", "-q", "-m", "fixture"]);
        Self { dir }
    }

    /// A `git worktree` of the lab's repository on a branch of its own:
    /// `.git` is a file there.
    fn worktree(&self) -> Checkout {
        let root = self.dir.path().join("wt");
        git(
            &self.dir.path().join("origin"),
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt",
                root.to_str().expect("UTF-8"),
            ],
        );
        assert!(root.join(".git").is_file(), "a worktree's .git is a file");
        self.checkout(root, "target-wt")
    }

    /// A plain `git clone` of the lab's repository: `.git` is a directory.
    fn plain_clone(&self) -> Checkout {
        self.plain_clone_with(&[])
    }

    /// The same, with `extra` added to `git clone`'s arguments.
    fn plain_clone_with(&self, extra: &[&str]) -> Checkout {
        let root = self.dir.path().join("clone");
        let mut args = vec!["clone", "-q"];
        args.extend_from_slice(extra);
        args.extend(["origin", root.to_str().expect("UTF-8")]);
        git(self.dir.path(), &args);
        assert!(root.join(".git").is_dir(), "a clone's .git is a directory");
        self.checkout(root, "target-clone")
    }

    fn checkout(&self, root: PathBuf, target: &str) -> Checkout {
        Checkout {
            root,
            target: self.dir.path().join(target),
        }
    }
}

/// What one `cargo build -v` of a checkout said, and what its binary prints.
struct Built {
    stderr: String,
    hash: String,
}

impl Built {
    /// Cargo rebuilt nothing: the fixture is `Fresh`, and no unit is
    /// `Dirty` or compiled.
    fn compiled_nothing(&self) -> bool {
        self.stderr.contains(&format!("Fresh {FIXTURE}"))
            && !self.stderr.contains("Dirty")
            && !self.stderr.contains(&format!("Compiling {FIXTURE}"))
    }
}

impl Checkout {
    /// `cargo build -v`, in `release` or `dev`, with `supplied` as
    /// `PMACS_GIT_HASH` when given; then the binary's output.
    fn build(&self, release: bool, supplied: Option<&str>) -> Built {
        let mut cmd = Command::new(env!("CARGO"));
        cmd.current_dir(&self.root)
            .args(["build", "-v", "--offline", "--target-dir"])
            .arg(&self.target)
            // the machine's sccache plays no part in what cargo decides
            .env("RUSTC_WRAPPER", "")
            .env("CARGO_TERM_COLOR", "never")
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_TARGET_DIR")
            .env_remove("PMACS_GIT_HASH");
        if release {
            cmd.arg("--release");
        }
        if let Some(hash) = supplied {
            cmd.env("PMACS_GIT_HASH", hash);
        }
        // The build script runs `git`; the same isolation as the rows'.
        let isolated = git_command(&self.root);
        for (key, value) in isolated.get_envs() {
            match value {
                Some(v) => cmd.env(key, v),
                None => cmd.env_remove(key),
            };
        }
        let out: Output = cmd.output().expect("cargo runs");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "cargo build in {}:\n{stderr}",
            self.root.display()
        );
        let profile = if release { "release" } else { "debug" };
        let bin = self.target.join(profile).join(FIXTURE);
        let ran = Command::new(&bin)
            .output()
            .expect("the fixture binary runs");
        Built {
            stderr,
            hash: String::from_utf8_lossy(&ran.stdout).into_owned(),
        }
    }

    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "--short", "HEAD"])
    }

    /// A commit that changes no file, so only the ref (or a detached HEAD)
    /// and the reflogs move: the index need not be written.
    fn commit(&self, message: &str) {
        git(
            &self.root,
            &["commit", "-q", "--allow-empty", "-m", message],
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum Layout {
    Worktree,
    Clone,
}

impl Lab {
    fn checkout_in(&self, layout: Layout) -> Checkout {
        match layout {
            Layout::Worktree => self.worktree(),
            Layout::Clone => self.plain_clone(),
        }
    }
}

/// #371's own case: a debug build runs no `git` and watches no file, so a
/// second build compiles nothing and a commit made after it rebuilds
/// nothing and embeds nothing. At the base a worktree's second build is
/// `Dirty` on `.git/HEAD`.
fn debug_compiles_nothing_twice_and_a_commit_rebuilds_nothing(layout: Layout) {
    let lab = Lab::new();
    let checkout = lab.checkout_in(layout);
    checkout.build(false, None);
    let second = checkout.build(false, None);
    assert!(
        second.compiled_nothing(),
        "{layout:?}: a second debug build compiles nothing:\n{}",
        second.stderr
    );
    assert_eq!(
        second.hash, "none",
        "{layout:?}: a debug build embeds no hash"
    );
    checkout.commit("after the build");
    let third = checkout.build(false, None);
    assert!(
        third.compiled_nothing(),
        "{layout:?}: a commit rebuilds no debug build:\n{}",
        third.stderr
    );
    assert_eq!(third.hash, "none", "{layout:?}");
}

#[test]
fn e10h_a_debug_build_in_a_worktree_compiles_nothing_twice_and_a_commit_rebuilds_nothing() {
    debug_compiles_nothing_twice_and_a_commit_rebuilds_nothing(Layout::Worktree);
}

#[test]
fn e10h_a_debug_build_in_a_clone_compiles_nothing_twice_and_a_commit_rebuilds_nothing() {
    debug_compiles_nothing_twice_and_a_commit_rebuilds_nothing(Layout::Clone);
}

/// A release build embeds the hash HEAD names, a second release build
/// compiles nothing, and a commit made after a build changes the hash the
/// next build embeds. The commit changes no file, so it moves only the
/// branch's ref and the reflogs: the index need not be written, and this
/// fails unless the ref is watched.
fn release_follows_a_commit_and_compiles_nothing_twice(layout: Layout) {
    let lab = Lab::new();
    let checkout = lab.checkout_in(layout);
    checkout.build(true, None);
    let second = checkout.build(true, None);
    assert!(
        second.compiled_nothing(),
        "{layout:?}: a second release build compiles nothing:\n{}",
        second.stderr
    );
    let before = checkout.head();
    assert_eq!(second.hash, before, "{layout:?}: the hash HEAD names");
    checkout.commit("after the build");
    let after = checkout.head();
    assert_ne!(before, after);
    let third = checkout.build(true, None);
    assert_eq!(
        third.hash, after,
        "{layout:?}: a commit after a build moves the hash the next build embeds:\n{}",
        third.stderr
    );
}

#[test]
fn e10h_a_release_build_in_a_worktree_follows_a_commit_and_compiles_nothing_twice() {
    release_follows_a_commit_and_compiles_nothing_twice(Layout::Worktree);
}

#[test]
fn e10h_a_release_build_in_a_clone_follows_a_commit_and_compiles_nothing_twice() {
    release_follows_a_commit_and_compiles_nothing_twice(Layout::Clone);
}

/// The two places a commit lands that are not the branch's loose ref: a
/// packed ref, which has no file of its own until the commit writes one,
/// and a detached HEAD, where the commit rewrites HEAD itself. Each still
/// moves the hash, and the build between compiles nothing, so every path
/// declared for them exists.
fn packed_ref_and_detached_head_move_the_hash(layout: Layout) {
    let lab = Lab::new();
    let checkout = lab.checkout_in(layout);
    git(&checkout.root, &["pack-refs", "--all"]);
    let branch = git(&checkout.root, &["symbolic-ref", "HEAD"]);
    let loose = PathBuf::from(git(&checkout.root, &["rev-parse", "--git-path", &branch]));
    assert!(
        !checkout.root.join(loose).exists(),
        "{layout:?}: the branch's ref is packed"
    );
    checkout.build(true, None);
    let again = checkout.build(true, None);
    assert!(
        again.compiled_nothing(),
        "{layout:?}, packed: a second release build compiles nothing:\n{}",
        again.stderr
    );
    assert_eq!(again.hash, checkout.head(), "{layout:?}, packed");
    checkout.commit("on a packed ref");
    let moved = checkout.build(true, None);
    assert_eq!(
        moved.hash,
        checkout.head(),
        "{layout:?}: a commit on a packed ref moves the hash:\n{}",
        moved.stderr
    );

    git(&checkout.root, &["checkout", "-q", "--detach"]);
    checkout.build(true, None);
    let again = checkout.build(true, None);
    assert!(
        again.compiled_nothing(),
        "{layout:?}, detached: a second release build compiles nothing:\n{}",
        again.stderr
    );
    assert_eq!(again.hash, checkout.head(), "{layout:?}, detached");
    checkout.commit("on a detached HEAD");
    let moved = checkout.build(true, None);
    assert_eq!(
        moved.hash,
        checkout.head(),
        "{layout:?}: a commit on a detached HEAD moves the hash:\n{}",
        moved.stderr
    );
}

#[test]
fn e10h_in_a_worktree_a_packed_ref_and_a_detached_head_still_move_the_hash() {
    packed_ref_and_detached_head_move_the_hash(Layout::Worktree);
}

#[test]
fn e10h_in_a_clone_a_packed_ref_and_a_detached_head_still_move_the_hash() {
    packed_ref_and_detached_head_move_the_hash(Layout::Clone);
}

/// A supplied `PMACS_GIT_HASH` is embedded in either profile and watches
/// no file, so in a worktree a second build compiles nothing and a commit
/// rebuilds nothing. At the base both `.git` lines printed before the
/// supplied hash's early return, so the worktree's second build was
/// `Dirty` on `.git/HEAD`.
#[test]
fn e10h_a_supplied_hash_is_embedded_in_either_profile_and_watches_nothing() {
    let lab = Lab::new();
    let checkout = lab.worktree();
    for release in [false, true] {
        checkout.build(release, Some("supplied"));
        let second = checkout.build(release, Some("supplied"));
        assert!(
            second.compiled_nothing(),
            "a second build with a supplied hash compiles nothing (release: {release}):\n{}",
            second.stderr
        );
        assert_eq!(second.hash, "supplied", "release: {release}");
    }
    checkout.commit("after the builds");
    for release in [false, true] {
        let after = checkout.build(release, Some("supplied"));
        assert!(
            after.compiled_nothing(),
            "a commit rebuilds nothing under a supplied hash (release: {release}):\n{}",
            after.stderr
        );
        assert_eq!(after.hash, "supplied", "release: {release}");
    }
}

// ---------------------------------------------------------------------------
// E10h review 1's probes (2026-10-28), committed by fix round 1 as
// witnesses, each failing where its finding said.
// ---------------------------------------------------------------------------

/// P1 (review 1, Low 1): the root build script itself, as cargo ran it for
/// the build this row was compiled in, declares only files that exist, and
/// in a debug build declares none and embeds no hash. The seven rows above
/// build a fixture whose script is `build/git_hash.rs`; nothing else reads
/// the root `build.rs`, so a `.git/HEAD` line restored there passes them.
/// Cargo keeps a script's stdout as `output` beside its `OUT_DIR`, a layout
/// it does not document; a cargo that moved it fails this row on the read,
/// never passes it. At 57cd5ac, in a worktree (the gate's layout) the
/// script declares `.git/HEAD`, which cannot exist (#371); in a clone
/// (CI's) it exists, and the debug assertion fails instead.
#[test]
fn e10h_review1_p1_the_root_build_script_declares_nothing_missing_and_nothing_in_a_debug_build() {
    let out_dir = Path::new(env!("OUT_DIR"));
    let output = out_dir
        .parent()
        .expect("OUT_DIR has a parent")
        .join("output");
    let text = std::fs::read_to_string(&output)
        .unwrap_or_else(|e| panic!("the root build script's output {}: {e}", output.display()));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let declared: Vec<PathBuf> = text
        .lines()
        .filter_map(|line| {
            line.strip_prefix("cargo:rerun-if-changed=")
                .or_else(|| line.strip_prefix("cargo::rerun-if-changed="))
        })
        .map(|path| root.join(path))
        .collect();
    let missing: Vec<&PathBuf> = declared.iter().filter(|path| !path.exists()).collect();
    assert!(
        missing.is_empty(),
        "the root build script declares files that do not exist, so cargo reruns it \
         and rebuilds pmacs on every invocation (#371): {missing:?}\n{text}"
    );
    // <target>/<profile>/build/<package>-<hash>/out
    let profile = out_dir
        .ancestors()
        .nth(3)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    if profile == Some("debug") {
        assert!(
            declared.is_empty(),
            "a debug build watches no file (E10h.2 as amended): {declared:?}\n{text}"
        );
        assert!(
            !text.contains("rustc-env=PMACS_GIT_HASH"),
            "a debug build embeds no hash (E10h.2 as amended):\n{text}"
        );
    }
}

/// A commit that rewrites HEAD's commit message alone: like an empty
/// commit it changes no file, so the index need not be written. The commit
/// it rewrites is the empty one P2 made, which git amends only when told
/// it may stay empty.
fn reword(checkout: &Checkout) {
    git(
        &checkout.root,
        &["commit", "-q", "--amend", "--allow-empty", "-m", "reworded"],
    );
}

/// P2 (review 1, Low 2): in a repository with the reftable ref backend
/// (`git init --ref-format=reftable`, git 2.45 and later; the default
/// planned for git 3.0) there is no loose ref, no `packed-refs` and no
/// `logs/HEAD`; `.git/HEAD` is a stub no commit rewrites. At 8ef24c6 only
/// it and the index were declared, so a release build's hash stayed at the
/// commit it was built from after an empty commit or a reworded one; the
/// stacks' `tables.list` now stand in. A git older than 2.45 fails this row
/// at its first `git init`, with git's own message (2.44.4's: `fatal:
/// unknown ref storage format 'reftable'`); every CI runner's git read at
/// fix round 1 was 2.55 or 2.56.
fn reftable_follows_an_empty_and_a_reworded_commit(layout: Layout) {
    let lab = Lab::with_repository(&["--ref-format=reftable"], &[]);
    let checkout = match layout {
        Layout::Worktree => lab.worktree(),
        Layout::Clone => lab.plain_clone_with(&["--ref-format=reftable"]),
    };
    assert_eq!(
        git(&checkout.root, &["rev-parse", "--show-ref-format"]),
        "reftable",
        "{layout:?}: the checkout's refs are a reftable stack"
    );
    checkout.build(true, None);
    let again = checkout.build(true, None);
    assert!(
        again.compiled_nothing(),
        "{layout:?}, reftable: a second release build compiles nothing:\n{}",
        again.stderr
    );
    assert_eq!(again.hash, checkout.head(), "{layout:?}, reftable");
    checkout.commit("on a reftable stack");
    let moved = checkout.build(true, None);
    assert_eq!(
        moved.hash,
        checkout.head(),
        "{layout:?}, reftable: an empty commit moves the hash:\n{}",
        moved.stderr
    );
    reword(&checkout);
    let reworded = checkout.build(true, None);
    assert_eq!(
        reworded.hash,
        checkout.head(),
        "{layout:?}, reftable: a reworded commit moves the hash:\n{}",
        reworded.stderr
    );
}

#[test]
fn e10h_review1_p2_in_a_reftable_worktree_a_commit_moves_the_hash() {
    reftable_follows_an_empty_and_a_reworded_commit(Layout::Worktree);
}

#[test]
fn e10h_review1_p2_in_a_reftable_clone_a_commit_moves_the_hash() {
    reftable_follows_an_empty_and_a_reworded_commit(Layout::Clone);
}

/// P3 (review 1, Low 2): with `core.logAllRefUpdates` off there is no
/// `logs/HEAD`, so at 8ef24c6, while the branch's ref was packed, only
/// HEAD, the index and `packed-refs` were declared; the commit wrote a
/// loose ref nothing declared named, and the hash stayed stale until
/// something wrote the index. The nearest directory under `refs` above the
/// loose file now stands in.
fn packed_ref_without_reflogs_follows_a_commit(layout: Layout) {
    let lab = Lab::with_repository(&[], &[("core.logAllRefUpdates", "false")]);
    let checkout = match layout {
        Layout::Worktree => lab.worktree(),
        Layout::Clone => lab.plain_clone_with(&["-c", "core.logAllRefUpdates=false"]),
    };
    git(&checkout.root, &["pack-refs", "--all"]);
    let reflog = PathBuf::from(git(
        &checkout.root,
        &["rev-parse", "--git-path", "logs/HEAD"],
    ));
    assert!(
        !checkout.root.join(reflog).exists(),
        "{layout:?}: no reflog is kept"
    );
    checkout.build(true, None);
    let again = checkout.build(true, None);
    assert!(
        again.compiled_nothing(),
        "{layout:?}, no reflog: a second release build compiles nothing:\n{}",
        again.stderr
    );
    assert_eq!(again.hash, checkout.head(), "{layout:?}, no reflog");
    checkout.commit("on a packed ref, no reflog");
    let moved = checkout.build(true, None);
    assert_eq!(
        moved.hash,
        checkout.head(),
        "{layout:?}, no reflog: a commit on a packed ref moves the hash:\n{}",
        moved.stderr
    );
}

#[test]
fn e10h_review1_p3_in_a_worktree_a_packed_ref_without_reflogs_moves_the_hash() {
    packed_ref_without_reflogs_follows_a_commit(Layout::Worktree);
}

#[test]
fn e10h_review1_p3_in_a_clone_a_packed_ref_without_reflogs_moves_the_hash() {
    packed_ref_without_reflogs_follows_a_commit(Layout::Clone);
}

/// P4 (review 1, Low 3): every identity this process gives for itself
/// (the daemon's `Hello`, `pmacs.instance.*`, `*pmacs-instance*`) is built
/// by `protocol::running_identity`, the one place `pmacs` reads the hash
/// its build script embeds; a production call of
/// `InstanceIdentity::for_running_process` anywhere else carries the hash
/// `pmacs-protocol` was compiled with, which a release build's script
/// never reaches. Read as `tests/daemon_write_consistency.rs` reads: a
/// line holding the needle that is not a comment, before its file's
/// `mod tests {`. At 57cd5ac it fails on `src/daemon.rs` and
/// `src/lua_bindings/mod.rs`.
#[test]
fn e10h_review1_p4_the_running_identity_is_built_in_one_place() {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read src") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    files.sort();
    let mut sites = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read");
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code == "mod tests {" {
                break;
            }
            if code.starts_with("//") || !code.contains("for_running_process(") {
                continue;
            }
            let name = file.strip_prefix(root).expect("under the root");
            sites.push(format!("{}:{}", name.display(), number + 1));
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "InstanceIdentity::for_running_process is called outside \
         protocol::running_identity: {sites:?}"
    );
    assert!(
        sites[0].starts_with("src/protocol.rs:"),
        "the one call is running_identity's: {sites:?}"
    );
}
