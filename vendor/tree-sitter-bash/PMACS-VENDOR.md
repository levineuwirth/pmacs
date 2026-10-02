# tree-sitter-bash 0.25.1, vendored by pmacs

The crate as published to crates.io (from tree-sitter/tree-sitter-bash at
`a06c2e4415e9bc0346c6b86d401879ffb44058f7`, recorded in `.cargo_vcs_info.json`),
with two changes; the parser is the published one.

- `src/tree_sitter/array.h` is tree-sitter-rust 0.24.2's, the header
  tree-sitter's CLI has generated since 0.24: its `array_push` assigns
  `contents` from the grow instead of writing it through an `(Array *)` cast.
- `src/scanner.c`'s brace-range scan compares `lookahead` against `'0'`..`'9'`
  where it called `isdigit` (marked `pmacs (E7h fix round 2)`; #302).

Why the header: the scanner pushes through the published header's `(Array *)`
cast, the aliasing UB GCC 16 turned into tree-sitter-haskell's abort. No compiler has yet
been seen to exploit it here, and `-fno-strict-aliasing` forbids every compiler
to, but only in builds cargo starts at the repository root (E7h review 1,
High 1). The owner ruled at E7h's fix round 1 that bash, haskell and html carry
the conforming header, so no shipped scanner depends on where a build starts
(haskell's copy was retired at fix round 2 for 0.24.1, published on it). Held
by `tests/e7g_review1_serialization_witnesses.rs` and
`tests/e7g_review1_probes.rs`.

Why the scan: `lookahead` is a codepoint, and `isdigit` is defined only for an
`unsigned char` or EOF. glibc indexes its table with the value, so `echo {`
before U+4A28A read unmapped memory and the release editor died with SIGSEGV
on opening the 12-byte file, three runs of three (#302). Held by
`e7h2_no_shipped_grammar_passes_a_codepoint_to_a_narrow_ctype_function` in
`tests/e7g_review1_probes.rs`.

Upstream, as of 2026-10-01: master `a06c2e4` is 0.25.1 itself. PR #343 (the
header, open since 2026-07-21) and PR #348 (`iswdigit` in `brace_start`, open
since 2026-09-20) fix the two; neither is merged or released.

To drop this copy: a crates.io release carrying both (its
`src/tree_sitter/array.h` passing no `(Array *)` to `_array__grow`, and no
narrow ctype call on `lookahead`), fuzzed clean with
`scripts/fuzz-grammars --grammar bash --seconds 600`, the `[patch.crates-io]`
entry and this directory removed in the same commit. A newer release missing
either is carried forward by replacing this directory with it and these
changes, never by dropping the patch.
