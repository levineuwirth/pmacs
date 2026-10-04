# Regression inputs

Inputs every arm of `scripts/fuzz-grammars` replays through the editor's
parse worker (`pmacs_grammar_fuzz replay-unit`, E7i.2), one directory per
grammar the worker parses them as. Each reproduces a finding of a kind
that took the editor down before E7i: the replay must report it contained
or answered, and never crashed, or the arm fails. A directory's route need
not reach the defect; what each input does there is said below.

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
| `301-hang-596.input` and `.edits` | #301, `![f` | a hang reproduced alone by E7h review 2's second 600 s markdown_inline run, `--edits 8 --seed 15703056251634817165`: its bytes parse at once; the hang is in the edits that seed made, recorded beside it |
| `301-hang-7672.input` | #301, `*f[` | a hang reproduced alone by E7h review 2's first 600 s markdown_inline run, stored without its 33 leading spaces (7,639 bytes) since E7i's fix round 1 |

What the replay (`pmacs_grammar_fuzz replay-unit`, as every arm runs it:
a 5 s deadline, 1 GiB) shows of each, through markdown's route, the way
a `.md` file meets them. Measured at E7i's fix round 1 with a release
worker and a debug one; under the ASan arms the parse is slower and
larger, so what the deadline or the watch stops there is a superset. The
`tsan` arm is not: it replays at a 30 s deadline and 4 GiB by the watch,
its shadow memory resident beside the parse, so it stops less. Its column
was measured at E7i's fix round 2 with fix round 1's TSan worker
(`79ca25d2`, kept under `~/build/e7i-fix1/tsan-workers/`) at those
limits, as E7i review 2 had found it.

| file | released worker | debug worker | `tsan` arm (30 s, 4 GiB) |
|---|---|---|---|
| `296-underscores-16k.input` | stopped by the memory limit (`RLIMIT_AS`) | the same | stopped at the deadline |
| `296-asterisks-8k.input` | **answered**: it returns, in about 1.8 s | stopped at the deadline | **answered** |
| `301-nested-openers-98.input` | stopped at the deadline (it returns after 23 s) | the same | the same |
| `301-hang-596.input` with its edits | stopped at the deadline (in-process, the third recorded parse never returns) | the same | the same |
| `301-hang-7672.input` | stopped at the deadline | the same | the same |

"Stopped at the deadline" is any of three things the replay reports as
contained by time: the worker cancelling the parse where tree-sitter
calls its progress callback, with no tree; the same for an injected
layer, the tree returned without the layers after it; or the harness
killing the worker a grace period after, where the callback is not
reached (#301's condensation). Which one a never-returning parse meets
depends on where it stands when the deadline passes.

So the replay proves the worker contains #296's underscore paragraph by
its memory limit and #301 by the deadline, three ways. It does not prove
#296's asterisks contained in a shipped worker: there they are a large
parse that returns, and only the overhead of a debug build or an ASan arm
(the watch, in CI) stops them; the `tsan` arm answers them, and stops the
underscores by time and not by memory.

Two records corrected at E7i's fix round 1 (review 1, Low 3). The 7,672
bytes as found began with 33 spaces, so markdown read them as an indented
code block, never injected the inline grammar, and the replay answered
them; without the spaces they are a paragraph, and hang through
markdown's route as they did through markdown_inline's. The 596 bytes
alone return at once by any route; their hang is the edit sequence, which
`replay-unit` now applies from `301-hang-596.edits` (one array per parse,
each edit `[start, old_end, inserted]` in the text that edit meets): the
whole text parsed, then each recorded parse's edits sent as the editor
sends them, with reads of the tree while each runs.

`tests/e7h_review2/accepted-296-301.tsv` keeps the two retired rows, naming
these files, so the accepted list's matching stays under test while the
list itself is empty.
