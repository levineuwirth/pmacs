# tree-sitter-yaml 0.7.2, vendored by pmacs

The crate as published to crates.io (tree-sitter-grammars/tree-sitter-yaml),
with two changes:

- `src/scanner.c`'s `serialize` checks that a level's four bytes fit before
  writing them (marked `pmacs (E7h, D36 as amended)`). Upstream checks
  `size < 1024` and then writes four, so a file nested 254 levels deep writes
  1026 bytes into the runtime's 1024-byte buffer and the runtime's
  `assert(length <= 1024)` aborts the editor. E7g unshipped YAML for it; the
  owner ruled at E7h that a local bound ships fixed in a vendored copy (D36 as
  amended), and E7h.5 restored the grammar once its fuzz run cleared.
- `src/tree_sitter/array.h` is tree-sitter-rust 0.24.2's, the header
  tree-sitter's CLI has generated since 0.24 (see
  `vendor/tree-sitter-python/PMACS-VENDOR.md`).

Past 253 levels the deepest are not carried across a reparse, as upstream's
loop already intends; nothing aborts. `tests/e7g_grammar_fuzz_acceptance.rs`
opens 254 and 300 levels in the editor and requires a YAML tree, and
`tests/e7g_review1_serialization_witnesses.rs` fails by name if the crate
resolves to crates.io again.

Upstream, as of 2026-10-01: issue #48 (the overrun, 2026-08-11) is open. PR #47
(the bound, 2026-08-11) and PR #45 (the header, 2026-07-21) are open. Master
`a1c4812` carries neither fix.

To drop this copy: a crates.io release with both fixed (the bound and a
`src/tree_sitter/array.h` passing no `(Array *)` to `_array__grow`; a release
merging #47 without #45 meets only the first), fuzzed clean with
`scripts/fuzz-grammars --grammar yaml --seconds 600`, the `[patch.crates-io]`
entry and this directory removed in the same commit.
