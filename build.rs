// build.rs --- Compile-time metadata.

//! Exposes `PMACS_GIT_HASH` as a build-time env var so that
//! `option_env!("PMACS_GIT_HASH")` resolves to a short git hash in
//! development builds and to `None` in source-tarball / release-build
//! cases where no git checkout is available.

use std::process::Command;

fn main() {
    // Re-run the build script if HEAD or the index changes; this keeps
    // the embedded hash fresh during development without making every
    // `cargo build` re-run `git`.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-env-changed=PMACS_GIT_HASH");

    // Honor a caller-supplied PMACS_GIT_HASH (CI builds, reproducible
    // builds, source-tarball packaging). Otherwise best-effort `git`.
    if std::env::var_os("PMACS_GIT_HASH").is_some() {
        return;
    }
    let Ok(out) = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !hash.is_empty() {
        println!("cargo:rustc-env=PMACS_GIT_HASH={hash}");
    }
}
