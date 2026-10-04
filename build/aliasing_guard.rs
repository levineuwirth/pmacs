// build/aliasing_guard.rs --- the build scripts' refusal to compile C
// without -fno-strict-aliasing (E7h fix round 1; E7i fix round 1).

//! Included, beside `strict_aliasing.rs`, by the root `build.rs` (the
//! editor, `pmacs`) and by `pmacs-syntax/build.rs`. `pmacs-syntax` links
//! tree-sitter's runtime and every bundled grammar's C, and both the editor
//! and the parse worker `pmacs-parse-unit` link it, so the worker, where
//! the grammar C runs by default since E7i, passes through this refusal as
//! the editor does: E7i review 1 installed the worker from outside the
//! checkout with no flag and no refusal.

/// Fail the build, before any grammar's C reaches a binary, when the C
/// compiler would not receive `-fno-strict-aliasing` (`strict_aliasing.rs`
/// says why). The question is put to `cc` itself, which builds every
/// grammar crate's C, so the answer carries what it reads from `CC`,
/// `CFLAGS`, `HOST_CFLAGS`/`TARGET_CFLAGS` and `CFLAGS_<target>`, in its
/// order; it also prints the `rerun-if-env-changed` lines that re-run this
/// check when any of them changes. MSVC has no strict-aliasing optimization
/// to refuse.
pub fn refuse_c_without_the_aliasing_flag() {
    let tool = match cc::Build::new().try_get_compiler() {
        Ok(tool) => tool,
        Err(e) => {
            // No C compiler at all: the grammar crates fail and say so.
            println!("cargo:warning=pmacs: cc found no C compiler to check: {e}");
            return;
        }
    };
    if tool.is_like_msvc() {
        return;
    }
    let args: Vec<String> = tool
        .args()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if let Err(message) = super::strict_aliasing::verdict(&tool.path().display().to_string(), &args)
    {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
