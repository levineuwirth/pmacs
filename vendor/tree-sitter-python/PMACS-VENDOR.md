# tree-sitter-python 0.25.0, vendored by pmacs

The crate as published to crates.io (tree-sitter/tree-sitter-python), with two
changes:

- `src/scanner.c`'s `serialize` checks that both bytes of an indent fit before
  writing them (marked `pmacs (E7h, D36 as amended)`). Upstream checks
  `size < 1024` and then writes two, so with an odd number of open string
  delimiters and 511 indentation levels it writes 1025 bytes and the runtime's
  `assert(length <= 1024)` aborts the editor. E7g review 1 found it; the owner
  ruled at E7h that a local bound ships fixed in a vendored copy (D36 as
  amended). Held by `tests/e7g_review1_serialization_witnesses.rs`.
- `src/tree_sitter/array.h` is tree-sitter-rust 0.24.2's, the header tree-sitter's
  CLI has generated since 0.24: its `array_push` assigns `contents` from the grow
  instead of writing it through an `(Array *)` cast. The cast is undefined
  behavior (E7g's Haskell overflow). `.cargo/config.toml`'s
  `-fno-strict-aliasing` forbids every compiler to exploit it, but only in
  builds cargo starts at the repository root (E7h review 1, High 1), and the
  owner ruled at E7h.1 that a copy vendored anyway carries the fixed header,
  which takes clang's type sanitizer's reports on this scanner to none.

Upstream, as of 2026-10-01: issue #350 (the overrun, 2026-09-15) and PR #351
(the same one-line bound as this copy's, 2026-09-24) are open. No pull request
moves the header. Master `26855ea` carries neither fix.

To drop this copy: a crates.io release with both fixed, fuzzed clean with
`scripts/fuzz-grammars --grammar python --seconds 600`, the `[patch.crates-io]`
entry and this directory removed in the same commit.
