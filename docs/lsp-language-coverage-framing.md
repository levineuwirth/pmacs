# LSP language coverage: LaTeX (and the Haskell/OCaml question)

**Status: revision 2, IMPLEMENTATION AUTHORIZED 2026-08-09.**

*Recorded precisely: the user authorized dispatch after a summary of
revision 2's four corrections, rather than returning findings on the
document as they did for the other lanes. The §3 verification caveat is
therefore still live and binding — it is step zero, not a footnote.*

**Revision 2 corrects three facts revision 1 got wrong or stale, and
answers the question revision 1 named as most likely to make the entry
wrong in practice.** Haskell's server *is* installed; Slice 1 is
**smaller** than framed because the extension wiring already exists;
Q#LX3's deferral argument rests on a `COHERENCE.md` line the same
document contradicts twice; and Q#LX2 (the LaTeX root) now has a
proposal rather than a shrug.

**Revision 1 was untracked, on `main`, in one checkout.** Per the
handoff's own rule — work is portable only after it is committed and
pushed — it did not travel. That is fixed by this branch.

---

## 0. What prompted this

An audit of the host machine against `builtin/runtime/lsp.lua`. pmacs
configures LSP for fourteen languages — verified exactly, by extracting
the `pmacs.lsp.config.*` keys:

    bash  c  cmake  cpp  cuda  dockerfile  go  json  lua
    python  rust  toml  yaml  zig

**Lean is NOT among the gaps, and revision 1's first draft wrongly said
it was.** Lean 4 has `builtin/runtime/lean.lua`, `lean_abbrev.lua` and
`lean_input.lua` (all three present), an `arborium-lean` grammar, and
comment/typed-edit integration — Arc 8 Stages 1–4b, merged. The error
came from grepping `lsp.lua` alone, which is the wrong place to look
for a language that earned its own module.

## 1. The gap, re-measured

| Language | tree-sitter | LSP config | Server on this machine |
|---|---|---|---|
| LaTeX | ✅ grammar + `builtin/queries/latex/highlights.scm` | ❌ | **`texlab` 5.25.1 — installed** |
| Haskell | ❌ | ❌ | **`haskell-language-server` — INSTALLED** |
| OCaml | ❌ | ❌ | `ocaml`/`opam`/`dune` yes, `ocaml-lsp-server` **absent** |

**Correction: revision 1 said Haskell's server was missing.** Both
`haskell-language-server` and `haskell-language-server-wrapper` are on
this machine. That collapses revision 1's Slice 1 / Slice 2 split,
which rested on "Slice 2 needs servers installed first" — only OCaml
does now.

LaTeX remains the sharp case: the grammar work landed, so a `.tex`
buffer highlights correctly **and** offers no completion, no
diagnostics, no go-to-definition, while `texlab` sits on disk unused.

## 2. Ground truth — Slice 1 is smaller than revision 1 claimed

Revision 1 proposed "one `pmacs.lsp.config.latex` entry, **plus**
filetype mappings for `.tex`/`.latex`/`.sty`/`.cls`, matching the
grammar's existing extension set so highlighting and LSP agree on what
a LaTeX file is."

**The filetype mappings are redundant, and the rationale describes a
problem that cannot occur.** Three facts, read rather than assumed:

- **The grammar already carries exactly those extensions.**
  `src/syntax.rs:1110-1112` — `name: "latex"`,
  `extensions: &["tex", "latex", "sty", "cls"]`.
- **Grammar-extension detection sits AHEAD of the LSP filetype map.**
  The merged `docs/latex-grammar-math-substrate-framing.md:166-171`
  states the chain — *modeline → grammar extension → LSP filetype map →
  filename map → shebang* — and concludes that adding those extensions
  "**wires the whole chain with no Lua edit**".
