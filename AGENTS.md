# pmacs agent instructions

**Start here: read `docs/agent-handoff.md`, then
`docs/active-work.md`, before taking on any work.** The handoff carries
durable project state, working method, substrate invariants, and the
standing backlog. The active-work ledger carries volatile branches,
checkpoints, verification, and exact cross-machine recovery commands.
Keep both updated according to their own update protocols.

Always true, independent of the handoff:

- Rust core + Lua runtime (`builtin/runtime/*.lua`), TUI + GPU
  (`pmacs-gpu`) frontends over a versioned semantic protocol
  (`pmacs-protocol`). `#![forbid(unsafe_code)]`.
- Workflow: framing doc in `docs/` -> user approval -> branch -> implement
  -> full gate suite -> PR -> user review rounds -> user says when to
  merge. Never merge unprompted. One feature, one branch, one PR.
- Gates before any PR: `cargo fmt --check`; `cargo clippy --workspace
  --all-targets -- -D warnings` (as its own step); `cargo test --lib`;
  `cargo test --lib --features crdt`; the touched acceptance suites;
  `cargo test --test m4_acceptance -- --skip basedpyright`;
  `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`; `git diff --check`.
- The checkout may be shared with the user: check `git status` for
  foreign uncommitted work before stash, checkout, or branch operations,
  and never delete untracked files you did not create.
- The canonical development URL is
  `https://github.com/levineuwirth/pmacs.git`; recovery docs normalize
  it to the local alias `githubsucks`. Remote names such as `origin` are
  machine-local and carry no authority by themselves. Bootstrap/verify
  the alias via `docs/active-work.md` before basing new work.
- Work is portable only after it is committed and pushed. Uncommitted
  worktree changes, untracked files, and `/tmp` dependencies do not
  travel to another machine.
- Write commit messages with `git commit -F <file>`. Never use
  `git add .`.
