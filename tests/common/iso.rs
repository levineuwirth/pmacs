// tests/common/iso.rs --- isolated bootstrap roots for integration tests.

//! The integration suite's side of `pmacs::bootstrap`.
//!
//! An integration test links `pmacs` as an ordinary dependency, so it is
//! compiled **without** `cfg(test)` and the `#[cfg(not(test))]` guard in
//! `EditorState::new` is live for it: a raw `EditorState::new()` reads
//! the developer's real `~/.config/pmacs/init.lua` and writes bundled
//! packages into the developer's real `~/.local/share/pmacs`. It cannot
//! fix that by setting an environment variable --- `std::env::set_var`
//! is `unsafe` and `pmacs` is `#![forbid(unsafe_code)]`. So it passes
//! [`roots`] to `EditorState::new_with_roots` instead.
//!
//! See the archived test-ambient-config-isolation framing.
//!
//! Included per-file rather than through `mod common;` so a suite that
//! needs no daemon or PTY fixture does not compile them:
//!
//! ```ignore
//! #[path = "common/iso.rs"]
//! mod iso;
//! ```
//!
//! [`roots`] is a **pure function of the build environment** --- no
//! counter, no `OnceLock`. Two copies of this module in one test binary
//! (one via `mod common;`, one via `#[path]`) therefore return the same
//! value rather than racing over a shared counter.

#![allow(dead_code)] // not every including suite uses every helper

use std::path::PathBuf;

use pmacs::bootstrap::BootstrapRoots;

/// The shared isolated base, under Cargo's per-package integration-test
/// temp directory.
///
/// `CARGO_TARGET_TMPDIR` (`target/<profile>/tmp`) rather than `/tmp`:
/// nothing here is ever unlinked --- there is no libtest teardown hook
/// to unlink it from --- so the tree has to live somewhere `cargo clean`
/// owns instead of leaking into the system temp dir once per run.
///
/// Deliberately shared across tests and across test binaries.
/// `materialize_all` is content-gated and idempotent, so after the first
/// construction every later one is a no-op read; a per-test directory
/// would repeat the whole materialization ~330 times per run for no
/// isolation gain (the tree is byte-identical for every caller).
#[must_use]
pub fn base() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ambient-isolation")
}

/// Isolated bootstrap roots for an in-process editor.
///
/// Pass to `EditorState::new_with_roots` / `open_with_roots`.
#[must_use]
pub fn roots() -> BootstrapRoots {
    let base = base();
    let roots = BootstrapRoots::isolated_under(&base);
    // Create the config dir eagerly. `load_user_config_at` registers it
    // on Lua's `package.path` whether or not `init.lua` exists, and a
    // suite that later writes a config chunk into it should not have to
    // know the layout.
    if let Some(dir) = roots.config_dir() {
        std::fs::create_dir_all(&dir).expect("create isolated config dir");
    }
    roots
}
