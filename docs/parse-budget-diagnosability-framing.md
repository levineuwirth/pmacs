# Parse-budget diagnosability — reporting the measurement that failed

**Status: revision 2 — AWAITING APPROVAL. Nothing implemented.**

Revision 2 answers review of 1. Three of its four changes are
corrections, and one is a scope reduction:

- **SPLIT.** Revision 1 bundled this with adding `workflow_dispatch` to
  `ci.yml`. `docs/active-work.md` already records those as "two
  follow-ups … both their own lanes", and a shared thesis is not an
  atomic feature boundary. **`workflow_dispatch` is no longer in this
  framing**; it gets its own.
- **The census was WRONG.** Revision 1 called
  `dispatch_parse_round_trips_a_rust_source_file` the sole outlier.
  `tests/m4_acceptance.rs:244` asserts the same measurement against the
  same budget and also omits it. **Both are now in scope.**
- **The completeness claim is WITHDRAWN**, not repaired. See §3.
- The concurrency reasoning revision 1 got backwards belonged to the
  other change and leaves with it.

## 1. What this fixes

Two assertions compare a parse duration against a 100ms budget and
report **nothing** about what they measured:

| where | assertion | message |
|---|---|---|
| `src/async_runtime.rs:3328` | `duration_ms < 100` | `"trivial parse should be fast"` |
| `tests/m4_acceptance.rs:244` | `duration_ms < 100` | `"200-line parse should be quick"` |

When either reds, the log says the parse was slow and stops there.

## 2. Why now, and the evidence

The first has redded **twice** on `Test (macos-latest / lua54)`, and
**both margins are unrecoverable**:

| occurrence | where | outcome |
|---|---|---|
| U11 | PR #242, run `32393462318`, job `96504773333` | red, green on rerun |
| U11's recurrence | PR #243, run `33156571314`, job `98800645872` | red, green on rerun |

`src/async_runtime.rs` was **byte-identical to `main`** for both — blob
`9310ce3fca8c5fd8ebd39a68c29ad6985e256049`. U11's own row predicted the
cost:

> *A recurrence is not another instance of this row. Because the margin
> was never captured, a second red cannot be compared with the first.*

That came true once. Nothing prevents a third.

**A 1ms overshoot and a 900ms overshoot are different failures** — one
says a threshold is marginal, the other says something stalled — and
today they produce identical logs.

## 3. Scope, and a claim this framing does NOT make

**In scope: the two assertions in §1.** They are the same measurement
against the same budget, in the same subsystem, and it would be strange
to fix one and leave the other to produce the next unreadable red.

**Out of scope, and deliberately: everything else.** Revision 1 claimed
these were the only measurement-omitting assertions in the codebase.
**That claim is withdrawn and is not replaced by a corrected one.** A
sweep wide enough to be complete also catches `Instant::now() <
deadline` loop guards and `eval::<bool>` turbofish, which are not
budget assertions at all; a sweep narrow enough to be accurate proves
nothing about completeness. **This lane is not an assertion-hygiene
audit and should not be read as one.**

Several nearby budget assertions do already report their measurements —
`composition_overhead_under_ten_percent`, `criterion_1_end_of_line_typing…`
(`optimistic.rs:989`), `dired_open_renders_10k_entries_under_200ms`,
`m6_2_pty_streaming_respects_byte_ceiling` (`observed {in_flight}`).
They are cited as **precedent for the shape**, not as evidence that the
set is exhausted.

## 4. What lands

```rust
assert!(
    duration_ms < 100,
    "trivial parse should be fast: took {duration_ms}ms against a \
     100ms budget"
);
```

and, in `tests/m4_acceptance.rs`:

```rust
assert!(
    duration_ms < 100,
    "200-line parse should be quick: took {duration_ms}ms against a \
     100ms budget"
);
```

**Both budgets stay at 100ms.** Widening is what R1 already rejected,
and it would discard the signal these reds carry.

## 5. Acceptance

| # | contract | witness | mutation |
|---|---|---|---|
| D1 | the `async_runtime` assertion reports the observed ms **and** the budget | a scratch build with the comparison bound forced to `0` panics with a message containing the observed value and `100ms` | restore the bare message → the row cannot separate a 1ms overshoot from a 900ms one |
| D2 | the `m4_acceptance` assertion does the same | same method, same row | same |
| D3 | both budgets are still `100` | the comparison literal is unchanged in both files | widen either → R1's rejected remedy returns |

**D1 and D2 are asserted against a real panic message, not by reading
the source.** A row that greps the format string would pass while the
assertion it describes had been deleted — which is the same
read-the-code-not-the-effect failure this project has repeatedly
caught.

**How the fault is injected, and what each half proves.** Temporarily
force **only the comparison bound** to `0` in a scratch build. The
resulting panic then proves two things and no more: that the observed
value is interpolated, and that the budget text reads `100ms`.

**It does NOT prove the exercised budget was 100ms — it was 0.** The
scratch build compares against `0` while the message still says `100ms`,
so that panic alone says nothing about the committed threshold. **D3
carries that half separately**, by pinning the comparison literal in
both files. The two rows are only a proof together, and neither
substitutes for the other.

This is the **smallest deterministic fault injection** available, not
the only conceivable one. A row that waited for a genuine 100ms
overshoot would be exactly as intermittent as the thing it documents.

## 6. Coherence impact (`COHERENCE.md` §20)

- **Journey steps touched: NONE.** No product behaviour changes. What
  moves is **evidence quality** — whether a failing check can be
  reasoned about — the same axis the `scripts/gate` SIGINT guard sat
  on.
- **Interaction islands: none added.**
- **Config registry: no entry.** A test budget is not a user-tunable.
- **Background work: none started**, and no attribution moves.

## 7. What this does NOT do

- **It does not fix the intermittence**, and makes no claim about
  cause. It makes the next occurrence *comparable*.
- **It does not widen, relax or `#[ignore]` any budget.** Two
  intermittent reds are not evidence a threshold is wrong.
- **It does not add `workflow_dispatch`.** That is its own lane, per
  the ledger's recorded decision.
- **It does not run U9's discriminating control**, which remains
  unrun and is a third, separate piece of work.
