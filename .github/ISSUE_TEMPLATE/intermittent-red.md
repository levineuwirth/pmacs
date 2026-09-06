---
name: Intermittent red
about: A test that failed once and passed on rerun, recorded so the next occurrence is recognizable
title: "intermittent: <test selector>"
labels: intermittent-red
---

One issue per mechanism. A later occurrence of the same signature is a
comment on this issue, not a new issue. A green rerun establishes
non-reproduction and nothing more; the same fragments again are a
second occurrence.

**Selector** (exact, as `cargo test` names it):

`--test <suite> <test_name>` or `--lib <module>::tests::<test_name>`

**Job or invocation** (CI job name and flavor, or the local gate stage):

**Required fragments** (normalized: no pids, no elapsed times, no
OS-error suffixes; every fragment must be present for a red to match):

- `…`
- `…`

**Log**: link to the CI job log or the gate log path, with the
`running N tests` and `test result:` lines quoted.

**Observing tree**: the commit, and whether its diff touches anything
the failing test links.

**Candidate mechanism**, stated as a candidate, or "none".

**What would settle it**: the discriminating control that has not been
run.
