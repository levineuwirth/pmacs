//! Re-export of `pmacs_protocol::transport`.
//!
//! Session 3 of the pmacs-gpu arc (`docs/pmacs-gpu-design.md`) moved
//! the length-prefix postcard codec into `pmacs-protocol` so
//! `pmacs-gpu` can use it without depending on the main `pmacs`
//! crate. Existing `crate::transport::...` import paths inside the
//! main `pmacs` crate continue to resolve through this re-export.
//!
//! The move surfaced an early Phase-A-style finding: the wire-types
//! crate's boundary as drawn in session 1 didn't include the
//! framing codec, but a real frontend needs both. Classified as
//! *small* under rule (iii) and absorbed here; structural lesson is
//! "transport is part of the wire contract."

pub use pmacs_protocol::transport::*;
