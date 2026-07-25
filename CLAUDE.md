# pmacs agent instructions

**Start here: read `docs/agent-handoff.md`, then `COHERENCE.md`, then
`docs/active-work.md`, before taking on any work.** The handoff carries
durable project state, working method, substrate invariants, and the
standing backlog. `COHERENCE.md` carries the product-coherence thesis
and its audited ground truth (scorecard, per-concern gaps, priority
order) — it is the standard new work gets evaluated against, not just a
backlog item; read it before framing anything and cite the section a
framing doc serves. The active-work ledger carries volatile branches,
checkpoints, verification, and exact cross-machine recovery commands.
Keep all three updated according to their own update protocols.

Always true, independent of the handoff:

- Rust core + Lua runtime (`builtin/runtime/*.lua`), TUI + GPU
  (`pmacs-gpu`) frontends over a versioned semantic protocol
  (`pmacs-protocol`). `#![forbid(unsafe_code)]`.
- Workflow: framing doc in `docs/` -> user approval -> branch -> implement
  -> full gate suite -> PR -> user review rounds -> user says when to
  merge. Never merge unprompted. One feature, one branch, one PR. A
  framing doc for coherence-affecting work should state its coherence
  impact (journey steps touched, interaction islands added, config
  registry adoption, background-work attribution) per `COHERENCE.md`
  §20.
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
