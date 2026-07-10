# pmacs — agent instructions

**Start here: read `docs/agent-handoff.md` before taking on any work.**
It carries current project state, the working method, substrate
invariants, and the standing backlog — it is the continuity bridge
between development machines. Keep it updated as part of your work
(update protocol is in the file itself).

Always true, independent of the handoff:

- Rust core + Lua runtime (`builtin/runtime/*.lua`), TUI + GPU
  (`pmacs-gpu`) frontends over a versioned semantic protocol
  (`pmacs-protocol`). `#![forbid(unsafe_code)]`.
- Workflow: framing doc in `docs/` → user approval → branch → implement
  → full gate suite → PR → user's review rounds → user says when to
  merge. Never merge unprompted. One feature, one branch, one PR.
- Gates before any PR: `cargo fmt --check`; `cargo clippy --workspace
  --all-targets -- -D warnings` (as its own step); `cargo test --lib`;
  `cargo test --lib --features crdt`; the touched acceptance suites;
  `cargo test --test m4_acceptance -- --skip basedpyright`;
  `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`; `git diff --check`.
- The checkout may be shared with the user: check `git status` for
  foreign uncommitted work before stash/checkout/branch operations, and
  never delete untracked files you didn't create.
- Commit messages via `git commit -F <file>`, ending with the Claude
  co-author line; PR bodies end with the Claude Code attribution.
