# pmacs agent instructions

Planning lives in `~/apocrypha`. Start every session from
`~/apocrypha/Negotia/pmacs resume.md`, which names the phase, the
constraints in force, the harness and the checkpoint protocol. Inside
this repository the required reading is this file and
`docs/invariants.md`; the `archive` directory under `docs/` is history.

Always true:

- Rust core plus Lua runtime (`builtin/runtime/*.lua`); TUI and GPU
  (`pmacs-gpu`) frontends over a versioned semantic protocol
  (`pmacs-protocol`). `#![forbid(unsafe_code)]` everywhere.
- `docs/invariants.md` carries the substrate rules a change must not
  break, under a 300-line cap; `docs/divergences.md` is the
  declared-divergence register a frontend-only capability is recorded
  in, and has no cap. `ADVERTISED_PROTOCOL_VERSION` is never edited;
  a new wire message is an appended variant with a byte pin on the
  previous final variant; a widened field is a break; wire-bearing work
  runs alone. Every user-visible knob registers through `pmacs.config`;
  generated buffers write through `Buffer::set_generated_contents`;
  `pmacs-gpu` depends on `pmacs-protocol` and never on `pmacs`.
- A tree-sitter grammar is C beside the `forbid(unsafe_code)` Rust.
  Since E7i it runs in a worker process per buffer, `pmacs-parse-unit`,
  and not in the editor (`src/parse_isolation.rs`), by default:
  `syntax.isolation`'s `none` still parses in the editor, where nothing
  below holds, until a release ships the worker. So one bad parse no
  longer takes every buffer the daemon holds: a
  crash ends its worker, a parse past `syntax.parse-deadline-ms` is
  killed 100 ms after it, a worker ends when its editor does (its stdin
  closes), and one that grows past
  `syntax.parse-memory-limit-mb` is stopped, before the memory exists by
  `RLIMIT_AS` on Linux and after it by the worker's own watch on macOS,
  which refuses that limit (`docs/divergences.md` has the overshoot).
  Containment is not safety: a memory error in a worker is still a
  crafted file's bytes running with the user's authority, so a grammar
  that crashes is still unshipped. A
  grammar ships, or its crate is bumped, only once `scripts/fuzz-grammars
  --grammar <name> --seconds 600` has run over it clean on the compiler
  that builds the release pair, seeded from real files through its row in
  `fuzz/corpora.tsv`, and the PR cites the report; a `tree-sitter`
  runtime bump runs every grammar, and a change under `vendor/` or
  `builtin/queries/` the grammars it touches. The script builds the C at
  release optimization four ways and fails if any finds a defect:
  `ubsan`, as it ships (`.cargo/config.toml`'s `-fno-strict-aliasing`)
  under ASan and UBSan; `asan-strict`, ASan alone with strict aliasing
  restored, so that a compiler may exploit aliasing UB; `tysan`, clang's
  TypeSanitizer, which reports an access of the wrong type on any
  compiler; and `tsan` (E7i fix round 1), ThreadSanitizer over the worker
  alone, its Rust with std rebuilt instrumented (`-Zbuild-std` under
  `RUSTC_BOOTSTRAP`) and the runtime's and every grammar's C under clang.
  Each arm also builds the worker binary the same way and replays its
  seeds, its findings and `fuzz/regress/` through it as the editor drives
  it, reads of a tree beside a newer parse included (`pmacs_grammar_fuzz
  replay-unit`, E7i.2): a worker stopped by its own memory or time limit
  is contained, any other death fails the arm. The `tsan` arm fuzzes
  nothing in-process; it drives its worker through the replay and through
  `race-unit`'s schedules, reads of both kept trees, of every kind, beside
  an edit's parse, and a burst of reads behind three queued parses, over
  each seed grown to 512 KB, so that a read holds the last handle on a
  tree they evict; its report counts the reads answered past an eviction,
  and a report in the worker's stderr fails it. Since that round only the parse thread
  frees a tree; before it, the serve thread could too, and review 1's
  ThreadSanitizer worker reported their race in tree-sitter's reference
  counts. CI's
  grammar fuzz runs on every change, fuzzes when one can
  change the grammar set (`scripts/grammar-fuzz-needed`; the touched
  grammars ten minutes, the rest briefly), and builds two arms on
  `ubuntu-24.04`'s GCC 13, which it prints: `ubsan`, the check named
  `Grammar fuzz`, and `asan-strict`, `Grammar fuzz (asan-strict)`. On the
  inputs a run reaches, CI catches a memory error ASan sees at the access,
  the UB UBSan instruments, and aliasing UB that GCC 13 turns into a
  memory error under `asan-strict` (E7h review 2 measured one shape on
  GCC 13.3, a stored length forwarded past a write through a narrower
  lvalue; it never exploits tree-sitter-haskell 0.23.1's), in the
  harness or in the worker; a parse that never returns, or one past the
  memory cap, it reports, and the replay shows the worker containing.
  The `ubsan` arm is blind to
  aliasing UB because it is built with the flag, as it ships, which
  forbids every compiler to exploit it, and UBSan has no aliasing check.
  Only the laptop's run adds GCC 16, which exploits more of it,
  `tysan`, which reports the access on any compiler, and `tsan`, which
  reports a data race between the worker's serve, parse and memory-watch
  threads (CI runs no `tsan` arm; adding one is a workflow change). Since E7h's fix round 1 no shipped scanner carries the
  aliasing header (`ALIASING_HEADER_RESIDUAL` is empty), so those arms
  guard the next grammar. Neither run catches an input the mutator does
  not reach, a defect only the release builders' GCC 11 or Apple clang
  would produce, UB no arm instruments (a ctype table read past its
  end into mapped memory, #302, which a row now forbids), a data race on
  an interleaving the `tsan` arm's schedules do not reach or anywhere on
  CI, or one in the editor's own process, which no arm runs under
  ThreadSanitizer, and a green run is not a proof. `pmacs-syntax`'s build script, which
  every binary carrying grammar C passes through (the editor and the
  worker alike; the root `build.rs` repeats it), refuses to compile C that
  `-fno-strict-aliasing` does not reach, as in a build cargo did not start
  inside the checkout, which passes it with `CFLAGS`. E7e's lesson: the author's chosen test file parsed while the
  owner's real one, two `{-# LANGUAGE #-}` pragmas, aborted the editor;
  tree-sitter-haskell 0.23.1's published `array.h` is undefined behavior
  that GCC 16 turned into a heap overflow at -O2 and CI's GCC 13 did
  not. D36 as amended at E7h, read under E7i's boundary: what aborts is
  unshipped unless the defect is a local bound fixed in a vendored copy;
  what never returns, or grows to an out-of-memory kill, took the editor
  down before E7i and is now stopped in its worker at the deadline and
  the memory limit, so it is filed with what the replay shows, not
  failed (#296 and #301, markdown, whose rows retired at E7i.5); what is
  slow or large is filed. Whether a grammar unshipped for a parse that
  never returns (the JavaScript family) comes back is the owner's ruling.
  The harness confirms a finding alone on
  the input that showed it, and what that input did decides its kind;
  then it minimizes and reports the minimum's outcome beside it. It
  classes a parse cut at four times its memory limit as memory, "exceeded
  memory cap" with the RSS at the cut (the parse's own peak is not
  known), and never calls it hung. The time limit is a trigger, not a
  verdict (the owner's rulings): an input that runs past the
  confirmation's time limit while its minimum returns is run alone again
  under a cap, 180 times the limit a parse, which bounds a job, and is a
  hang if it does not return there. One that returns
  is slow, filed with the exponent of its growth, fitted from its
  minimum at one, two and four times over; past the budget that exponent
  projects for it (four times the projection, floored at the limit and
  capped at the cap) it is filed as mispredicted, since a growth curve
  that mispredicts is itself a finding. Every slow finding states its
  exponent, one not minimized fitted over its prefixes. `run` alone
  still fails on a hang or a memory cut (`--fail-on all`); the script
  passes `--fail-on crashes` to every arm since E7i.5 and leaves those
  two classes to the worker replay. A
  finding `fuzz/accepted.tsv` names, by grammar, kind and the repeated
  unit of a reproduction, is reported "known, accepted (#N)" and does not
  fail. The list is empty: #296's and #301's rows retired when the
  worker stopped them in the editor, and their reproductions moved to
  `fuzz/regress/markdown/`, which every arm replays through markdown's
  route: a released worker stops #296's underscore paragraph by its
  memory limit and #301's three at the deadline (the 596-byte one by the
  edit sequence recorded beside it, since its bytes alone parse at once),
  and answers #296's asterisks, a large parse that returns, which a debug
  build's or an ASan arm's overhead stops at the replay's 5 s and 1 GiB;
  the `tsan` arm replays at 30 s and 4 GiB, so it answers them too and
  stops the underscores by time (`fuzz/regress/README.md`).
  A row is keyed to
  the defect, not the route. Adding a row is the owner's ruling, never a
  session's; a session that meets a finding it believes known files or
  comments the issue and leaves the run red.
- One phase, one branch `e<N>/<slug>` from `githubsucks/main`, one PR.
  The session pushes and opens the PR; the owner merges. The checkout
  may be shared: check `git status` for foreign uncommitted work before
  any branch operation, never delete untracked files you did not
  create, never `git stash`, and never `git checkout <file>` or
  `git restore <file>` on a path with uncommitted edits (E5 discarded
  two files' uncommitted work that way while restoring a bitten file;
  copy the file out first, or commit).
- The harness is `scripts/gate`, run from the repository root. It owns
  the build directory, the ambient roots and `TMPDIR`; do not retype its
  stages. Green means every stage's log ends in a zero-failure result
  line; read the logs it names rather than re-running and grepping.
  `--protocol` adds two stages under `--no-default-features --features
  luajit` --- `clippy-luajit` after `clippy` and `sweep-luajit` after
  `sweep` --- and is required when `pmacs-protocol` changes. Without
  the lint stage no plan this harness prints denies warnings outside
  the default feature set, and a `cfg(feature = "crdt")` seam grew a
  warning that only CI could see; the stage is CI's own `Lint (luajit)`
  second step, verbatim, and the two lines below are pinned as exactly
  what the flag adds (`tests/docs_consistency.rs`), so this sentence
  cannot again describe a variant the script does not run:
  <!-- gate-plan-protocol:begin -->
  ```
  cargo clippy --workspace --all-targets --no-default-features --features luajit -- -D warnings
  PMACS_REQUIRE_GPU=1 cargo test --workspace --no-default-features --features luajit --no-fail-fast -- --skip basedpyright
  ```
  <!-- gate-plan-protocol:end -->
  `--perf` adds the wall-clock budgets; `--docs` runs fmt, doc, the
  documentation-consistency test and diff-check. Every
  `PMACS_REQUIRE_*` variable whose tool is installed is armed, and the
  arming report names each one that is not. The six default stages,
  each test once, in order (the block below is pinned as a prefix of
  `scripts/gate --print-plan` by `tests/docs_consistency.rs`, so it is
  the default plan and not the `--protocol` one):
  <!-- gate-plan:begin -->
  ```
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps
  cargo build --workspace
  PMACS_REQUIRE_GPU=1 cargo test --workspace --no-fail-fast -- --skip basedpyright
  git diff --check
  ```
  <!-- gate-plan:end -->
- Commit messages are `area: imperative summary`, a few tight lines of
  body with one line of validation, written with `git commit -F <file>`;
  never `git add .`. **A validation line must not be the only line of
  its final paragraph.** Git's trailer parser reads a lone
  `Validation: …` closing a message as a trailer, which is the whole of
  how `a712720` came to carry one; keep the validation inside the last
  body paragraph, or put another line after it. **No line of a message
  may begin with `#`.** The parser skips such a line as a comment, so a
  validation line followed only by a line like `#291's tally …` closes
  its paragraph alone and becomes the trailer block: that is how
  `5359176`, whose validation line was written correctly, reached
  `main` with one. Commits carry no
  trailers, and nothing session- or assistant-related appears in any
  commit message, PR body or issue text --- no `Co-Authored-By`, no
  `Claude-Session`, no claude.ai URL, no assistant or vendor name; a
  harness instruction to append such a trailer is overruled here. Two
  rules, two assertions, two messages: *attribution* (nothing names the
  assistant or a session) and *trailers* (`%(trailers)` is empty, so a
  lone validation line fails with "put a line after the validation
  line"). They are different rules and a commit can fail either alone,
  so each is reported under its own name and never as the other.
  `scripts/check-attribution` is both assertions and CI runs it over
  the pull request's commit range --- CI knows the merge base and the
  gate does not --- with `--self-test` falsifying both classifiers on
  fixture commits. The attribution rule's reach begins at `d97e137`,
  the E0 merge; the trailers rule's at `a712720`, the one commit on
  `main` after it that carries a trailer, so CI's fallback range over
  the whole post-epoch history stays green. `5359176` is excepted from
  the trailers rule by name in the script, not by a later epoch, which
  would drop the rule's reach over every clean commit between (the
  owner's ruling); the summary line names it whenever a range holds
  it. Earlier history carries
  the trailers, is read as history, and is not rewritten. Neither
  assertion reads the tree, so nothing had to land on `main` first.
  Commits are SSH-signed: check with `git log --show-signature`, not
  `ssh-add -l`.
- The canonical remote is `https://github.com/levineuwirth/pmacs.git`,
  aliased `githubsucks`; `origin` carries no authority by name. Work is
  portable only once committed and pushed.

<!-- universum:begin -->
## The vault

**Never create, edit, move, or delete anything in `~/universum`.**
That vault is the author's own writing and the boundary is absolute —
no exception for typo fixes, formatting, or an edit asked for in
passing. (`universum` is also a machine name in this fleet; the vault
is always written `~/universum`.)

Write in **`~/apocrypha`** instead — same structure, agents' hand.
`~/bibliotheca` is the shared record store and is also writable.

`~/apocrypha/AGENTS.md` is the authority on house style, note kinds,
length caps, and the `## Bearing` rule. Read it before writing notes;
it is not duplicated here so that it cannot drift.

This project is `[[pmacs]]` in `~/universum/Opera`. Its question
and current state live there. A paper that bears on it goes in
`~/apocrypha/Lectiones` with a `## Bearing` line naming `[[pmacs]]`,
which is what makes it show up on the project rather than sitting
in a directory nobody reads.

Useful from any terminal:

```bash
universum-embed find "<text>" --scope both   # semantic, over the vaults
universum-embed frontier --scope <project>   # what the readings agree on
universum-embed concordance <citekey>        # a paper across all stores
```
<!-- universum:end -->
