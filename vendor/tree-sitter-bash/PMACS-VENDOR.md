# tree-sitter-bash 0.25.1, vendored by pmacs

The crate as published to crates.io (from tree-sitter/tree-sitter-bash at
`a06c2e4415e9bc0346c6b86d401879ffb44058f7`, recorded in `.cargo_vcs_info.json`),
with one change: `src/tree_sitter/array.h` is tree-sitter-rust 0.24.2's, the
header tree-sitter's CLI has generated since 0.24 (see
`vendor/tree-sitter-haskell/PMACS-VENDOR.md`). The scanner and the parser are
the published ones.

Why: the scanner pushes through the published header's `(Array *)` cast, the
aliasing UB GCC 16 turned into tree-sitter-haskell's abort. No compiler has yet
been seen to exploit it here, and `-fno-strict-aliasing` forbids every compiler
to, but only in builds cargo starts at the repository root (E7h review 1,
High 1). The owner ruled at E7h's fix round 1 that bash, haskell and html carry
the conforming header, so no shipped scanner depends on where a build starts.
Held by `tests/e7g_review1_serialization_witnesses.rs` and
`tests/e7g_review1_probes.rs`.

To drop this copy: a crates.io release whose `src/tree_sitter/array.h` passes
no `(Array *)` to `_array__grow`, fuzzed clean with
`scripts/fuzz-grammars --grammar bash --seconds 600`, the `[patch.crates-io]`
entry and this directory removed in the same commit. A newer release still on
the old header is carried forward by replacing this directory with it and this
header, never by dropping the patch.
