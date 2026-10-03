// build.rs --- pmacs-syntax: refuse to compile the grammars' C without
// -fno-strict-aliasing.

//! This crate links tree-sitter's runtime and every bundled grammar, and
//! the editor and its parse worker `pmacs-parse-unit` both link it, so the
//! refusal the root `build.rs` makes for the editor (E7h fix round 1) is
//! made here too, for every binary that carries the grammar C (E7i review
//! 1, Medium 3: `cargo install` of the worker from outside the checkout
//! compiled every grammar with `HOST_CFLAGS = None` and was not refused).

#[path = "../build/strict_aliasing.rs"]
mod strict_aliasing;

#[path = "../build/aliasing_guard.rs"]
mod aliasing_guard;

fn main() {
    aliasing_guard::refuse_c_without_the_aliasing_flag();
}
