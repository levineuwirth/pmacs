# R8 — the fixture boundary the LSP panel tests never set

**Status: revision 2 — APPROVED 2026-08-08 and IMPLEMENTED. PR #226
open (`https://github.com/levineuwirth/pmacs/pull/226`), held for
review; not merged.** The verification in §5 is no longer a plan: every
item was run, and the results are recorded in the lane in
`docs/active-work.md` and in the retired R8 row of
`docs/ci-red-signatures.md`.

**Revision 2 corrects two counts I got wrong by miscounting, adds the
portable witness revision 1 lacked, and bounds Q#R8-1's answer.** The
counts came from raw `grep -c`, which counted a function definition, a
comment, a test *name* and two assertion strings as if they were calls
— the same class of error as trusting a test-name match instead of a
signature, which this project has a whole registry about.

Test hermeticity. **Not coherence-affecting** — no journey step, no
interaction island, no config-registry setting, no background work
(`COHERENCE.md` §20). Named so the absence is a statement.

---

## 1. What is wrong, and what is not

`docs/ci-red-signatures.md` **R8** fails deterministically on this
machine:

```
assertion `left == right` failed: the flat references row renders verbatim
  left:  ".tmpPZsycN/r.rs:12:3"
  right: "/tmp/.tmpPZsycN/r.rs:12:3"
```

Diagnosed 2026-08-08:

1. `builtin/runtime/lsp.lua:2397` `display_path` shortens a location
   against the **detected project root** before rendering it.
2. `pmacs.project.detect` walks **upward** for a marker. From
   `/tmp/.tmpXXXXXX/r.rs` it reaches `/tmp`.
3. This machine has a stray **`/tmp/.git`** — an empty directory, not a
   repository. The `.git` marker is directory-only, so it matches.
4. Root resolves to `/tmp`; the prefix is stripped; the row renders as
   observed.

Control: the same test with `TMPDIR` outside `/tmp` **passes**.

**The product behaviour is correct and is not being changed.** Shortening
a location against its project root is the feature. A file that really
does sit inside a project really should render relative to it.

**The defect is that the fixture does not bound its own project
detection**, so its assertion depends on whether the developer's `/tmp`
happens to contain a `.git`. That is a hermeticity bug in the test, and
it is what this lane fixes.

## 2. The mechanism already exists, and this suite already uses it

`src/project.rs:208` documents `detect_project_within(start, markers,
stop_root)` as existing

> *"so a stray marker in a temp-dir's ancestor (e.g. a developer's
> `/tmp/.git`) can't leak into a fixture that lives below it."*

It is reachable from Lua as `pmacs.project.set_search_boundary(path)`
(`src/lua_bindings/mod.rs:12308`), and **eight test files already call
it** — fourteen real calls between them, of which
`tests/m4_acceptance.rs` holds **five**. One of those five carries the
comment *"(a developer's `/tmp/.git`, say) can't masquerade as the
root."*

(Revision 1 said nine, from a raw `grep -c` that also counted a
comment, the name of a test *about* the binding, and two assertion
message strings.)

So this is not a missing capability, a design question, or a new
pattern. **It is one helper that missed an established one**, and the
hazard it missed is documented by name in the same file.

`open_against_fake` (`tests/m4_acceptance.rs:7985`) builds an
`EditorState`, declares frame geometry, points the `rust` server at the
fake, and opens the file — and never sets a boundary.

## 3. The change

Set the search boundary to the fixture's own temporary directory inside
`open_against_fake`, before the file is opened, matching the existing
call sites.

**Three tests** call `open_against_fake` (`tests/m4_acceptance.rs:8048,
8137, 8261` — revision 1 said four, counting the definition at 7985).
All three must still pass, and the fix is expected to change the
rendering of exactly one — the one whose assertion spells the path out.

## 4. Open questions

### Q#R8-1 — the boundary's value: the file's parent, or the tempdir root?

`open_against_fake` receives a *path*, not the `TempDir`. The parent
directory of that path is the fixture root in every current caller.

*My vote: **the file's parent directory***, derived inside the helper.
It needs no signature change, and it is what the other call sites in
this file effectively use.

