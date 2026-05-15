//! Shared test helpers across pmacs's integration tests.
//!
//! Submodules are imported into individual integration-test files
//! via `mod common;` followed by `use common::<submodule>::*;`.
//! Cargo treats files under `tests/common/` as non-test modules
//! (no main fn required) — the standard pattern for shared
//! integration-test code.
//!
//! Submodules:
//!
//! - [`pty`]: real-PTY pmacs spawner. First consumer M5.8 reconnect
//!   tests (`tests/m5_8_acceptance.rs`); second consumer M10.11
//!   doubled-PTY tests (`tests/m10_11_acceptance.rs`).
//! - [`daemon`]: `pmacs --daemon` subprocess fixture. First
//!   consumer M5.5 acceptance suite; second consumer M10.11
//!   doubled-PTY tests.

#![allow(dead_code)] // not every integration-test file uses every helper

pub mod daemon;
pub mod pty;