- **The filetype map is explicitly a fallback.** `lsp.lua:267-270`:
  "Every language with an LSP config now also ships a grammar, so this
  is mainly the **LSP-only fallback** that keeps a language id stable if
  a grammar is ever dropped, plus the seam for user-added mappings."

So a `.tex` buffer **already** resolves to language `latex`. They
cannot disagree, because the grammar's extension list *is* what drives
detection.

**Slice 1 is therefore one thing: the `pmacs.lsp.config.latex` entry**
(plus its root resolver, §3). Filetype-map entries may still be added
as the documented drop-a-grammar fallback, but that is belt-and-braces
and should be labelled as such rather than sold as making two systems
agree.

## 3. Q#LX2 — the LaTeX project root **(answered in rev 2)**

Revision 1 called this "the question most likely to make the entry
wrong in practice" and left it open. It is the difference between
texlab serving a multi-file thesis and serving isolated files, so it is
the whole value of the lane for the stated use case.

**The mechanism exists.** `pmacs.lsp.config.<lang>.root` accepts a
string **or a resolver function**, resolved through `resolve_root_fn`
(`lsp.lua:543`) with per-resolver memoization, and on the *reuse* path
as well as the spawn path (`:535`). A configured root "MUST be a
canonical absolute path" (`:525`). So this is a config entry, not new
machinery.

*My vote: **an upward marker walk with an explicit precedence, falling
back to the file's own directory.*** In order:

1. **`.texlabroot`** — if texlab honours it (see the verification
   caveat below), an explicit user-placed marker should win over
   everything inferred.
2. **`latexmkrc` / `.latexmkrc`** — a build config is a strong,
   deliberate signal of a document root.
3. **`Tectonic.toml`** — the same for tectonic projects.
4. **The file's own directory**, as the fallback.

**Deliberately NOT in the walk: `.git`.** A repository root is the
wrong answer for LaTeX — texlab wants the *document* root, and a thesis
inside a monorepo would otherwise get the monorepo. This is the one
place where copying the other fourteen entries' instinct would be
actively wrong.

**Deliberately NOT proposed: scanning for `\documentclass`.** That is
the semantically correct notion of a root document, and it is a
directory scan on every resolve, with its own caching and invalidation
questions. If the marker walk proves insufficient in use, that is the
next increment — with evidence.

**VERIFICATION CAVEAT, stated rather than buried.** `texlab 5.25.1` is
installed and its version and CLI surface were checked directly. Its
**LSP-level** behaviour — whether it honours `.texlabroot`, and how it
resolves multi-file projects from a root URI — was **not** verified
here; the CLI exposes only `run` / `inverse-search`, so this needs a
live session. **Marker 1 is provisional and must be confirmed against a
running texlab before implementation**, exactly as the sibling
`gate-protocol-build` lane requires its precondition to be observed
rather than reasoned about. If `.texlabroot` is not a real marker, it
drops and the walk starts at `latexmkrc`.

## 4. Open questions

### Q#LX1 — does `texlab` need `settings` or `init_options`?

