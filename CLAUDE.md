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
- A tree-sitter grammar is C beside the `forbid(unsafe_code)` Rust, and
  in daemon mode one bad parse takes every buffer the daemon holds. A
  grammar ships, or its crate is bumped, only once `scripts/fuzz-grammars
  --grammar <name> --seconds 600` has run over it clean on the compiler
  that builds the release pair, seeded from real files through its row in
  `fuzz/corpora.tsv`, and the PR cites the report; a `tree-sitter`
  runtime bump runs every grammar, and a change under `vendor/` or
  `builtin/queries/` the grammars it touches. The script builds the C at
  release optimization three ways and fails if any finds a defect:
  `ubsan`, as it ships (`.cargo/config.toml`'s `-fno-strict-aliasing`)
  under ASan and UBSan; `asan-strict`, ASan alone with strict aliasing
  restored, so that a compiler may exploit aliasing UB; and `tysan`,
  clang's TypeSanitizer, which reports an access of the wrong type on any
  compiler. CI's grammar fuzz runs on every change, fuzzes when one can
  change the grammar set (`scripts/grammar-fuzz-needed`; the touched
  grammars ten minutes, the rest briefly), and builds two arms on
  `ubuntu-24.04`'s GCC 13, which it prints: `ubsan`, the check named
  `Grammar fuzz`, and `asan-strict`, `Grammar fuzz (asan-strict)`. On the
  inputs a run reaches, CI catches a memory error ASan sees at the access,
  the UB UBSan instruments, a parse that never returns, one past the
  memory cap, and aliasing UB that GCC 13 turns into a memory error under
  `asan-strict` (E7h review 2 measured one shape on GCC 13.3, a stored
  length forwarded past a write through a narrower lvalue; it never
  exploits tree-sitter-haskell 0.23.1's). The `ubsan` arm is blind to
  aliasing UB because it is built with the flag, as it ships, which
  forbids every compiler to exploit it, and UBSan has no aliasing check.
  Only the laptop's run adds GCC 16, which exploits more of it, and
  `tysan`, which reports the access on any compiler. Since E7h's fix round 1 no shipped scanner carries the
  aliasing header (`ALIASING_HEADER_RESIDUAL` is empty), so those arms
  guard the next grammar. Neither run catches an input the mutator does
  not reach, a defect only the release builders' GCC 11 or Apple clang
  would produce, or UB no arm instruments (a ctype table read past its
  end into mapped memory, #302, which a row now forbids), and a green run
  is not a proof. `build.rs` refuses to compile C that
  `-fno-strict-aliasing` does not reach, as in a build cargo did not start
  inside the checkout, which passes it with `CFLAGS`. E7e's lesson: the author's chosen test file parsed while the
  owner's real one, two `{-# LANGUAGE #-}` pragmas, aborted the editor;
  tree-sitter-haskell 0.23.1's published `array.h` is undefined behavior
  that GCC 16 turned into a heap overflow at -O2 and CI's GCC 13 did
  not. D36 as amended at E7h: what aborts or never returns is unshipped
  unless the defect is a local bound fixed in a vendored copy; what is
  slow or large is filed; and a parse whose memory grows to an
  out-of-memory kill is neither, since it takes the editor down as an
  abort does (#296, markdown: an instance memory limit, the wasm
  phase's, is what bounds it). The harness confirms a finding alone on
  the input that showed it, and what that input did decides its kind;
  then it minimizes and reports the minimum's outcome beside it. It fails
  a parse cut at four times its memory limit, reported as "exceeded
  memory cap" with the RSS at the cut (the parse's own peak is not
  known), and never calls it hung. The time limit is a trigger, not a
  verdict (the owner's ruling): an input that runs past the
  confirmation's time limit while its minimum returns is run alone again
  under ten times that limit, and is slow, filed with its time, if it
  returns there and a hang that fails the run if it does not, so a
  quadratic parse passes and one that does not terminate fails (the
  `tysan` arm, whose time and memory are the sanitizer's, fails on its
  crashes alone). A
  finding `fuzz/accepted.tsv` names, by grammar, kind and the repeated
  unit of a reproduction, is reported "known, accepted (#N)" and does not
  fail: markdown_inline's #296 and #301, until the wasm phase's limits.
  A row is keyed to the defect, not the route, so those two name
  `markdown` too, through whose inline injection a `.md` file meets
  them.
  Adding a row is the owner's ruling, never a session's; a session that
  meets a finding it believes known files or comments the issue and
  leaves the run red.
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
