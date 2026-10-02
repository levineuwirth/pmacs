# E7h review 1 probes

The probes behind `tests/e7h_review1_probes.rs` and the review pass, kept so
a fix round can rerun them. None is a test target; each prints what it
measured and the build or pty it measured it with.

- `drv.c` parses a file cold (optionally N times in one process), then
  retypes it a byte at a time with an incremental reparse after each byte.
  Build it with `-DLANG=tree_sitter_<name>` beside the runtime and a grammar.
- `haskell_matrix.sh` builds tree-sitter-haskell 0.23.1 on the 0.26.8 runtime
  with the system compiler at -O2 and -O3, with and without
  `-fno-strict-aliasing`, under no sanitizer, ASan, ASan and UBSan (as the
  fuzz job builds) and UBSan, and runs the two-pragma file and three of E7g's
  crashers through `drv.c`. On GCC 16.2.1 without the flag, ASan alone fails
  8 of 8 runs and the plain build aborts 1 of 8, while ASan with UBSan and
  UBSan alone fail none: UBSan has no aliasing check, and its
  instrumentation stops GCC exploiting this one. With the flag, every build
  fails none. GCC 11.4 (the Linux release builder), GCC 13.3 (CI's) and
  clang 22.1.8, run plain and under ASan, fail none with or without it.
- `lua_plant.patch` plants defects in tree-sitter-lua 0.5.0's scanner, fired
  by `@Z` in a block comment or block string and chosen by `PMACS_PLANT`:
  `crash`, `hang`, `grow` (never returns, ~225 MB/s), `slow` (15 s),
  `slow130`, `sized` (2.5 s per trigger), `afterparses` (a crash once the
  process has created `PMACS_PLANT_N` scanners, which no single input
  reaches) and `bigreturn` (about 6 GB touched, then freed and returned).
  Apply it to a copy of the crate routed in through `[patch.crates-io]` and
  run `scripts/fuzz-grammars --grammar lua --from plant=<trigger seeds>` at
  CI's smoke settings. Each mode was first shown firing through `drv.c`
  under the harness's own compiler flags.
- `pty_probe.py` opens the TUI in a pseudo-terminal of a real size (set on
  the slave before exec) with isolated state, sends C-x C-c at a chosen
  second and reports how long the editor took to answer, its CPU and peak
  RSS, and the exit status. `PROBE_NO_LSP=1` writes an `init.lua` emptying
  `pmacs.lsp.config`, so no language server attaches; without it a `.rs`
  file brings up rust-analyzer, which the review's first pass learned the
  hard way.
- `resp.py` measures the stall a user feels: it sends `M-x` 1 s after
  opening and again after each of N typed characters, and times the
  prompt's appearance on screen (`--no-lsp` as above). The review's #292
  table is this probe's.
