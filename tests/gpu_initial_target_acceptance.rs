//! Named gate for the approved GPU initial-target framing.
//!
//! The target cases share the existing managed-lifecycle fixtures so every run
//! also proves the #141 reuse, spawn, isolation, and child-reaping invariants.

#![cfg(unix)]

#[path = "gpu_invocation_acceptance.rs"]
mod gpu_invocation_acceptance;
