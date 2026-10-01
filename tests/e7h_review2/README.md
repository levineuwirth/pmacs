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

#296's input, as the issue generates it (`l` repeated 14 times is 8 KB, 28
times 16 KB, 56 times 32 KB):

    python3 -c "l='_'*582 + 'a \`_\`_'; print('\n'.join([l]*28))" > us-16k.md
