# Regression inputs

Inputs every arm of `scripts/fuzz-grammars` replays through the editor's
parse worker (`pmacs_grammar_fuzz replay-unit`, E7i.2), one directory per
grammar the worker parses them as. Each was a finding that took the editor
down before E7i and that the worker now stops at its own limit: the replay
must report it contained (or answered) and never crashed, or the arm fails.

## `markdown/`

The reproductions of #296 and #301, moved here from `fuzz/accepted/` when
their rows retired at E7i.5 (the worker stops both in the editor:
`tests/e7i_parse_containment_acceptance.rs`). They are markdown_inline's
defects, replayed as markdown because a `.md` file meets them through
markdown's inline injection, the way the editor parses it.

| file | finding | where it came from |
|---|---|---|
| `296-underscores-16k.input` | #296, `_` | the issue's generator, 28 lines (16,492 bytes); alone under the `ubsan` build 33.6 s and 5.29 GB |
| `296-asterisks-8k.input` | #296, `*` | 8,237 asterisks, an allocation E7h review 2's sweep reached from ordinary seeds (`asan-strict` arm) |
| `301-nested-openers-98.input` | #301, `![f` | `'*bar**\n' + 'f![' * 28 + 'f*bark]'`, 37 s natively against a 5 s deadline |
| `301-hang-596.input` | #301, `![f` | a hang reproduced alone by E7h review 2's second 600 s markdown_inline run |
| `301-hang-7672.input` | #301, `*f[` | a hang reproduced alone by E7h review 2's first 600 s markdown_inline run |

`tests/e7h_review2/accepted-296-301.tsv` keeps the two retired rows, naming
these files, so the accepted list's matching stays under test while the
list itself is empty.
