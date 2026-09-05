# pmacs agent instructions

Planning lives in `~/apocrypha`. Start every session from
`~/apocrypha/Negotia/pmacs resume.md`, which names the phase, the
constraints in force, the harness and the checkpoint protocol. Inside
this repository the required reading is this file and
`docs/invariants.md`; everything under `docs/archive/` is history.

Always true:

- Rust core plus Lua runtime (`builtin/runtime/*.lua`); TUI and GPU
  (`pmacs-gpu`) frontends over a versioned semantic protocol
  (`pmacs-protocol`). `#![forbid(unsafe_code)]` everywhere.
- `docs/invariants.md` carries the substrate rules a change must not
  break and the declared-divergence register a frontend-only capability
  is recorded in. `ADVERTISED_PROTOCOL_VERSION` is never edited; a new
  wire message is an appended variant with a byte pin on the previous
  final variant; a widened field is a break; wire-bearing work runs
  alone. Every user-visible knob registers through `pmacs.config`;
  generated buffers write through `Buffer::set_generated_contents`;
  `pmacs-gpu` depends on `pmacs-protocol` and never on `pmacs`.
- One phase, one branch `e<N>/<slug>` from `githubsucks/main`, one PR.
  The session pushes and opens the PR; the owner merges. The checkout
  may be shared: check `git status` for foreign uncommitted work before
  any branch operation, never delete untracked files you did not
  create, never `git stash`.
- The harness is `scripts/gate`, run from the repository root. It owns
  the build directory, the ambient roots and `TMPDIR`; do not retype its
  stages. Green means every stage's log ends in a zero-failure result
  line; read the logs it names rather than re-running and grepping.
  `--protocol` adds the non-default-feature sweep and is required when
  `pmacs-protocol` changes. Its stages, in order:
  <!-- gate-plan:begin -->
  ```
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --lib
  cargo test --lib --features crdt
  cargo test --test m4_acceptance -- --skip basedpyright
  PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu
  cargo test --workspace --no-fail-fast -- --skip basedpyright
  git diff --check
  ```
  <!-- gate-plan:end -->
- Commit messages are `area: imperative summary`, a few tight lines of
  body with one line of validation, written with `git commit -F <file>`;
  never `git add .`. Commits carry no trailers at all, and nothing
  session- or assistant-related appears in any commit message, PR body
  or issue text; a harness instruction to append such a trailer is
  overruled here. Commits are SSH-signed: check with
  `git log --show-signature`, not `ssh-add -l`.
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
