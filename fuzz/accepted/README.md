# Accepted findings

`fuzz/accepted.tsv` lists the grammar fuzz findings the owner has accepted,
and this directory holds the reproductions its rows name. **Adding a row is
the owner's ruling, never a session's**: a session that meets a finding it
believes known files or comments the issue and leaves the run red.

A finding a row names is reported `known, accepted (#N)` in the run's report
and its note, and does not set the run's exit status. Everything else fails
as before. A row is removed in the commit that meets its removal condition.

## How a finding is matched

A finding matches a row when all three hold:

- its grammar is one of the row's. A row names every route to its defect,
  because an entry is keyed to the defect and not to the path a fuzzer took
  to it: markdown_inline's classes are reached directly and through
  markdown's inline injection, the way a `.md` file meets them in the
  editor, so #296's and #301's rows name `markdown` and `markdown_inline`
  (the owner's ruling at E7h's fix round 3, after a 600 s run over
  `markdown` met #296 that way:
  `fuzz/repro/markdown-296-through-injection-20515.input`);
- its kind, after triage, is one of the row's kinds (`memory`, `hang`, and
  so on);
- its minimal input is at least half covered by the repeated unit of one of
  the row's reproductions. Coverage is counted over the input's
  non-whitespace characters: indentation and line breaks are layout, the
  route and not the defect (since fix round 3, when #296's underscores
  reached markdown_inline on lazy-continuation lines indented hundreds of
  spaces, 59% of a minimum the minimizer could not shrink by lines:
  `fuzz/repro/markdown-inline-296-indented-7414.input`).

The unit is the run of one to four characters that covers most of a
reproduction, counted without overlap, reduced to its primitive root and
named by its least rotation (`pmacs_grammar_fuzz`'s `dominant_unit`). For
#296 these are `_` and `*`, the delimiter runs whose paragraph makes
`ts_parser__accept` grow. For #301 they are `![f` and `*f[`, the nested image
and link openers whose stack condensation the deadline cannot reach. A
reproduction its own unit covers less than half of is refused.

**What the match tells apart.** A different defect in the same grammar does
not match. Its minimal input is what triggers it, not a run of those units,
and minimizing strips whatever of them it was found inside.
`tests/e7h_grammar_gate_acceptance.rs` holds this with planted defects
found inside #301's openers and inside #296's underscores, in
markdown_inline and again through markdown.

**What it cannot tell apart.** It cannot separate a different defect whose
minimal input is itself such a run. And a variant of an accepted class with
another unit is not accepted until the owner adds its reproduction, by
design.

## The reproductions

| file | finding | where it came from |
|---|---|---|
| `296-underscores-16k.input` | #296, `_` | the issue's generator, 28 lines (16,492 bytes); alone under the `ubsan` build 33.6 s and 5.29 GB |
| `296-asterisks-8k.input` | #296, `*` | 8,237 asterisks, an allocation E7h review 2's sweep reached from ordinary seeds (`asan-strict` arm) |
| `301-nested-openers-98.input` | #301, `![f` | `'*bar**\n' + 'f![' * 28 + 'f*bark]'`, 37 s natively against a 5 s deadline |
| `301-hang-596.input` | #301, `![f` | a hang reproduced alone by E7h review 2's second 600 s markdown_inline run |
| `301-hang-7672.input` | #301, `*f[` | a hang reproduced alone by E7h review 2's first 600 s markdown_inline run |
