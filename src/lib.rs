// lib.rs --- Pmacs library root. Declares the editor's module structure.

//! Pmacs core library.
//!
//! This crate is the in-process Rust core of the Pmacs editor. The binary in
//! `src/main.rs` wires the modules together; tests and benchmarks exercise
//! them directly.
//!
//! Module map (mirrors spec Chapter 3, "The Buffer Model"):
//! * [`rope`] --- persistent byte-addressed sequence
//! * [`buffer`] --- rope + identity + views + undo
//! * [`buffer_registry`] --- owned-Buffer storage behind opaque IDs (M2)
//! * [`command`] --- editor command registry (M2)
//! * [`view`] --- the View trait, polymorphic interpretive layer
//! * [`text_view`] --- the plain-text View implementation
//! * [`cell`] --- cell-grid rendering target
//! * [`frontend`] --- TUI backend (crossterm)
//! * [`file_io`] --- file load and atomic save
//! * [`editor_core`] --- mutable world state shared between Rust and Lua (M2.5)
//! * [`key`], [`keymap_tree`], [`keymap_stack`] --- chord parsing, trie, and dispatcher (M2.4)
//! * [`lua`] --- Lua VM owner (M2)
//! * [`lua_bindings`] --- hand-curated Lua surface (R51)
//! * [`worker`] --- work-stealing pool with cooperative cancellation (M3.1)
//! * [`message_bus`] --- typed in-process bus with `MessagePack` codec (M3.2)
//! * [`async_runtime`] --- main-thread dispatcher + tick over the bus (M3.3)

pub mod ansi;
pub mod async_runtime;
pub mod attach;
pub mod attach_dispatch;
pub mod attach_reconnect;
pub mod audit;
pub mod buffer;
pub mod buffer_registry;
pub mod builtin_packages;
pub mod cell;
pub mod code_action;
pub mod command;
pub mod completion;
pub mod completion_framework;
pub mod config;
// T M10.2: CRDT-backed buffer state. Feature-gated so v0.1 builds
// carry zero overhead — the `loro` dependency isn't pulled in, no
// field on the Buffer struct layout, no branch on `apply_edit`.
#[cfg(feature = "crdt")]
pub mod crdt;
// T M10.10: frontend-side CRDT replica for optimistic local edits.
// Gated on `crdt` because BufferMirror wraps `CrdtState`.
#[cfg(feature = "crdt")]
pub mod buffer_mirror;
pub mod daemon;
pub mod daemon_attach;
pub mod definition;
pub mod diag;
pub mod document_highlight;
pub mod editor;
pub mod editor_core;
pub mod file_io;
pub mod formatting;
pub mod frontend;
pub mod fs;
pub mod help;
pub mod highlight;
pub mod hook;
pub mod hover;
pub mod inlay_hint;
pub mod instance_buffer;
pub mod instance_render;
pub mod key;
pub mod keymap_stack;
pub mod keymap_tree;
pub mod locations;
pub mod lockfile;
pub mod lsp;
pub mod lsp_status;
pub mod lua;
pub mod lua_bindings;
pub mod lua_isolation;
pub mod mcp;
pub mod message_bus;
pub mod minibuffer;
// T M10.10: frontend-side optimistic-apply infrastructure (predicate
// + echo-dedup filter). Gated on `crdt` because it consumes
// BufferMirror.
#[cfg(feature = "crdt")]
pub mod optimistic;
pub mod overlay;
pub mod overlay_color;
pub mod overlay_paint;
pub mod packages;
pub mod prepare_rename;
pub mod presence;
pub mod process;
pub mod project;
pub mod project_index;
pub mod protocol;
pub mod rename;
pub mod rope;
// T M11.5 — the headless semantic consumer composes BufferMirror +
// optimistic (both `crdt`-gated) and is only meaningful on a
// `semantic_render` session, which the negotiation dependency rule
// ties to `crdt_replica`. Gated to match.
#[cfg(feature = "crdt")]
pub mod semantic_client;
pub mod semantic_render;
pub mod semantic_tokens;
pub mod signature;
pub mod socket_path;
pub mod symbol;
pub mod syntax;
pub mod text_view;
pub mod transport;
pub mod view;
pub mod window;
pub mod worker;
pub mod workers_buffer;
