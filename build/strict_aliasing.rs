// build/strict_aliasing.rs --- the verdict behind build.rs's refusal to
// compile C without -fno-strict-aliasing (E7h fix round 1).

//! `build.rs` and `pmacs-syntax/build.rs` ask the `cc` crate for the C
//! compiler it would run, the same question every grammar crate's build
//! script asks, and hand its arguments here (`aliasing_guard.rs`). `cc` puts the environment's flags last (`CFLAGS`, then
//! `HOST_CFLAGS` or `TARGET_CFLAGS`, then `CFLAGS_<target>`), so what
//! reaches a grammar's compile is decided by the last aliasing flag in the
//! list. `.cargo/config.toml` forces `-fno-strict-aliasing` into
//! `HOST_CFLAGS` and `TARGET_CFLAGS`, but cargo reads that file only in a
//! build started inside the checkout; E7h review 1 built pmacs with `cargo
//! install --git`, every grammar's record read `HOST_CFLAGS = None`, and
//! the editor aborted on the first keystroke into a two-pragma Haskell
//! file. `tests/e7h_grammar_gate_acceptance.rs` includes this file and
//! holds the verdict; `tests/e7h_review1_probes.rs` builds from outside
//! the checkout and requires the refusal.

/// `Ok` when the last aliasing flag among `args` is
/// `-fno-strict-aliasing`; otherwise the message the build fails with,
/// naming the flag, the command and the ways to supply it.
pub fn verdict(compiler: &str, args: &[String]) -> Result<(), String> {
    let last = args
        .iter()
        .rev()
        .find(|a| *a == "-fstrict-aliasing" || *a == "-fno-strict-aliasing");
    if last.is_some_and(|a| a == "-fno-strict-aliasing") {
        return Ok(());
    }
    let why = if last.is_some() {
        "a later -fstrict-aliasing overrides it (cc appends CFLAGS_<target> after HOST_CFLAGS)"
    } else {
        "nothing supplies it: .cargo/config.toml is read only by a build started inside the \
         pmacs checkout, and this one was not (cargo install --git, or --manifest-path from \
         elsewhere)"
    };
    Err(format!(
        "pmacs: refusing to build: the C compiler would not receive -fno-strict-aliasing; {why}.\n\
         \n\
         pmacs compiles tree-sitter's runtime and every bundled grammar's C into the editor \
         and its parse worker, pmacs-parse-unit, where a grammar's memory error runs a \
         crafted file's bytes with your authority. \
         GCC 16 at -O2 turned the strict-aliasing UB in a grammar's array header into a heap \
         overflow that aborted the editor (E7g); the grammars that carried that header ship \
         from vendored copies with the fixed one, and every C source is still compiled with \
         -fno-strict-aliasing so that no compiler may exploit what no one has yet found.\n\
         \n\
         Build from the repository root, where .cargo/config.toml supplies it, or supply it:\n\
         \n    CFLAGS=-fno-strict-aliasing cargo install --git <pmacs repository> pmacs pmacs-parse-unit\n\
         \n\
         The compiler cc would run: {compiler} {}",
        args.join(" ")
    ))
}
