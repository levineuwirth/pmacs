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
//! - [`ready`]: the one readiness wait, a predicate plus a deadline
//!   that reports elapsed time and the last observed state.
//! - [`reap`]: a spawned child in its own process group, signalled as a
//!   group on drop so no daemon outlives its test.
//! - [`iso`]: isolated bootstrap storage roots, so an in-process
//!   editor neither reads the developer's real `init.lua` nor writes
//!   into their real data root. Most suites include it directly via
//!   `#[path = "common/iso.rs"] mod iso;` rather than through this
//!   module; it is re-exported here for `daemon`'s use.

#![allow(dead_code)] // not every integration-test file uses every helper

pub mod daemon;
pub mod iso;
pub mod pty;
pub mod ready;
pub mod reap;
pub mod sigint_conformance;