**Its limitation, stated now rather than discovered later.** This is
correct only while fixtures put the file as a **direct child** of the
fixture root, which all three callers do. A future nested fixture —
say `<root>/crate/src/r.rs` — that *wants* detection to reach its outer
root would not be served by this, and **passing a deeper path cannot
fix it**: the boundary is derived from the path's parent, so a deeper
path clamps the walk *sooner*, never later. That case needs an explicit
boundary argument or a second helper, and revision 1's suggestion that
a caller "can pass a deeper path" had the direction backwards.

### Q#R8-2 — is this one helper, or a census?

The suite constructs state through `EditorState::new_with_roots` **113
times**. An unknown number of those are equally unbounded, and the same
stray marker would affect any of them **whose assertion renders a
path**. Most do not, which is why only this one fails.

*My vote: **fix this helper in this lane; census separately.*** A
113-site audit is not a bug fix, and bundling it would make the
regression-fixing change unreviewable. But the census should be a named
follow-on rather than a good intention — this row cost real time twice,
and the next one will look like a new mystery.

**Recommend: file the census as a named backlog item** in
`docs/agent-handoff.md` §6 when this lands.

### Q#R8-3 — does anything depend on the unbounded walk?

If some test's expectation quietly relies on detection escaping its
fixture, bounding it would break that test — and that would be a
finding worth having rather than an obstacle.

*Expected: no.* To be established by running the suite, not asserted
here.

## 5. Verification

### 5.1 The primary witness is PORTABLE and plants its own marker

Revision 1 rested the bite on `/tmp/.git`, which makes the proof a
property of **this machine** — the same mistake as a test that passes
only where the developer happens to be standing. A hermeticity fix
whose only evidence is one machine's stray directory is not
demonstrated; it is anecdotal.

So the primary witness builds the hazard itself:

```
<tempdir>/            <- an empty `.git` is PLANTED here
  proj/               <- the file's parent; becomes the boundary
    r.rs
```

- **With the helper's boundary set to `proj`**, detection examines
  `proj`, finds no marker, and stops — the planted `.git` one level up
  is out of reach. `display_path` finds no root and falls back, so the
  rendered row is the **absolute** path.
- **Reverting the boundary** lets the walk reach `<tempdir>`, match the
  planted marker, and strip the prefix — **deterministically, on every
  machine**, with no dependence on `/tmp` or `TMPDIR`.

That pair is the bite. It runs in CI, where `/tmp/.git` does not exist,
and it fails for the right reason if the helper regresses.

*This is a new test rather than a rewrite of the existing one*: the
existing assertion's value is that it renders a real path verbatim, and
changing its layout to carry a planted marker would blur two purposes
into one fixture.

### 5.2 The machine observation stays, as confirmation only

- **The existing test passes with `/tmp/.git` still present** — on the
  machine that reproduces R8, not merely on a clean one. The stray
  directory is deliberately **not** removed: deleting it would hide the
  hermeticity defect, and it is an external directory with unresolved
  provenance.
- This is corroboration for the portable witness above, **not the bite
  itself**.

### 5.3 The rest

- **The other two `open_against_fake` tests still pass**, and the full
  `m4_acceptance` suite passes (Q#R8-3).
- **The standard gate suite**, run by hand from `docs/agent-handoff.md`
  §3.

**`scripts/gate` is deliberately NOT a criterion here.** This lane
branches from `main`, where that script **does not exist** — it is
unmerged on `gate-script` (#225). Naming it would make this lane's
verification depend on an artifact absent from its own base, and would
quietly couple two lanes that are meant to land independently.

The sequencing instead: **R8 lands first, on its own merits. Then #225
rebases onto it, and "`scripts/gate` runs green" becomes #225's
re-gate criterion** — which is where it belongs, since a green gate run
is the thing #225 ships.

**What this does not prove:** that the other 113 construction sites are
hermetic. Q#R8-2's census is why.

## 6. Not in scope

Removing or altering `/tmp/.git` — and equally, settling its provenance.
Observations of its timestamps have disagreed, `/tmp` is a tmpfs whose
entries are touched by inspection, and nothing in this lane depends on
the answer: §5.1's witness plants its own marker precisely so the fix
does not rest on that directory at all. Changing `display_path` or project
detection semantics — the product behaviour is correct. The 113-site
census (Q#R8-2). Rebasing or merging #225, which follows this landing.
