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
pub mod buffer;
pub mod buffer_registry;
pub mod cell;
pub mod command;
pub mod completion;
pub mod completion_framework;
pub mod config;
pub mod daemon;
pub mod daemon_attach;
pub mod definition;
pub mod diag;
pub mod editor;
pub mod editor_core;
pub mod file_io;
pub mod formatting;
pub mod frontend;
pub mod help;
pub mod highlight;
pub mod hook;
pub mod hover;
pub mod instance_buffer;
pub mod instance_render;
pub mod key;
pub mod keymap_stack;
pub mod keymap_tree;
pub mod lockfile;
pub mod lsp;
pub mod lsp_status;
pub mod lua;
pub mod lua_bindings;
pub mod message_bus;
pub mod minibuffer;
pub mod overlay;
pub mod packages;
pub mod process;
pub mod project;
pub mod project_index;
pub mod protocol;
pub mod rope;
pub mod signature;
pub mod socket_path;
pub mod syntax;
pub mod text_view;
pub mod transport;
pub mod view;
pub mod window;
pub mod worker;
pub mod workers_buffer;