It pulls configuration via `workspace/configuration` under a `texlab`
section, which pmacs answers (#13). An empty section takes defaults, as
the Go entry does for gopls.

*My vote: **ship nothing.*** Build-on-save and forward-search are the
two candidates and both are opinionated; forward-search additionally
needs a configured viewer, so a default would be wrong for most
machines. Users override through the same `init.lua` seam as the other
fourteen.

### Q#LX4 — do Haskell and OCaml belong in this lane at all? *(renumbered — see below)*

With HLS installed, Haskell is now the same shape as LaTeX: one entry,
no new dependency. **But the argument against it never rested on the
dependency.** The `.hs` files here are `levineuwirth.org`'s Hakyll
generator, edited rarely; HLS is version-coupled to GHC and is a large
resident process for a language touched a few times a year.

*My vote: **LaTeX only in this lane.*** Add Haskell when there is use
evidence, which is a one-line change at that point. OCaml needs
`ocaml-lsp-server` via opam (not packaged for Arch) and is not close.

**Renumbered from Q#HS1 deliberately.** The merged
`docs/latex-grammar-math-substrate-framing.md` already uses **Q#LX2**
for a different question — its grammar vendoring source (`:83`) — so
revision 1's Q#LX2 collided with a live ID in the same language area.
This document's LaTeX questions are Q#LX1 and the root question in §3;
the language-scope question takes Q#LX4 to avoid a second collision.

### Q#LX3 — does this touch multi-root LSP affinity? — **RESOLVED, and revision 1 read a stale line**

Revision 1 called this "the one item that could argue for deferring
Slice 1", on the basis that multi-root affinity was in flight.

**It merged as PR #161.** `COHERENCE.md:124` lists it among landed
coherence work, and `:867` says "First slice landed (PR #161)". Only
`:1669` still says "first slice in flight" — and that line contradicts
the other two **within the same document**.

So the deferral argument dissolves: a LaTeX entry keyed like the
existing servers rides the convention that already landed. **The
`COHERENCE.md:1669` inconsistency is real and should be fixed**, but by
whoever next touches §20 — not smuggled into this lane.

## 5. Coherence impact (§20)

- **Journey steps touched: none.** This adds a row to an existing
  registry; no new surface, keybinding, or panel.
- **Interaction islands: none added.**
- **Config registry adoption: yes, and only that.** One entry in the
  existing `pmacs.lsp.config` table, overridable from `init.lua` by the
  same mechanism as the fourteen already there.
- **Background-work attribution (§9): unchanged, and NOT improved.**
  texlab spawns under the existing LSP supervision path with no new
  lifecycle — but it is another process that appears in `*lsp*` and
  whose requests appear in `*workers*` with nothing joining them. The
  worker-identity lane owns that; this lane neither helps nor worsens
  it.
- **§20 classification: WIRING, not model.** It surfaces machinery that
  already exists rather than adding a runtime entity — and §2 shows it
  is *more* purely wiring than revision 1 thought.

## 6. Verification

- **A `.tex` buffer attaches texlab**, witnessed end to end rather than
  by asserting the config table's contents.
- **Detection is unchanged**: `.tex`/`.latex`/`.sty`/`.cls` still
  resolve to `latex` via the grammar path (§2), asserted so that a
  later "helpful" filetype-map addition cannot be mistaken for the
  thing that made it work.
- **The root resolver returns the marker directory**, witnessed on a
  fixture with a `latexmkrc` above a `chapters/` subdirectory — the
  thesis shape, which is the case a file-directory root gets wrong.
- **It falls back to the file's own directory** with no marker present.
- **`.git` does NOT become the root** (§3) — a fixture with a
  repository above a document directory, asserting the document
  directory wins. This is the case where copying the other entries'
  instinct is wrong, so it is pinned.
- **A missing `texlab` surfaces guidance**, through the existing
  spawn-failure path (#204) — asserted, not assumed, since that path is
  what makes the failure honest.
- **Fixtures bound project detection** with
  `pmacs.project.set_search_boundary`. R8 was a fixture letting
  detection escape into the developer's environment; a LaTeX root
  fixture is exactly that hazard's shape.

**What this will NOT prove:** that texlab resolves multi-file `\input`
graphs correctly (that is texlab's job, not pmacs's), or that Haskell
and OCaml work (Q#LX4).

## 7. Not in scope

New tree-sitter grammars — Haskell and OCaml would have LSP without
highlighting, a real asymmetry that must be stated in the PR rather
than discovered by a user. Any change to Lean, which needs none.
Math/typesetting work (`#172` owns it). Any change to the LSP
spawn-failure surface (#204). Scanning for `\documentclass` to find a
root document (§3). Fixing `COHERENCE.md:1669`'s stale multi-root line
(Q#LX3) — real, but another lane's edit. Haskell and OCaml entries
(Q#LX4).
