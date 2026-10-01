# Kept reproductions

Inputs a fuzz run found that a pin in the suite names, or that a ruling of
the owner's waits on, kept here so they outlive the scratch directory of the
session that found them.

## `typescriptreact-hang-74.input`

74 bytes, `\t\t\t\ttypt*/: T]: T } & { [g: G]: G }` twice with a blank line
before and after. E7h.5's full-length run found it in tree-sitter-typescript
0.23.2's TSX grammar: `hang`, `one parse over 10000 ms`, reproduced alone,
with 8 edits and seed 15527335689300254787. It is one reason the JavaScript
family stays out (`e7g_the_javascript_family_stays_unshipped` in
`tests/e7g_grammar_fuzz_acceptance.rs`).

Replay it with the family registered again, since the harness drives only
the grammars `BUILTIN_LANGUAGES` holds: move `tree-sitter-typescript` and
`tree-sitter-javascript` into `[dependencies]`, add their `LanguageEntry`s
back to `src/syntax.rs` as they stood at `0b72108`, build
`pmacs_grammar_fuzz`, then

    pmacs_grammar_fuzz repro typescriptreact fuzz/repro/typescriptreact-hang-74.input --edits 8 --seed 15527335689300254787

E7h review 1 did that in a worktree of `0a85d18` (a plain `fuzz` build, GCC
16.2.1): the parse did not return in 90 s, RSS 146 MB at 10 s and 926 MB at
60 s, linear; TypeScript and JavaScript on the same bytes and edits finished
37 parses in about a second. E7h's replay through the harness ran it to a
60 s deadline at 853 MB.

## `markdown-296-through-injection-20515.input`

20,515 bytes. E7h's fix round 2 found it with a 600 s run of the `ubsan` arm
over `markdown`, the block grammar, at `02fe7af` (GCC 16.2.1): `memory`,
`exceeded memory cap at 4 GB (cut at 4.0 GB after 113.4 s)` on a 241,490-byte
mutated input, reproduced alone and minimized to this. This minimum returned
alone in 27.4 s at a 2.9 GB peak. It is not on `fuzz/accepted.tsv`, whose #296
row names `markdown_inline`, so that run exited 1.

It is #296 reached through markdown's inline injection. The file is one line
of 73 spaces and underscores, a list item, then an unindented paragraph of
underscore runs whose lines, indented 74 to 87 spaces, are lazy
continuations. Natively, through `tests/e7h_review2/drvp.c`, the block grammar
alone parses the file in 0.27 s at 6 MB. `markdown_inline` on the 20,011-byte
paragraph takes 50.2 s and peaks at 8.87 GB, with a 49.5 s gap between
progress callbacks after the last byte, in `ts_parser__accept`. Whether the
accepted list names #296 under `markdown` too is the owner's ruling
(commented on #296).

    pmacs_grammar_fuzz repro markdown fuzz/repro/markdown-296-through-injection-20515.input
