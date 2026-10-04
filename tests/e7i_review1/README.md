# E7i review round 1's instruments

Witnesses and instruments of the review of PR #309 at `fa176de` (2026-10-03).
The rows themselves are `tests/e7i_review1_probes.rs`; this directory holds
what they and the pass used that is not a Rust test.

| file | what it is |
|---|---|
| `r1_run.py` | E7i.1's measurement driver (`~/build/e7i/e7i_run.py`), with an `--impl` arm: the built tip's release pair or the comparison's retained prototype binaries; the probe is the comparison's headless `pmacs-gpu` |
| `r1_campaign.py` | the comparison's campaign shape: six interleaved rounds a file, 40 samples a run, arm order shuffled per round (seed 20261003); then the pathological-parse, steady-state and acceptance runs |
| `r1_analyze.py` | its tables, with the comparison's tie band per quantile per file |
| `campaign-analysis.md` | the analyzer's output for the review's campaign (77 runs, 2026-10-03 20:42 to 21:29 local) |
| `archive_probe.py` | a 40x120 pty run of a `pmacs` alone in a directory, as `release.yml` stages it: what the screen shows without the parse worker |
| `time_regress.py` | each `fuzz/regress/markdown/` input and its variants alone through a worker, timed, with a long deadline |
| `strip-7672.input` | `301-hang-7672.input` without its 32 leading spaces: a paragraph, not an indented code block; hangs through markdown's route |
| `fence-7672.input` | the same bytes inside a ```` ```markdown_inline ```` fence of a markdown document |
| `tsan-outlive-latex.stderr` | the ThreadSanitizer worker's two reports under `e7i_review1_a_read_that_outlives_two_installs` (LaTeX's `core_spec.tex`, 20 iterations): a plain read of a subtree's `ref_count` in `ts_subtree_release` (`ts_assert(child.ptr->ref_count > 0)`, a `volatile` load) against an atomic decrement, two evicted trees freed at once by the serve thread and the parse thread |

The ThreadSanitizer worker was built from this tree with

    RUSTC_BOOTSTRAP=1 RUSTFLAGS="-Zsanitizer=thread -Cforce-frame-pointers=yes" \
    CC=clang CFLAGS="-fsanitize=thread -fno-strict-aliasing -g -O1" \
    cargo build -Zbuild-std --target x86_64-unknown-linux-gnu --release -p pmacs-parse-unit

on the pinned 1.95.0 with its `rust-src` component, and driven with
`PMACS_REVIEW_UNIT=<that binary>` and `TSAN_OPTIONS=halt_on_error=0`.
