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
//! - `editor` (`common/editor.rs`): the in-process editor's Lua driver,
//!   `exec` and `eval`, the byte-identical bodies 67 suites once carried
//!   each (E8e). Included per file the way `iso` is and NOT declared
//!   here: ten suites pull both `mod common;` and the include, and
//!   `clippy::duplicate_mod` refuses a file loaded as a module twice.
//! - [`gpu_probe`]: the headless GPU probe (`pmacs-gpu
//!   --headless-probe`) as a row drives it: the binary beside this
//!   build's `pmacs`, and a finished probe's report as facts. Extracted
//!   at E10 (D37) from `tests/e9_session_acceptance.rs`.
//! - [`wire`]: a frontend session over a real daemon socket (the
//!   semantic handshake as `pmacs-gpu` performs it, or a grid attach),
//!   a reader thread, and the keys and waits a row drives it with.
//!   Extracted at E10 (D37) from the copy the E9 suites carry; `crdt`
//!   only, as a semantic session is.

#![allow(dead_code)] // not every integration-test file uses every helper

pub mod daemon;
pub mod gpu_probe;
pub mod iso;
pub mod pty;
pub mod ready;
pub mod reap;
pub mod sigint_conformance;
#[cfg(feature = "crdt")]
pub mod wire;
