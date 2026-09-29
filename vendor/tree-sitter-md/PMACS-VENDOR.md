# tree-sitter-md 0.5.3, vendored by pmacs

The crate as published to crates.io (from tree-sitter-grammars/tree-sitter-markdown
at `f969cd3ae3f9fbd4e43205431d0ae286014c05b5`, recorded in `.cargo_vcs_info.json`),
with the upstream `LICENSE` from that commit added, since the published crate
carries none, and one change: `tree-sitter-markdown/src/scanner.c`'s `serialize`
bounds its copy of the open blocks to tree-sitter's 1024-byte serialization
buffer (marked `pmacs (E7h, D36 as amended)`).

Why: upstream copies five bytes and then four per open block with no bound, so a
file nesting 255 blockquotes or list items (`> ` repeated 255 times before a
character: 512 bytes on one line) writes past the runtime's buffer, and the
runtime's `assert(length <= 1024)` aborts the editor, and in daemon mode every
buffer the daemon holds. E7g review 1 found it; the owner ruled at E7h that a
local bound like this ships fixed in a vendored copy (D36 as amended) rather
than unshipped. Held by `tests/e7g_review1_serialization_witnesses.rs`.

To drop this copy: a crates.io release whose `serialize` is bounded, fuzzed clean
with `scripts/fuzz-grammars --grammar markdown --grammar markdown_inline
--seconds 600`, the `[patch.crates-io]` entry and this directory removed in the
same commit.
