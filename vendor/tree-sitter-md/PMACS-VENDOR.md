# tree-sitter-md 0.5.3, vendored by pmacs

The crate as published to crates.io (from tree-sitter-grammars/tree-sitter-markdown
at `f969cd3ae3f9fbd4e43205431d0ae286014c05b5`, recorded in `.cargo_vcs_info.json`),
with the upstream `LICENSE` from that commit added, since the published crate
carries none, and two changes, both in `tree-sitter-markdown/src/scanner.c`:

- `serialize` bounds its copy of the open blocks to tree-sitter's 1024-byte
  serialization buffer (marked `pmacs (E7h, D36 as amended)`);
- `parse_ordered_list_marker` compares `lookahead` against `'0'`..`'9'` where it
  called `isdigit` (marked `pmacs (E7h fix round 2)`; #302).

Why the bound: upstream copies five bytes and then four per open block with no bound, so a
file nesting 255 blockquotes or list items (`> ` repeated 255 times before a
character: 512 bytes on one line) writes past the runtime's buffer, and the
runtime's `assert(length <= 1024)` aborts the editor, and in daemon mode every
buffer the daemon holds. E7g review 1 found it; the owner ruled at E7h that a
local bound like this ships fixed in a vendored copy (D36 as amended) rather
than unshipped. Held by `tests/e7g_review1_serialization_witnesses.rs`.

Why the digits: `lookahead` is a codepoint, and `isdigit` is defined only for an
`unsigned char` or EOF. glibc indexes its table with the value, so `4` before
U+4A28A read unmapped memory and the release editor died with SIGSEGV on
opening the 6-byte file, two runs of three (#302). Held by
`e7h2_no_shipped_grammar_passes_a_codepoint_to_a_narrow_ctype_function` in
`tests/e7g_review1_probes.rs`.

**This copy has no retirement path.** Nothing upstream will retire it as
things stand, so it is carried until upstream changes that, not until a known
pull request lands. As of 2026-10-01:
- issue #243 (the overflow, 2026-04-28) is open, and no pull request fixes it:
  #259 (2026-09-13) was closed unmerged by its own author on 2026-10-01 "so it
  does not sit in the queue";
- the digits fix, #252, was closed unmerged on 2026-07-16 after the maintainer
  called it a tree-sitter matter (tree-sitter/tree-sitter#5255, the wasm
  stdlib), which does not reach a native build against glibc, and nothing has
  replaced it;
- the default branch, `split_parser` at `a0a00f8` (2026-07-19), carries
  neither fix, and the last release is 0.5.3 (2026-02-26).

A session reading this should not expect an upstream release to retire the
copy; it should check the trackers again rather than wait on a named pull
request.

To drop this copy all the same: a crates.io release with both fixed, fuzzed
clean with `scripts/fuzz-grammars --grammar markdown --grammar markdown_inline
--seconds 600`, the `[patch.crates-io]` entry and this directory removed in the
same commit.
