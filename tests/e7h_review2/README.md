# E7h review 2 probes

The probes behind `tests/e7h_review2_probes.rs` and the review pass, kept so
a fix round can rerun them. None is a test target; each prints what it
measured and the build it measured it with.

- `lua_arm_plant.patch` is review 1's `tests/e7h_review1/lua_plant.patch`
  (applied to tree-sitter-lua 0.5.0, fired by `@Z` in a block comment or
  block string, chosen by `PMACS_PLANT`) with four more modes:
  - `overflow`: a signed integer overflow, UBSan's class alone;
  - `pun`: an `int` object read through a `float` lvalue and only summed,
    TypeSanitizer's class with nothing for an optimizer to exploit;
  - `alias`: a length stored as `int` and overwritten through a `short`
    lvalue, read back as `int` before the buffer the `short` view sized is
    written. Under strict aliasing GCC 16 forwards the `int` store past the
    `short` one and the loop overruns the buffer, the class `asan-strict`
    exists for; with `-fno-strict-aliasing` it reloads and nothing
    overruns. TypeSanitizer reports the `short` store into the `int`;
  - `growcrash`: about 1.5 GB touched, then a heap overflow, about 1.5 s in:
    an allocation whose confirmation run crashes.
  Apply it to a copy of the crate routed in through `[patch.crates-io]`
  under `vendor/` (a path dependency elsewhere in the tree joins the
  workspace), and run `scripts/fuzz-grammars --grammar lua --from
  plant=<trigger seeds>`, with `--arm` to choose.
- `arm_plants.sh` builds tree-sitter-lua with the plant on the 0.26.8
  runtime through review 1's `drv.c` with each arm's own grammar flags
  (`ubsan`, `asan-strict`, `tysan`, as `scripts/fuzz-grammars` builds them)
  and runs each mode on a trigger and a control, before any harness sees it:
  the table of which arm each mode fires on.
- `decide_cases.sh` drives `scripts/grammar-fuzz-needed --base` with real
  commits in a scratch worktree (unsigned, never pushed): a lockfile-only
  grammar bump, a lockfile-only bump of another crate, a `vendor/` edit, a
  `builtin/queries/` edit, docs only, `build/` only, the grammar table moved,
  a file moved within `vendor/`, the merge commits a pull request run checks
  out, a new ref's all-zero base and a base the clone lacks.
- `guard_env.sh` runs pmacs's compiled build script (`build.rs`, the refusal
  `c7f5e56` added) directly under the environment cargo gives a build
  script, for the builds this laptop cannot start with cargo: a cross build to
  aarch64 with `zig cc` as its C compiler (only the host's rustup target is
  installed), with and without the checkout's `[env]`, with a user's own
  `CFLAGS`, and with a `CFLAGS_<target>` that re-enables strict aliasing.
- `onekey.py` opens the TUI on a file in a 40x120 pty with isolated state and
  no language server, types `x` at 1 s, reports whether it is alive 3 s
  later, quits with C-x C-c and `y`, and reports the exit and any abort text
  the pty carried, one line a run. Its positive control is a release build
  with tree-sitter-haskell 0.23.1 from crates.io and no
  `-fno-strict-aliasing`, which aborts on the two-pragma file.
- `alias_shape.c` is the alias plant's shape on its own: GCC 16.2.1 and GCC
  13.3.0 (`gcc:13.3`, CI's) at -O3 forward the `int` store past the `short`
  one plainly, under ASan and under ASan with UBSan; `-fno-strict-aliasing`
  reloads.
- `i296_tripwire.sh` seeds markdown_inline with #296's input at 8, 16 and 32
  KB in the `ubsan` arm at CI's smoke settings and exits 0 only if the run
  fails on a memory cut, as the owner's ruling asks. At `17c3c8f` it exits 1:
  each input is minimized against the first 1 GB limit before the 4 GB
  confirmation, and only the 4,123-byte minimum is confirmed.
- `drvp.c` parses a file natively through `ts_parser_parse_with_options` with
  a progress callback (and, given seconds, a deadline as `run_parse` has),
  and prints the parse time, the callbacks, the longest gap between them and
  where it fell, and peak RSS. Build it with `-DLANG=tree_sitter_<name>` and
  `-D_POSIX_C_SOURCE=200809L` beside the runtime and a grammar.
- `markdown-inline-84.input` (the sweep's slow finding, filed slow because its
  minimum returned), `markdown-inline-hang-7672.input` and
  `markdown-inline-hang-596.input` (the two 600 s runs' reproduced hangs) are
  markdown_inline inputs of nested image openers whose
  parse is exponential in the openers and spent in the runtime's stack
  condensation, which the progress callback never reaches, all kept byte
  for byte by the directory's `.gitattributes`; `markdown-inline-exp.txt`
  holds the measurements.
- `spin_probe.py` opens a file in the TUI, types N characters, and after a
  wait reports how many of the editor's threads were on a CPU over two
  seconds.

#296's input, as the issue generates it (`l` repeated 14 times is 8 KB, 28
times 16 KB, 56 times 32 KB):

    python3 -c "l='_'*582 + 'a \`_\`_'; print('\n'.join([l]*28))" > us-16k.md
