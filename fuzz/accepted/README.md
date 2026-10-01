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

- its grammar is the row's;
- its kind, after triage, is one of the row's kinds (`memory`, `hang`, and
  so on);
- its minimal input is at least half covered by the repeated unit of one of
  the row's reproductions.

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
`tests/e7h_grammar_gate_acceptance.rs` holds this with planted defects in
markdown_inline: one found inside #301's openers, and one inside #296's
underscores.

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
