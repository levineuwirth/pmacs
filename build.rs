// build.rs --- Compile-time metadata.

//! Exposes `PMACS_GIT_HASH` as a build-time env var so that
//! `option_env!("PMACS_GIT_HASH")` resolves to a short git hash in
//! development builds and to `None` in source-tarball / release-build
//! cases where no git checkout is available.

use std::process::Command;

#[path = "build/strict_aliasing.rs"]
mod strict_aliasing;

fn main() {
    refuse_c_without_the_aliasing_flag();

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

/// E7h fix round 1: fail the build, before any grammar's C reaches the
/// editor, when the C compiler would not receive `-fno-strict-aliasing`
/// (`build/strict_aliasing.rs` says why). The question is put to `cc`
/// itself, which builds every grammar crate's C, so the answer carries
/// what it reads from `CC`, `CFLAGS`, `HOST_CFLAGS`/`TARGET_CFLAGS` and
/// `CFLAGS_<target>`, in its order; it also prints the
/// `rerun-if-env-changed` lines that re-run this check when any of them
/// changes. MSVC has no strict-aliasing optimization to refuse.
fn refuse_c_without_the_aliasing_flag() {
    let tool = match cc::Build::new().try_get_compiler() {
        Ok(tool) => tool,
        Err(e) => {
            // No C compiler at all: the grammar crates fail and say so.
            println!("cargo:warning=pmacs: cc found no C compiler to check: {e}");
            return;
        }
    };
    if tool.is_like_msvc() {
        return;
    }
    let args: Vec<String> = tool
        .args()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if let Err(message) = strict_aliasing::verdict(&tool.path().display().to_string(), &args) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
