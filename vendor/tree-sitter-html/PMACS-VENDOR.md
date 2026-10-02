# tree-sitter-html 0.23.2, vendored by pmacs

The crate as published to crates.io (from tree-sitter/tree-sitter-html at
`5a5ca8551a179998360b4a4ca2c0f366a35acc03`, recorded in `.cargo_vcs_info.json`),
with the upstream `LICENSE` from that commit added, since the published crate
carries none, and one change: `src/tree_sitter/array.h` is tree-sitter-rust
0.24.2's, the header tree-sitter's CLI has generated since 0.24: its
`array_push` assigns `contents` from the grow instead of writing it through an
`(Array *)` cast. The scanner, `tag.h` and the parser are the published ones.

Why: the scanner pushes its tag stack through the published header's
`(Array *)` cast, the aliasing UB GCC 16 turned into tree-sitter-haskell's
abort. No compiler has yet been seen to exploit it here, and
`-fno-strict-aliasing` forbids every compiler to, but only in builds cargo
starts at the repository root (E7h review 1, High 1). The owner ruled at E7h's
fix round 1 that bash, haskell and html carry the conforming header, so no
shipped scanner depends on where a build starts (haskell's copy was retired at
fix round 2 for 0.24.1, published on it). Held by
`tests/e7g_review1_serialization_witnesses.rs` and
`tests/e7g_review1_probes.rs`.

Upstream, as of 2026-10-01: master `73a3947` still carries the casts, and no
issue or pull request about the header is open. The last release is 0.23.2
(2024-11-11).

To drop this copy: a crates.io release whose `src/tree_sitter/array.h` passes
no `(Array *)` to `_array__grow`, fuzzed clean with
`scripts/fuzz-grammars --grammar html --seconds 600`, the `[patch.crates-io]`
entry and this directory removed in the same commit. A newer release still on
the old header is carried forward by replacing this directory with it and this
header, never by dropping the patch.
