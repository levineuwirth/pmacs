// build.rs --- Compile-time metadata.

//! Exposes `PMACS_GIT_HASH` as a build-time env var so that
//! `option_env!("PMACS_GIT_HASH")` in this package resolves to a short git
//! hash in a release build from a git checkout, and to `None` in a debug or
//! test build and wherever no checkout is available, unless the caller
//! supplies the variable (`build/git_hash.rs`).

#[path = "build/strict_aliasing.rs"]
mod strict_aliasing;

#[path = "build/aliasing_guard.rs"]
mod aliasing_guard;

#[path = "build/git_hash.rs"]
mod git_hash;

fn main() {
    // E7h fix round 1 (`build/aliasing_guard.rs`); `pmacs-syntax`'s build
    // script makes the same refusal for the parse worker since E7i.
    aliasing_guard::refuse_c_without_the_aliasing_flag();

    // The hash, and the files git reports for this checkout that keep it
    // fresh; nothing to watch in a debug build (E10h, #371).
    git_hash::embed();
}
