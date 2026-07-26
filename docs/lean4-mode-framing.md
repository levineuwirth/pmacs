# Lean 4 mode — framing (Arc 8)

pmacs has no Lean support of any kind: `grep -rin lean` over `*.rs`,
`*.lua`, `*.toml`, `*.md` returns zero hits outside the words "clean",
"boolean", and "leans on". A `.lean` file today opens as a pathless-ish
plain buffer — no grammar, no major mode, no comment syntax, no pair set,
no server.

This lane closes that in nine stages. Stage boundaries are drawn where
the *substrate* changes, not where the feature list does — see §4.
§9 states the lane's coherence impact per `COHERENCE.md` §20.

## 0. Why this lane, why now

- Arc 5 (terminal), Arc 4 (themes), the config registry, and the mode
  system are all complete, and Arc 7 Stage 1 (bottom panel) merged as #155
  at `e745068`. The goal view in Stage 5 is the first real consumer of the
  panel placement API outside listview/compile/terminal, which is a useful
  forcing function for it.
- The language-support pattern is well worn and cheap: #123 (JSON/YAML),
  #144 (LaTeX), #146 (HTML+CSS). Stage 1 is that pattern almost exactly.
- Stages 2 and 4–6 are **not** that pattern, and none should be mistaken
  for a one-liner. Stage 2 changes `ensure_server`, shared by every LSP
  language. Stage 4a changes how typed-character provenance is consumed
  and Stage 4b builds the editor's first input method. Stage 5 is the
  first consumer of a non-standard LSP method family. Stage 6 adds a
  severity-routing policy to `LspServerSpec`.
- The user's stated north star is **matching or exceeding what VS Code
  does with Lean**. §5's bet 6 scores honestly how close the nine stages get
  and names precisely what is still missing.

Parallel-safety: Stage 1 touches `Cargo.toml`, `src/syntax.rs`,
`src/highlight.rs`, and four runtime Lua files. Stage 2 touches
`src/lua_bindings/mod.rs` and `builtin/runtime/lsp.lua` only. Folding
Stage 3 (the other open lane) touches `pmacs-gpu/*` and
`src/semantic_render.rs`. None of the three footprints overlap; the only
file Stage 1 shares with anything is `Cargo.toml`, at one line.

Stages 1 and 2 were independent of each other and could have run as
sibling worktrees — they shared no file. Both have since landed (#160,
#161). **Stages 3a and 3b are not independent**: 3b's subscriber is
written against the seam 3a adds, and both touch
`builtin/runtime/lsp.lua`. They are strictly sequential — recorded here,
per the #126/#127 lesson, before either starts rather than discovered
during a rebase.

## 0.1 Revision history

Revision 1 — initial. Current revision: **7**.

### Round 1 (rev 1 → rev 2)

Five findings, all revision edits — no re-scout was required. The reviewer
independently reproduced both crate teardowns, every file:line citation,
the blast-radius greps, and the Lean server facts.

1. **Acceptance 8 contradicted the change it pinned.** The negative pin
   named Lua and Python as "byte-identical" fixtures, but both are among
   the languages `constructor` retro-paints — so a fixture that didn't
   move would have been exactly the vacuous-assertion shape from the #155
   R2 lesson. Acceptance 7/8 redrawn: Lua and Python moved to the positive
   side with asserted deltas, and the negative pin now uses languages
   verified to emit none of the four names.
2. **The retro-paint is broader than rev 1 stated, and differently
   shaped.** `tree_sitter_javascript::HIGHLIGHT_QUERY` is concatenated
   into the `javascriptreact`, `typescript`, and `typescriptreact`
   entries (`src/syntax.rs:1009`–`1056`), so `constructor` reaches
   **seven** language entries, not four. More importantly the *shape* is
   not "constructors": rust/python/javascript tag **every capitalized
   identifier** (`#match? "^[A-Z]"`), and lua tags **every
   table-constructor brace**. §2.3 and Q#LN4 now state this, because it
   is what the ruling is actually about.
3. **The goal view's refresh loop had no seam.** There is no motion hook.
   Named the real mechanism (debounced polling off `process.after-tick`)
   in Q#LN13 rather than letting it grow a polling loop or new hook
   substrate unframed.

*(Round-1 findings are stated against the features, not stage numbers:
round 2 renumbered the stages, so a rev-1 "Stage 4" is now Stage 5.)*
4. **Q#LN10's ordering example didn't motivate the ordering.** `<` is not
   in the proposed pair set, so `\<>` is safe under either order. Replaced
   with the real collisions (64 abbreviation keys contain a pair-set
   character), and stated the contract that finding exposes: the
   abbreviation consumer must claim self-inserts that *extend an open
   pending abbreviation*, not only completed expansions.
5. **Q#LN8's resolver must honor the search boundary.** A Lua
   `lean-toolchain` walk that ignores `pmacs.project.search_boundary()`
   breaks the contract `detect_project_within` exists to enforce and makes
   the Stage 3b outermost-root test non-hermetic.

### Round 2 (rev 2 → rev 3) — scope expansion

Not review findings: the user pulled seven items out of §6 and into scope,
with the stated north star that **the arc should eventually match or
exceed what VS Code does with Lean**. Folded in, with two designs
corrected against ground truth the expansion request assumed differently:

- **Lake version probe** (was deferred) → Q#LN7, rewritten. **Corrected:
  there is no blocking process run in pmacs.** `pmacs.process` is
  `spawn`/`write_stdin`/`terminate`/`list`/`status`/`events_take`/
  `forget`/`resize_pty`, all drained asynchronously off
  `process.after-tick`; nothing returns output synchronously. A lazy
  probe therefore cannot gate the first attach, so the design is
  probe-plus-fallback-latch rather than probe-then-configure. Second
  correction: **`lake` being on PATH does not mean Lean works** — see
  §2.9, where the scouting machine's own `lake --version` fails.
- **Multi-root Lake scoping** (was deferred) → Q#LN15, and promoted to
  its own stage. **Corrected: `root` is computed at
  `builtin/runtime/lsp.lua:537`, *after* the reuse loop at `:529`–`:536`,
  not before it.** The fix therefore hoists the computation above the
  loop, which makes `project_root_for` run on the reuse path where it
  previously did not — a real consequence for Q#LN8's function-valued
  resolver, handled there.
- **`⦃⦄` / `⟮⟯` pairs** → folded into Q#LN6.
- **Lean in markdown fences** → Q#LN17.
- **`textDocument/waitForDiagnostics`** → Q#LN16.
- **Abbreviation table upkeep** → Q#LN11, as a documented process rather
  than a deferral.
- **`#eval` / `#check` output channel** → Q#LN18, its own stage.
- **Module hierarchy** → Q#LN19, its own stage.

Deliberately still deferred: the interactive infoview (`$/lean/rpc/*`) —
named as the arc's eventual destination, not its scope; the GPU goal band
(blocked on bottom-panel Stage 2); a `cursor.after-move` hook; `.olean` /
`.ilean`; and block-comment toggle, which the user confirmed belongs to
the comment arc's framing rather than this one.

Nits corrected in round 1: the ledger-drift note (`agent-handoff.md`
omits the panel lane rather than describing it as in-review); Q#LN12's
layering (`_request_*_raw` are the Lua bindings in
`src/lua_bindings/mod.rs`; `src/lsp.rs` has `request_hover` — Stage 5
touches both files); §2.2's chain step is
`pmacs.parse.language_from_filename`; §2.3 no longer calls
`Style::default()` entries "styled"; and bet 1's grammar count.

### Round 3 (rev 3 → rev 4)

Six findings against the round-2 expansion. All revision edits.

1. **The response half of the seam was never designed** (the real hole).
   Q#LN16 and Q#LN19 both awaited replies "through the Q#LN9 seam", and
   acceptance pinned it — but Q#LN9 defined only `on_notification` and a
   notification arm. Confirmed: **no Lua anywhere consumes
   `ev.kind == "response"`**, so a `send_request` reply is drained and
   dropped; `send_request` is effectively write-only from Lua. Q#LN9 now
   specifies both halves, including one-shot removal-before-invoke and a
   pending-response purge on server death, with acceptance mirroring the
   notification-side integrity pins.
2. **The affinity key silently fragmented loose files for every
   language.** `project_root_for`'s last fallback is `dir_of(path)`, so it
   **never returns nil for a file with a path** — a naive
   `(language_id, root)` key would give every directory of markerless
   scratch files its own server, in Python and Go and TypeScript, caused
   by a change made for Lean. Q#LN15 now rules: the affinity key is the
   root only when a root was actually *detected*, and nil for the
   fallback. Two acceptance cases pin it.
3. **An acceptance criterion was unimplementable.** `pmacs.hook` exposes
   `add` / `define` / `list` / `run` and **no `remove`**, so "leaves no
   `process.after-tick` subscription" could not be satisfied or tested.
   Reworded to the observable: after teardown, ticks issue no request and
   write nothing.
4. **Four stale cross-references survived the round-2 renumber**, despite
   that round claiming reconciliation: the opening "four stages"; §2.5
   still calling multi-root a §6 deferral; Q#LN12's "only Rust in Stages
   2–4", broken three ways; and §4's row 7 omitting Stage 7's typed
   request. Q#LN12 now carries a per-stage Rust table instead of a prose
   claim, which is harder to get wrong on the next renumber.
5. **Q#LN7's latch had no named observation mechanism.** Added: it polls
   `pmacs.lsp.list()` state on the `process.after-tick` cadence (there is
   no event for "died before initialize"), it calls `pmacs.lsp.stop`
   before spawning the fallback so `RestartPolicy` cannot respawn the
   broken command underneath it, and it updates `command`/`args` only,
   preserving user-supplied `env`/`settings`/`init_options`/`root`.
6. Wording: `\{}` expands to `{$CURSOR}`; `⦃⦄` comes from `\{{}}`.

### Round 4 (rev 4 → rev 5) — Stage 3 re-scout and split

Stages 1 and 2 landed (#160, #161). Re-scouting Stage 3 against `main`
@ `46a1b8f` — six merged PRs past the rev-4 snapshot (#159–#164) —
produced three findings that change the plan and four that confirm it.
Every fact below was verified in a worktree at that commit; the two
marked *probed* were established by running Lua in a fresh
`EditorState`, not by grep.

1. **Stage 3 violated this document's own splitting rule.** §4 says "no
   PR in this arc mixes a cross-cutting substrate change with Lean
   feature content" and "a reviewer looking at Stage 3 sees only Lean" —
   while §4's own risk column for Stage 3 read *"two `lsp.lua`
   generalizations."* Those cannot both be true. One of the two landed
   as Stage 2; the other is Q#LN9's dispatch seams, which modify
   `handle_server_requests` — confirmed the **only** production drain of
   LSP events (`LspManager::take_all_events` has no non-test caller). By
   the same test that justified splitting Stage 2 out, that is
   cross-cutting substrate. **Stage 3 is now 3a (substrate, no Lean) and
   3b (Lean).**
2. **The Lean resolver could not satisfy the contract Stage 2
   documented.** #161 established that a configured root — string or
   resolver return — must be a canonical absolute path, because it
   reaches `file_uri_for` verbatim and that URI is the affinity key.
   *Probed:* `pmacs.editor.file_path()` is **not** canonical. Opening
   `<tmp>/linkpkg/sub/./../sub/a.lean`, where `linkpkg` symlinks to
   `pkg`, yields `<tmp>/linkpkg/sub/a.lean` — lexical `.`/`..` collapse
   only, symlinks unresolved. No canonicalize binding is exposed to Lua,
   and `pmacs.project.detect` canonicalizes but returns nil without a
   marker. So a Lean resolver walking up from the buffer's path returns
   a non-canonical root, and one package opened by two spellings spawns
   two `lake serve` processes — reintroducing precisely the bug Stage 2
   exists to prevent. New Q#LN20 adds `pmacs.fs.canonicalize`; it rides
   3a because it is substrate, and it retires the footgun for every
   future function-valued root rather than only Lean's.
3. **`pmacs.fs.stat` is unusable in the resolver.** It is asynchronous —
   `fs.lua:93` returns an awaitable handle — and the resolver runs
   synchronously inside `ensure_server` ← `attach_buffer` ← the
   `buffer.after-load` hook, where there is no coroutine to await on.
   *Probed:* the `io` and `os` stdlib **are** exposed in the sandbox
   (`type(io.open) == "function"`; `terminal.lua` already uses
   `os.getenv`), and `io.open` returns nil for a missing path. So the
   marker walk is implementable, but through the Lua stdlib rather than
   the pmacs fs API — the opposite of what a reader would assume.
   Q#LN8 now says so, with the one edge that matters: `io.open`
   **succeeds on a directory**, so a bare existence check would accept
   a `lean-toolchain` *directory* as a marker.

Confirmations, recorded because each was load-bearing and unverified:

4. **Q#LN7's "stop the failing server first" is necessary, not
   defensive.** The spec default is `LspRestartPolicy::OnCrash`, and the
   termination handler calls `should_restart(policy)`
   (`matches!(OnCrash | Always)`) — which, unlike the
   `termination_warrants_restart` helper beside it, never consults the
   exit code. `maybe_restart` re-fires on every elapsed backoff with **no
   attempt ceiling**, so a broken `lake` respawns forever. `stop()` sets
   `restart = Never` (`src/lsp.rs:1349`), which is exactly what disarms
   it. Acceptance 36 pins a real mechanism.
5. **The response seam works as designed.** `Response` events are pushed
   unconditionally (`src/lsp.rs:2652`) — the typed-store absorb above
   does not consume them — and reach Lua as `{kind = "response",
   request_id = <number>, method, result, error}`, with
   `pmacs.lsp.send_request` returning that same numeric id. So
   `on_response(sid, request_id, fn)` is keyable as specified.
6. **The seams' contract is narrower than rev 4 implied, and the
   narrowing is load-bearing.** `handle_server_requests` builds its sid
   list from `attachments`, and `push_event` appends with no cap. So a
   subscriber fires only for a server with a live attachment, and an
   unattached server's event queue grows unboundedly.

   *Corrected during implementation (rev 5, round 2).* Rev 5 first
   claimed the reachable leak was a killed buffer. **That was wrong.**
   The Rust core fires exactly five hooks — `buffer.after-edit`,
   `buffer.after-load`, `buffer.after-switch`, `frontend.detached`,
   `process.after-tick` — and **there is no buffer-kill hook at all**,
   so `lsp.lua` never tears an attachment down and the drain keeps
   reaching that server. The premise was right and the inference was
   not: it needed attachments to be removed on kill, and nothing
   removes them.

   The reachable leak is a different path with the same root cause.
   `attach_buffer` drops a sid from `attachments` the moment
   `server_is_live` reports false, rebuilding against a fresh server —
   so the `crashed` / `stopped` event that should trigger a purge is
   **precisely the one most likely to go undrained**. An event-driven
   purge leaks exactly when it matters. Q#LN9 therefore drives the
   purge off `pmacs.lsp.list()`, which enumerates the manager directly
   and is unaffected by attachment bookkeeping.
7. **The `cfg.restart` gap is still open** (recorded landing #161):
   `ensure_server` never forwards `pmacs.lsp.config[lang].restart` to
   `pmacs.lsp.spawn`, so the field is silently dropped on auto-attach.
   Stage 3b is the first stage that would benefit from setting it, and
   Q#LN7 now records why it deliberately does not need it.

Citation drift repaired per COHERENCE §25. Round 4's first pass stated
the `project_root_for` correction in this section without editing the
citation in §2.5 — the correction and the fix are different acts, and
noting one is not doing the other. Review caught a second stale citation
(`handle_server_requests`), which prompted a full sweep of every
`file:line` from §2.4 onward; it found four more. All six:
`project_root_for` 513 → **592** (and it now returns `root, source`
rather than a bare root), `ensure_server` 527 → **610**,
`handle_server_requests` 1448 → **1549**, `take_typed_edit`
12798 → **12827**, `pair.lua` 213 → **229**, and `compile.lua`
264 → **266**. Verified good and left alone: `listview.lua:138`,
`src/lsp.rs:264`, `src/diag.rs:50`, `src/process.rs:193`,
`src/project.rs:145`, and the `mod.rs` binding-block citations. The
pre-#161 line numbers inside Q#LN15 are left as written: that stage has
landed and its citations are historical record, not navigation.

### Round 5 (rev 5 → rev 6) — Stage 4 re-scout and split

Stages 3a and 3b landed (#167, #170). Re-scouting Stage 4 against `main`
@ `d400f30` produced **six findings that change the plan** and three
that confirm it. (Round 6 found five more, four of them internal to this
revision; read that section too before trusting a rev-6 statement.) The pmacs-side facts were verified in a worktree at
that commit; the upstream facts were verified by downloading and reading
`leanprover/vscode-lean4` at commit `17d1d08` (2026-05-29) — the
algorithm, not its documentation, since the `lean4-unicode-input`
package ships no README. *(Round 6: it does, at `src/README.md` — see
that section. Corrections to round 5's own numbers are marked inline
below rather than rewritten, per the standing rule that revision
entries are record, not navigation.)*

1. **Stage 4 violated this document's own splitting rule — the same way
   Stage 3 did.** §4 says "no PR in this arc mixes a cross-cutting
   substrate change with Lean feature content," and §4's own risk column
   for Stage 4 read *"refactors `pair.lua`'s provenance read."*
   `pair.lua` is every language's auto-pairing; the refactor is
   cross-cutting substrate by exactly the test that split out stages 2
   and 3a. Rev 5 already conceded the shape without acting on it —
   Q#LN10 said the refactor "lands *first*, as its own commit with no
   behavior change, so a regression bisects cleanly." A commit boundary
   is not a review boundary. **Stage 4 is now 4a (the typed-edit
   consumer chain, no Lean) and 4b (the input method).** Confirmed
   `pair.lua:226` is still the **only** production `take_typed_edit`
   caller; the other eight call sites are all in
   `tests/auto_pair_acceptance.rs`.
2. **The expansion semantics in rev 5's Q#LN10 were wrong in three
   ways.** Reading `AbbreviationProvider.ts` and `TrackedAbbreviation.ts`
   rather than inferring from behavior:
   - Rev 5 said expansion fires on "a unique complete match that no
     longer key extends." Upstream's rule is
     `findSymbolsByAbbreviationPrefix(abbrev)[0]` — the symbol of the
     **shortest key having `abbrev` as a prefix**. `\alp` + space is not
     a failure; it yields `α`, because `alpha` is the shortest key
     starting with `alp`. Verified against the table: `\al` → `∀`, from
     `all`, not from `alpha`.
   - Rev 5 named "an explicit terminator (space, tab, RET, or a second
     `\`)." **There is no terminator list upstream.** A character
     terminates iff extending the pending key by it leaves zero prefix
     matches. Space usually does — but `'+ '` **is a key** (one of
     1,855), so after `\+` a space extends rather than terminates. And a
     second `\` is not a terminator either: `'\'` is a key mapping to
     `\`, so `\\` extends, matches uniquely, and expands to a single
     backslash. It terminates only when the pending key is non-empty and
     no key extends it.
   - Rev 5 did not carry the suffix rule at all. When no key has
     `abbrev` as a prefix, upstream recurses on `abbrev` minus its last
     character and **appends the leftover**: `\alp7` → `α7`. Dropping
     this makes a large class of real input silently unexpandable.
3. **There is no cursor-motion hook, so acceptance 43 as written cannot
   be built.** The Rust core fires exactly eight named hooks
   (`builtin/hooks/default.lua`): `buffer.before-save`,
   `buffer.after-load`, `buffer.after-edit`, `buffer.after-switch`,
   `buffer.after-save`, `editor.before-quit`, `frontend.detached`,
   `process.after-tick`. Upstream drives abandonment off
   `changeSelections`, a seam pmacs does not have. Abandonment must
   therefore be **lazy** — validated at the next typed edit against the
   pending region — which changes what acceptance 43 can assert. Q#LN22
   states the state machine this forces.
4. **`dispatch_key` is only half of Stage 4b's production path.** Rev 5
   inherited the auto-pairing suite's dispatch-driven harness without
   noticing why that harness is sufficient *there*: Q#AP1 removed the
   pair characters from both optimistic classifiers, so for pair chars
   dispatch **is** production. `\` and the ASCII letters are not
   excluded — `classify_key` returns `Insert(c)` for them
   (`src/optimistic.rs:144`: `Char(c) if !c.is_control() &&
   !is_builtin_pair_char(c)`), so on a CRDT frontend an abbreviation is
   typed entirely through the *optimistic* producer, which arms the same
   record from `handle_remote_crdt_op` (`src/daemon.rs:3965` pins the
   classification). A dispatch-only Stage 4b suite would pin the path
   real users do not take. The trap underneath: that producer is
   `#[cfg(feature = "crdt")]`, and CI never enables `crdt` — so a
   crdt-gated integration test is dark twice over, since the required
   gate list runs `--features crdt` only for `--lib`. Q#LN22 and §7 say
   what to do about it instead of discovering it in review.
5. **The whole expansion has cross-peer-degraded undo, and it is a
   larger bite than `⟨⟩`'s.** Q#LN6 already accepts this for three
   bracket pairs. But there the mismatch is one optimistic opener
   against one daemon-peer closer; here the user's `\alpha` is six
   source-peer optimistic inserts and the expansion is a single
   daemon-peer `replace` **over all six**. Q#LN21 takes the decision —
   including why `pmacs.buffer.set_round_trip_input`, which already
   exists and would fix it, is the wrong instrument.
6. **The table's shape is sharper than "1,855 entries."** Re-counted at
   `17d1d08`: 1,855 entries, all `string → string`, **all keys ASCII**,
   longest key 25 characters, 36,861 bytes of JSON. **64** keys contain
   a `lean4` pair-set character (rev 5's number, reproduced exactly).
   Three numbers rev 5 did not have and the algorithm needs: **305**
   keys are proper prefixes of another key (so 1,550 are eager-expandable
   on uniqueness and 305 are not), **26** values carry `$CURSOR` (not
   just `\<>`), and **93** values are multi-codepoint. Two values contain
   a backslash — `n` → `\n` and `setminus` → `\` — which is why upstream
   needs a `doNotTrackNewAbbr` guard and why §2.11 records that pmacs
   does not.

   *Corrected in round 6.* **119** symbols are multi-codepoint, of which
   26 carry `$CURSOR`; "93" was the non-`$CURSOR` subset stated as a
   total. **Three** values contain a backslash — the `\` → `\` identity
   entry was missed. And this entry's biggest omission is not a number:
   the shortest-key rule needs a **tie-break by source declaration
   order**, which the README round 5 said did not exist states outright.
   §2.11 and Q#LN11 carry the corrected facts.

Confirmations, recorded because each was load-bearing and unverified:

7. **`take_typed_edit`'s one-shot contract is unchanged**
   (`src/editor_core.rs:4047`): per-frontend, cleared by the producer
   when the fan-out returns, nil to a nested manual `hook.run`. The
   hazard rev 5 built Q#LN10 around is real and still the reason 4a
   exists.
8. **Load order still constrains the chain.** `pair.lua` loads at
   `src/editor.rs:430` and `lsp.lua` at `:436`, and Q#AP7's reason
   holds: `lsp.lua`'s `buffer.after-edit` callback synchronously flushes
   `didChange` on the signature-trigger path. An expansion that landed
   after that flush would send the server the unexpanded text.
9. **Embedding the table needs no special machinery.** Every builtin
   runtime chunk is an `include_str!`, and `lsp.lua` is already 111 KB
   of the 414 KB total. A ~45 KB generated Lua table is within the
   existing practice, so Q#LN11 embeds it rather than inventing a
   lazy-load path.

Citation drift repaired per COHERENCE §25, on the same terms as round
4's sweep. Five live citations moved in the 50 commits since rev 5:
`take_typed_edit` 12827 → **12990**, `handle_server_requests` 1549 →
**1815**, `fs.stat` 93 → **133**, `detect_buffer_language` 452 →
**457**, and `send_request`/`send_notification` 9342/9361 →
**9507**/**9527**. Left as written: the pre-#161 numbers inside Q#LN15
and the revision-history entries above, which are historical record
rather than navigation.

### Round 6 (rev 6 → rev 7)

The 4a/4b split held; five P1s against the revision's own content, all
real, all reproduced. Four share a root: **rev 6 verified its external
facts and under-verified its internal ones.**

1. **Stage 4a's declared footprint excluded the tests its acceptance
   required.** Q#LN10 listed three production files while 46a–46e demand
   chain-specific tests that cannot live in
   `tests/auto_pair_acceptance.rs` — criterion 46 requires that file
   byte-identical. Footprint now names
   `tests/typed_edit_chain_acceptance.rs` and adds it to the PR's gates.
2. **Pending state had the wrong owner.** §2.11 reasoned "no
   multi-cursor, therefore one point" and Q#LN22 keyed pending
   abbreviations by buffer. pmacs is multi-frontend: `EditorCore.views`
   is per-`FrontendId` with its own active window, `take_typed_edit` is
   *already* frontend-keyed, and `pmacs.frontend.id()` exists. Two
   frontends on one Lean buffer — the TUI-plus-GPU case this project
   ships — would share one slot. Worse, `buffer.after-switch` takes no
   arguments, so a buffer-keyed clear-on-switch lets any frontend
   discard another's pending abbreviation. Now keyed
   `(frontend, buffer)` with a window check, frontend-scoped
   after-switch clearing, a `frontend.detached` purge, and acceptance
   45i — which the buffer-keyed design passes every other criterion
   without.
3. **The shortest-match rule was missing its tie-break, and rev 6's
   research method is why.** Upstream keeps declaration order among
   equal-length shortest keys. The README states it in one sentence —
   and rev 6 asserted "the package ships no README" after a 404 on the
   package root, without checking the directory listing it had already
   fetched, which shows `README.md` under `src/`. **A 404 on a guessed
   path is not evidence of absence.** The rule is load-bearing: 101
   prefixes have equal-shortest candidates resolving to *different*
   symbols (`f` → `f<` not `f>`; `"` picks `"A` from eleven). A `pairs`-
   iterated Lua map cannot express it, so Q#LN11 now emits an ordered
   sequence and Q#LN22 sorts by `(#key, source rank)`.
4. **The generator's rejection rule rejected the current table.** "Abort
   on keys needing Lua escaping" would reject `\` and the eleven `"X`
   keys — and acceptance 45d requires `\` to work. Replaced with
   canonical lossless escaping; the generator aborts only on duplicate
   keys, invalid UTF-8, and a failed self-round-trip. Relatedly, 45g
   claimed the suite compares against `abbreviations.json`, which is not
   shipped; it now pins self-consistency properties and leaves
   source fidelity to the generator, where the source is in hand.
5. **Durable and volatile state were not reconciled** —
   `docs/agent-handoff.md` still anchored `main` at `d152120` with
   neither #167 nor #170, while `docs/active-work.md` kept full merged
   Stage 3a/3b histories against its own instruction to remove merged
   entries, under a stale July 25 snapshot date. Round 5 updated the
   ledger and skipped the handoff; per CLAUDE.md both are required
   reading, and the one that outranks the other was the one left wrong.

Corrections carried in the same revision, each verified against the
data: the README exists (finding 3); there are **119** multi-codepoint
symbols, of which 26 carry `$CURSOR` — rev 6's "93" was the
non-`$CURSOR` subset reported as a total; **three** values contain a
backslash (`\`, `n`, `setminus`), not two; Q#LN22 now states the rule
acceptance 45d depended on, that an unclaimed terminating `\` is
reprocessed as a new leader; acceptance 38 now says the terminator is
retained, so undo restores `\alpha ` with its space; the coherence
section cites golden-journey **step 5** ("Edit immediately"), not step
4; and §8's config-registry prior art points at Q#LN22, where the gate
now lives.

## 1. What ships

Nine stages, after round 4 split Stage 3 and round 5 split Stage 4. The
north star is VS Code parity; the honest statement of where that lands
is in §5, bet 6.

**Stage 1 — grammar, mode, and the editing table stakes.** `.lean` files
highlight, carry a `lean4` major mode, and get comment-toggle and
auto-pairing (including `⟨⟩`, `⦃⦄`, `⟮⟯`). Lean fenced blocks in markdown
highlight too. No LSP, no protocol change, no frontend change.

**Stage 2 — multi-root LSP server affinity.** Pure substrate, no Lean
content: `ensure_server` stops reusing a server across project roots.
Independently valuable for every language pmacs supports; a prerequisite
for Lean being usable across more than one Lake package. Split out
precisely *because* it is cross-cutting — see §4.

**Stage 3a — LSP dispatch seams and a path canonicalizer.** Pure
substrate, no Lean content, split from Stage 3 in round 4 for the reason
Stage 2 was: it changes machinery every language runs through.
`handle_server_requests` gains notification and response arms with a
pending-response purge, so a `send_request` reply is no longer drained
and dropped; `pmacs.fs.canonicalize` gives Lua the one primitive a
function-valued `config.root` needs to honor the canonical-path contract
#161 could only document.

**Stage 3b — the Lean language server.** `pmacs.lsp.config.lean4` drives
`lake serve` with a Lake-aware outermost root, a lazy toolchain probe and
a one-shot `lean --server` fallback, and subscribes `$/lean/fileProgress`
on 3a's seam. Adds `textDocument/waitForDiagnostics`. Diagnostics, hover,
completion, goto-definition, document symbols, and semantic tokens all
arrive through the existing typed surfaces.

**Stage 4a — the typed-edit consumer chain.** Pure substrate, no Lean
content, split from Stage 4 in round 5 for the reason stages 2 and 3a
were: it changes machinery every language runs through. The one-shot
`take_typed_edit()` record stops being auto-pairing's private property
and becomes a small ordered chain that reads it once and offers it to
registered consumers. `pair.lua` becomes the chain's first and only
consumer, with no behavior change.

**Stage 4b — the Unicode input method.** Typing `\alpha` produces `α`,
`\to` produces `→`, `\<>` produces `⟨⟩` with the point between them.
1,855 abbreviations vendored from vscode-lean4, registered as a chain
consumer ahead of auto-pairing. This is the stage that makes Lean
actually typable in pmacs.

**Stage 5 — the goal view.** A `*lean-goal*` panel that renders
`$/lean/plainGoal` at the point, refreshed on a debounced tick and on
file-progress completion, displayed through #155's
`pmacs.window.display(buf, { side = "bottom" })`.

**Stage 6 — the `#eval` / `#check` output channel.** Lean reports command
output as *information*-severity diagnostics, which pmacs currently
squiggles and counts in the modeline. Routes them to a `*lean-output*`
panel instead, via a per-server severity policy that changes nothing for
any other language.

**Stage 7 — module hierarchy.** `$/lean/prepareModuleHierarchy` and
`$/lean/moduleHierarchy/{imports,importedBy}` into the existing listview
panel.

## 2. Ground truth (scouted 2026-07-24, `main` @ `e745068`)

Stage 3's facts were **re-verified 2026-07-25 against `main` @
`46a1b8f`**, six merged PRs later; what changed is recorded in §0.1's
round 4 rather than rewritten in place, so a reader can see which
claims moved. Facts for stages 4–7 still carry the 2026-07-24 date and
should be re-scouted before those stages are framed for
implementation.

### 2.1 Crate facts (external, verified by downloading and reading both)

Two candidate grammar crates exist. They are not close in quality.

**`tree-sitter-lean4` 0.3.0** (`wvhulle/tree-sitter-lean`) — **rejected**:

- Depends on `tree-sitter = "0.25"` **directly**, not on the shared
  `tree-sitter-language 0.1` ABI crate. The workspace is on
  `tree-sitter = "0.26"`, and `^0.25` excludes it, so this forks the graph
  and its `Language` is a different type from ours. This is the exact
  failure mode already documented for `tree-sitter-dockerfile` in
  `Cargo.toml`'s comments.
- `src/lib.rs` exports only `pub fn language() -> Language`. Its README
  advertises `tree_sitter_lean4::LANGUAGE.into()`, which **does not
  exist** — the README is stale.
- Its `include` list is `["build.rs", "src/*", "grammar.js", "grammar/*",
  "tree-sitter.json"]`. **No `queries/`.** It ships no highlights query at
  all; the upstream repo's queries target Helix.
- Its `build.rs` shells out to a `tree-sitter` CLI when `src/parser.c` is
  absent. `parser.c` *is* in the package, so this would not fire — but it
  is a live hazard in a crate we would otherwise depend on.

**`arborium-lean` 2.18.1** (`bearcove/arborium`) — **selected**:

- `[dependencies] tree-sitter-language = "0.1"` and nothing else at
  runtime. No second `tree-sitter` in the graph.
- `grammar/src/parser.c` declares `#define LANGUAGE_VERSION 15` and
  `.abi_version = LANGUAGE_VERSION`. ABI 15 is current for tree-sitter
  0.25/0.26. Pre-generated; no CLI at build time. A 1,150-byte
  `grammar/scanner.c` supplies one external token (`NEWLINE`).
- Exports `pub const fn language() -> LanguageFn`, plus
  `HIGHLIGHTS_QUERY` (`include_str!("../queries/highlights.scm")`, 213
  lines), `INJECTIONS_QUERY` (empty string), and `LOCALS_QUERY` (empty
  string).
- `edition = "2024"`, `rust-version = "1.85"`. The workspace is edition
  2024 / MSRV 1.95 on rustc 1.95.0. Compatible.
- **Two things to verify at implementation, not assumed here.** (a) Every
  existing entry in `BUILTIN_LANGUAGES` is spelled
  `tree_sitter_foo::LANGUAGE.into()` — a `LanguageFn` const. arborium
  exposes a `const fn` instead, so the entry reads
  `arborium_lean::language().into()`, a shape no current entry uses.
  (b) arborium's README shows usage against a
  `tree_sitter_patched_arborium` crate. That crate is *not* in the
  dependency graph and the `LanguageFn` ABI is the shared one, so this
  should be cosmetic — but the loader gets a real parse smoke test against
  `tree-sitter 0.26` before the entry is trusted.

Neither crate is first-party. `leanprover` ships no tree-sitter grammar;
Lean's own tooling parses with the Lean kernel. The upstream README of the
rejected crate says so plainly: *"Lean is a very extensible language.
Therefore, the Tree-Sitter grammar is of limited use."* That is true and it
bounds what Stage 1 can promise — see the bets in §5.

### 2.2 The grammar table and the detection chain

`src/syntax.rs:816` `BUILTIN_LANGUAGES` is a `&[LanguageEntry]` of
`{ name, extensions, loader, highlights_query, locals_query,
injections_query }`. Adding a grammar is one entry plus one `Cargo.toml`
line; the doc comment at `src/syntax.rs:756` says exactly this and it has
held for every grammar since.

`builtin/runtime/syntax.lua:457` `detect_buffer_language` resolves, in
order: modeline → `pmacs.parse.language_for_path` (the grammar extension
table) → `pmacs.lsp.filetypes[ext]` → `pmacs.parse.language_from_filename`
→ shebang. A grammar entry claiming `lean` therefore resolves `.lean`
without any `pmacs.lsp.filetypes` entry; adding one would be dead weight.

**`LanguageEntry.name` is the LSP `language_id`.** `ensure_server` at
`builtin/runtime/lsp.lua:540` passes `language_id = language` straight
into `pmacs.lsp.spawn`, and the surrounding comments (lines 70, 86, 122)
record that `c`/`cpp` and the four TS/JS entries exist as separate entries
*only* so that id is accurate. This makes the entry name a wire-visible
decision, not a label — see Q#LN2.

### 2.3 The global capture table (the #146 trap)

`Theme::default_dark()` at `src/highlight.rs:143` is a single flat
`&[(&str, Style)]` shared by **every** language and by LSP semantic-token
type names. `lookup()` walks dotted prefixes right-to-left, so
`@function.definition` falls back to `function`.

Resolving arborium's Lean query against the current table:

| Lean capture | Resolves to | Effect |
|---|---|---|
| `@comment` `@string` `@number` `@operator` `@constant` | themselves | distinct style |
| `@constant.builtin` `@property` `@attribute` | themselves | distinct style |
| `@function.definition` `@function.call` `@function.builtin` | `function` | distinct style |
| `@type.definition` | `type` | distinct style |
| `@string.special` | `string` | distinct style |
| `@keyword.conditional` `.function` `.import` `.modifier` | `keyword` | styled, but flattened |
| `@variable` | `variable` | **entry exists but is `Style::default()`** — visually plain |
| `@punctuation.special` `.bracket` `.delimiter` | `punctuation` | **`Style::default()`** — visually plain |
| `@constructor` | — | **unstyled** |
| `@character` | — | **unstyled** |
| `@warning` | — | **unstyled** |

Blast radius of adding each name, measured over every `.scm` in the
workspace's actual dependency graph (crates confirmed present in
`Cargo.lock`):

- **`constructor` — seven language entries, not four grammars.** The
  emitting crates are `tree-sitter-javascript`, `-lua`, `-python`, and
  `-rust`, but `tree_sitter_javascript::HIGHLIGHT_QUERY` is concatenated
  base-first into `javascriptreact`, `typescript`, and `typescriptreact`
  as well (`src/syntax.rs:1009`–`1056`), so the reachable set is
  `rust`, `lua`, `python`, `javascript`, `javascriptreact`, `typescript`,
  `typescriptreact`.

  **And the shape is not "constructors".** Verified against the crate
  queries:

  ```scheme
  ; rust, python, javascript — every capitalized identifier
  ((identifier) @constructor (#match? @constructor "^[A-Z]"))

  ; lua — every table-constructor brace
  (table_constructor [ "{" "}" ] @constructor)
  ```

  So adding `constructor` recolors **every capitalized identifier** in
  five entries (in Rust that is `Some`/`None`/`Ok`/`Err` and every
  class-cased name; in Python/JS/TS every class-cased name) and **every
  `{`/`}` of a Lua table literal.** That is the change being ruled on in
  Q#LN4 — not a narrow constructor-only recolor.
- `character` — `tree-sitter-zig` only.
- `keyword.conditional` — `tree-sitter-cmake` and `-zig` only. Both
  currently flatten to `keyword`.
- `warning` — used by **no** grammar in the graph.

Verified clean of all four names (usable as negative-pin fixtures):
markdown, json, yaml, html, css, c, cpp, go, containerfile, make, toml,
bash.

This is the #146 lesson verbatim: *the capture table is global, so adding a
capture name retro-paints every other language; check the reverse direction
and pin it.*

### 2.4 The LSP substrate

- `pmacs.lsp` already exposes generic `send_request(id, method, params)`
  → request id and `send_notification(id, method, params)`
  (`src/lua_bindings/mod.rs:9507`, `:9527`). Non-standard methods need no
  new Rust to *send*.
- `LspEventKind` (`src/lsp.rs:264`) has generic `Notification { method,
  params }` and `Response { id, result, error, method }` variants. Unknown
  server methods are delivered, not dropped.
- **But `events_take` has exactly one consumer**: `handle_server_requests`
  at `builtin/runtime/lsp.lua:1815`, driven off `pmacs._async.tick`. It
  `take`s — a drain. Its `if/elseif` chain handles five `request` methods
  and `initialized`, and **ignores every `notification` and every
  `response`**. A second module calling `events_take` would steal events
  from it. Any new consumer must extend that loop, not open a second one.
- **`_request_*_raw` helpers route outbound positions through
  `outbound_position` (byte → negotiated encoding); a raw `send_request`
  does not.** This is handoff §4's standing invariant, and it is sharper
  for Lean than for any language pmacs already supports: Lean source is
  saturated with non-ASCII (`α`, `→`, `⟨⟩`, `∀`), the Lean server
  negotiates UTF-16 by default, and a raw byte column is wrong on
  essentially every interesting line. This is why Stage 5 is not
  "just call `send_request` from Lua" — see Q#LN12.

### 2.5 Project-root detection

`project_root_for` (`builtin/runtime/lsp.lua:592`) resolves:
`pmacs.lsp.config[language].root` → `pmacs.project.detect` → the file's own
directory. Two gaps for Lean:

1. **No Lean marker, and no way to add one.** `default_markers()`
   (`src/project.rs:145`) is `Cargo.toml`, `.luarc.json`, `pyproject.toml`,
   `go.mod`, `deno.json`, `deno.jsonc`, `package.json`, `.git`. The
   `pmacs.project` Lua surface is `detect`, `set_search_boundary`,
   `search_boundary` — **there is no marker-registration binding.**
2. **`detect_project` returns the innermost match; Lean needs the
   outermost.** `walk_for_marker` returns the first ancestor that matches.
   `lean4-mode` deliberately does the opposite:

   ```elisp
   (while-let ((dir (locate-dominating-file file-name "lean-toolchain")))
     (setq root dir
           file-name (file-name-directory (directory-file-name dir))))
   ```

   It keeps walking *past* each hit and takes the topmost. This matters
   concretely: Lake vendors dependencies under `<pkg>/.lake/packages/*`,
   and each vendored package carries its own `lean-toolchain`. Opening
   `<pkg>/.lake/packages/batteries/Batteries/Data/List.lean` must serve
   from `<pkg>`, not from `batteries`. Innermost-wins gets this backwards
   every time, and the symptom is a server that starts, initializes, and
   then reports import errors for the whole file.

Third, and the reason Stage 2 exists: `ensure_server`
(`builtin/runtime/lsp.lua:610`) reuses any live server with a matching
`language_id` regardless of the new file's project, so **the first `.lean`
file opened fixes the root for every later `.lean` file.** For most
languages that is an inconvenience; for Lean, where `lake serve` is bound
to one package, it is a correctness failure. Rev 1 carried this as a
deferral. **It is now Stage 2 / Q#LN15.**

One property of `project_root_for` matters for that fix and is easy to
miss: its final fallback is `dir_of(path)`, so **it never returns nil for
a file with a path.** A markerless scratch file's "root" is its own
directory. Q#LN15's affinity rule has to account for that or it silently
changes loose-file behavior for every language.

### 2.6 Typed-edit provenance — the only input-method-shaped seam

`builtin/runtime/pair.lua` is the whole precedent for "react to a typed
character": subscribe to `buffer.after-edit`, gate on
`ed.this_command() == "buffer.self-insert"` (`pair.lua:229`), then take the
exact provenance record.

`pmacs.editor.take_typed_edit()` (`src/lua_bindings/mod.rs:12990`) returns
`{ buffer, window, codepoint, char, requested_start, requested_end,
effective_start, effective_end, inserted_len, post_cursor, clean }` — or
nil. Its doc comment is explicit:

> Consuming clears the slot: later callbacks and nested manual hook runs
> see nil, and the producer clears any untaken record when the fan-out
> returns. Per-frontend — one frontend can never take another's record.

**It is one-shot and first-come-first-served.** `pair.lua` already consumes
it on every self-insert. A Lean abbreviation expander that independently
calls `take_typed_edit()` in the same `buffer.after-edit` fan-out gets nil
or steals it from auto-pairing, depending on hook order — and hook order is
not a contract. This is the single load-bearing constraint on Stage 4 and
the reason Stage 4 is its own PR rather than a rider on Stage 1 — and,
after round 5, the reason its substrate half is Stage 4a rather than a
first commit on a Lean branch.

Re-verified at `d400f30`: `pair.lua:226` remains the **only** production
caller. The eight other call sites in the tree are all in
`tests/auto_pair_acceptance.rs`. So the chain Stage 4a introduces has
exactly one consumer to migrate, which is what makes a no-behavior-change
substrate PR possible at all.

Two producers arm the record, not one, and §2.11 is where that matters:
the dispatch fallback and — under `#[cfg(feature = "crdt")]` — the
optimistic CRDT arm reached from `handle_remote_crdt_op`.

Related, from `pair.lua:30`'s Q#AP1 note: only the nine built-in pair chars
`()[]{}"'` and backtick are excluded from the frontends' optimistic
classifiers. A pair char outside that set still pairs, but its opener is a
source-peer op and its closer a daemon-peer op, so **its undo is
cross-peer-degraded**. Lean's `⟨⟩` is outside that set.

### 2.7 Panels and generated read-only buffers

- #155 landed on `main` at `e745068`. `pmacs.window.display(buf, { side =
  "bottom", select = true })` is the placement call; `listview.lua:138`
  shows the adopter shape, gated on `spec.display == "panel"`.
  `pmacs.window.params()` and `pmacs.window.quit()` complete the surface.
- Read-only generated buffers use the listview idiom, documented at
  `builtin/runtime/compile.lua:266`: an erroring `pmacs.buffer.add_intercept`
  for user edits, with module writes passing `{ bypass_intercept = true }`.
- **Note for whoever picks this up on another machine:** the ledgers are
  stale about this. `docs/active-work.md:57` still heads the lane "Stage 1
  IN REVIEW"; `docs/agent-handoff.md` §1 is stamped at #148 and **omits
  the bottom-panel lane entirely**. It merged at `e745068`. That drift is
  #156's business, not this lane's, but do not scout Stage 5 off either
  file.

### 2.8 Lean server facts (external, verified against `leanprover/lean4`)

From `src/Lean/Server/FileWorker/RequestHandling.lean` and
`src/Lean/Data/Lsp/Extra.lean`:

- `$/lean/plainGoal` — params extend `TextDocumentPositionParams`; result
  is `PlainGoal { rendered : String, goals : Array String }` or null.
- `$/lean/plainTermGoal` — result `PlainTermGoal { goal : String,
  range : Range }` or null.
- `$/lean/fileProgress` — a server→client **notification**, params
  `{ textDocument : VersionedTextDocumentIdentifier, processing : Array
  { range : Range, kind : "processing" | "fatalError" } }`. This is the
  "orange bar": which regions are still elaborating.
- Also present, all deferred here: `$/lean/rpc/{connect,call,release,
  keepAlive}` (the interactive widget/infoview stack),
  `$/lean/prepareModuleHierarchy`, `$/lean/moduleHierarchy/{imports,
  importedBy}`, `$/lean/waitForILeans`, `textDocument/waitForDiagnostics`.
- From `src/Lean/Data/Lsp/InitShutdown.lean`:
  `InitializationOptions { hasWidgets? : Option Bool, logCfg? : Option
  LogConfig }`. `hasWidgets?` **defaults to false**, and its documented
  meaning is: when true, the server may *omit* information from standard
  LSP messages because the client will fetch it interactively. A
  plain-goal client wants the default. Omitting `initializationOptions`
  entirely is accepted (`FromJson` maps missing/null to `none`).
- Server launch, from `lean4-mode`'s `lean4--server-cmd`: `lake serve` when
  Lake ≥ 3.1.0 is found, else `lean --server`.

### 2.9 There is no blocking process run — and `lake` on PATH proves nothing

The complete `pmacs.process` Lua surface is `spawn`, `write_stdin`,
`terminate`, `list`, `status`, `events_take`, `forget`, `resize_pty`,
`_tick`. `ProcessSpec` (`src/process.rs:193`) carries no
wait-for-output mode, and there is no `wait_with_output` anywhere in
`src/process.rs`. Everything is asynchronous and drained off
`process.after-tick`, which is how compile mode streams.

**Consequence: any toolchain probe is async, and cannot gate the attach
that triggered it.** A design that reads "probe, then set
`pmacs.lsp.config.lean4.command`, then attach" cannot be written against
this substrate.

Second, and sharper — scouted on this machine:

```
$ elan --version
elan 4.2.1 (3d5138e15 2026-03-18)
$ lake --version
error: no default toolchain configured. run `elan default stable` to ...
$ lean --version
error: no default toolchain configured. run `elan default stable` to ...
```

`elan` installs `lake` and `lean` as **toolchain shims**. Both are on
PATH, both are executable, and both fail. So:

- A version probe must parse a *failure*, not just a version string —
  `lake --version` returning non-zero is a normal, common state.
- `command -v lake` is worthless as a capability check.
- The failure modes a Lean client must survive are: lake absent, lake
  present but shimmed with no toolchain, lake present and working but too
  old, and lake working but the directory is not a Lake package. Only the
  third is a *version* question.
- **Acceptance cannot assume a working Lean toolchain exists.** Every
  Stage 3b+ test runs against the fake LSP server; a live `lake serve`
  smoke is PATH-gated *and* success-gated, following the #123 JSON/YAML
  provider-smoke pattern.

### 2.10 Information-severity diagnostics are squiggled and counted

`src/diag.rs:50` defines `DiagnosticSeverity` with `Information` mapped
from LSP severity `3` (`:103`). Information diagnostics get the
`ui.diag.info` face (`:408`), a `UnderlineStyle::Single` squiggle
(`:426`), and a slot in the severity count tuple (`:223`) that feeds the
modeline.

Lean reports every `#eval`, `#check`, `#print`, and `example` result as an
information-severity diagnostic at the command's position. Under the
current surface those render as underlined "problems" with a gutter sign
and a modeline count — which is why rev 1 called them noise. The claim is
now specific: they are not merely unstyled, they are **actively
mis-rendered as defects**, and the count misleads.

The publish path absorbs into the Rust store *and* still delivers the
notification to `events_take`, so Lua can observe them; but suppressing
them from the store needs a Rust-side policy, not a Lua filter. Q#LN18.

### 2.11 The upstream input method (external, verified by reading it)

Scouted 2026-07-26 against `leanprover/vscode-lean4` @ `17d1d08`,
package `lean4-unicode-input`, files `AbbreviationProvider.ts`,
`TrackedAbbreviation.ts`, `AbbreviationRewriter.ts`,
`AbbreviationConfig.ts`, `abbreviations.json`, and — round 6 — the
package README at `lean4-unicode-input/src/README.md`. Apache-2.0.

**Rev 6 first claimed this package ships no README. It does**, at
`src/README.md` rather than the package root, and the 404 on the root
path was taken as absence without checking the directory listing that
was already in hand. That cost the tie rule below: the README states it
in one sentence, and reading only the code left it as an inference from
`Array.prototype.sort`'s stability rather than a documented contract.

**Resolution.** `findSymbolsByAbbreviationPrefix(p)` collects every key
having `p` as a prefix, sorts them by **key length ascending**, and maps
to symbols. `getReplacementText(a)`:

1. If any key has `a` as a prefix, return the shortest such key's symbol.
2. Otherwise recurse on `a` minus its last character; if that yields
   something, return it **with the dropped character appended**.
3. Otherwise undefined — no expansion.

Verified against the table: `alpha` → `α`, `alp` → `α` (via `alpha`),
`al` → `∀` (via `all`, *not* `alpha` — shortest wins, and this is
surprising enough to be worth an acceptance criterion), `alp7` → `α7`
via rule 2, `a` → `α` (`a` is itself a key, among 29 prefix matches).

**The tie rule, and why it is a constraint on the vendored format.**
When several shortest keys have equal length, upstream takes **the one
declared first in `abbreviations.json`**. The README says so outright;
the code achieves it because `Object.keys()` yields JSON insertion order
and `Array.prototype.sort` is stable. Ties are not rare: **101 prefixes
have equal-shortest candidates that resolve to *different* symbols**.
`f` picks `f<` → `‹` over `f>` → `›`; `"` picks `"A` → `Ä` from eleven
equal-length candidates; `(` picks `()` over `(=`, `(b`, `((`, `([`.

A Lua table iterated with `pairs` has no order at all, so **a generated
`{ [key] = symbol }` map cannot express this contract** — it would
resolve these 101 prefixes nondeterministically, and worse, *stably
wrong* per build. Q#LN11 therefore carries source rank alongside the
symbol.

**Two things the README explains that the code does not.** `Tab` is the
manual early-replacement trigger upstream binds, which is why
`getReplacementText`'s shortest-prefix rule is user-visible at all
rather than an internal detail. And the `[]_`/`{}_` entries in the table
are not symbols anyone types — they are **decoys**, added so that `\[`
is not uniquely-and-completely matching and therefore does not eagerly
expand before the user can type the second `[`. That is the same
collision Q#LN22 handles from the pairing side, solved upstream by
editing the data. Anyone regenerating the table must not "clean up"
those entries.

**Tracking.** The leader `\` is inserted into the buffer like any other
character, and the tracked range starts after it; the replaced range
spans the leader inclusive (`abbreviationRange.moveKeepEnd(-1)`). So the
buffer literally shows `\alpha` until expansion, then that whole span
becomes `α`.

**Termination.** There is no terminator set. On each typed character
`c`, if `findSymbolsByAbbreviationPrefix(a .. c)` is empty the
abbreviation is marked `finished`, **`c` is not absorbed into it**, and
the pending text expands before `c` lands. Otherwise `c` extends the
key. Two consequences the obvious "space ends it" model gets wrong:

- `'+ '` is a key, so after `\+` a space **extends**. Space is a
  terminator by consequence, never by rule.
- `'\'` is a key (→ `\`), so `\\` extends, is uniquely complete, and
  eagerly expands to one backslash. A second `\` terminates only when
  the pending key is non-empty and unextendable — at which point the
  rewriter starts a *new* tracked abbreviation on it.

**Eager expansion.** When `eagerReplacementEnabled`, an abbreviation
expands the moment it is *unique and complete*: exactly one key has it
as a prefix, and it is itself a key. 1,550 of the 1,855 keys qualify;
the other 305 are proper prefixes of some other key and must wait for
termination. `\to` is in the first group — it expands with no terminator
typed, which is why acceptance 41 is meaningful and not a restatement of
38.

**Cursor placement.** `$CURSOR` is stripped from the symbol and its
index becomes the post-expansion point, applied only when the point sat
at the end of the abbreviation. 26 values carry it.

**Abandonment.** Upstream expands on `changeSelections` — any tracked
abbreviation the cursor has left. pmacs has no cursor-motion hook
(round-5 finding 3), so this seam does not exist here and Q#LN22 makes
abandonment lazy instead.

**The re-arm guard pmacs does not need.** Three values contain a
backslash — `\` → `\`, `n` → `\n`, and `setminus` → `\` (rev 6 first
said two, dropping the `\` → `\` identity entry) — so an expansion can
insert a backslash; upstream sets
`doNotTrackNewAbbr` across the replace so that backslash does not open a
new abbreviation. In pmacs the expansion is a programmatic `buf:replace`
that arms no typed-edit record, so the chain sees nothing and cannot
re-arm. The guard is unnecessary here **because of** the provenance
contract, not by accident — and the acceptance must pin it, because a
future consumer that inferred from buffer text rather than provenance
would reintroduce the bug.

**What pmacs does not have to carry.** Multi-cursor within a frontend.
Upstream tracks a `Set<TrackedAbbreviation>` and sorts changes bottom-up
for that reason; pmacs has one point per frontend view.

**What pmacs has instead, and rev 6 got wrong.** Rev 6 read "no
multi-cursor" as "one point" and keyed pending state by buffer alone.
**pmacs is multi-frontend**: `EditorCore.views` is a
`HashMap<FrontendId, FrontendView>`, each with its own active window and
cursor; `take_typed_edit` is already keyed by frontend
(`typed_edit_armed: Option<(FrontendId, TypedEditRecord)>`, matched
against `active_frontend`); the record carries `window` as well as
`buffer`; and `pmacs.frontend.id()` is exposed to Lua. Two frontends
editing the same Lean buffer — the ordinary TUI-plus-GPU case, not an
exotic one — would share a single buffer-keyed pending slot, so one
could extend, expand, or silently clear the other's half-typed
abbreviation. `buffer.after-switch` makes it worse: it fires with no
arguments, so a buffer-keyed clear-on-switch would let *any* frontend's
navigation discard a pending abbreviation belonging to another. Q#LN22
keys the state accordingly.

## 3. Decisions

### Q#LN1 — Bundle `arborium-lean` 2.18; reject `tree-sitter-lean4`

Per §2.1. The decision is forced by the dependency graph, not by taste:
`tree-sitter-lean4`'s `tree-sitter ^0.25` cannot coexist with the
workspace's 0.26. Its missing queries and stale README are secondary.

Risk accepted: `arborium-lean` is a third-party republish from a grammar
collection, not the grammar's upstream. `codebook-tree-sitter-latex` (#144)
set this precedent for exactly the same reason — no usable first-party
crate exists. The mitigation is the same: pin a real parse + highlight
smoke test so a bad republish fails our suite, not a user's file.

### Q#LN2 — Name the entry `lean4`, not `lean`

`LanguageEntry.name` becomes the `didOpen` `language_id` (§2.2), and the
Lean ecosystem's id is `lean4` (vscode-lean4 uses it; `lean` is Lean 3).
The grammar's C symbol is `tree_sitter_lean`, but that is arborium's
business — the entry name is ours to choose.

Consequences, all deliberate: `pmacs.comment.strings.lean4`,
`pmacs.pair.sets.lean4`, `pmacs.lsp.config.lean4`, and a mode line reading
`lean4`. An Emacs `-*- mode: lean -*-` or a Vim `ft=lean` modeline is
normalized to `lean4` through `pmacs.parse.modeline_aliases`, so neither
spelling strands a file.

### Q#LN3 — Extensions: `lean` only

Not `.olean` (compiled binary artifacts — opening one as text is never
what the user wants) and not `.ilean` (JSON metadata; if anything it
belongs to the `json` entry).

### Q#LN4 — Add four capture entries and pin the retro-paint in both directions

**Ruled (round 1): add to the global table.** The alternative is an in-repo
overlay, and there is no partial option — see below.

Add to `Theme::default_dark()`: `constructor`, `character`,
`keyword.conditional`, and `warning`.

Rationale, per name:

- **`constructor`** is the consequential one. Per §2.3 it reaches seven
  language entries and its real effect is *"recolor every capitalized
  identifier in five entries and every `{}` in Lua"*. That is stated
  bluntly because it is the decision, not a side effect. It is
  nevertheless the right call: capitalized-identifier-as-constructor is
  the mainstream editor convention (it is what nvim-treesitter, Helix, and
  Zed all render off these same queries), those tokens are currently
  *unstyled* rather than deliberately plain, and Lua's braces gaining a
  colour is a cosmetic difference on a token that today renders as default
  text. The cost of avoiding it is owning a forked 213-line query forever.
- `character` — `tree-sitter-zig` only, currently unstyled.
- `keyword.conditional` currently flattens to `keyword`. Giving it
  `keyword.control`'s brighter style makes Lean's `if`/`then`/`else`/
  `match`/`do` read the way Rust's already do, and reaches cmake and zig
  the same way.
- `warning` has zero blast radius and gives Lean's `(sorry)` — an
  unproved goal, the single most important thing to see in a proof file —
  a visible style.

All four are pinned in the reverse direction exactly as #146 required —
acceptance 7 asserts the retro-paint *happened* on each affected language,
acceptance 8 asserts it did not leak into languages that emit none of the
four names.

**Rejected alternative:** an in-repo query overlay
(`builtin/queries/lean4/highlights.scm`) rewriting the capture names into
the existing vocabulary, the #144 LaTeX pattern. It avoids touching the
global table, but it forks a 213-line query we would then own and
hand-merge on every arborium bump. Overlays are for grammars whose crate
ships *no* usable query; arborium ships one.

**There is no middle option.** Styling Lean's constructors without
touching the other seven entries requires renaming the capture, which
requires the overlay, which forks the query. The choice is binary: accept
the retro-paint, or own the fork.

### Q#LN5 — Comments: `--` only in Stage 1

`pmacs.comment.strings.lean4 = "--"`. Lean's block comment is `/- ... -/`
and its docstring is `/-- ... -/`; block-comment toggling is an existing
named deferral of the comment arc (`docs/comment-toggle-framing.md`) and
this lane does not front-run it.

### Q#LN6 — Pair set includes `⟨⟩`, and the degradation is named

`pmacs.pair.sets.lean4 = { "()", "[]", "{}", "⟨⟩", "⦃⦄", "⟮⟯", '""' }`.

`⟨⟩` (anonymous constructor) is among the most-typed constructs in Lean and
omitting it would make the pair set feel broken. It is outside the nine
built-in pair chars, so per §2.6 its undo is cross-peer-degraded. That is a
documented, pre-existing limitation of user-extended pairs whose general
fix is chronological cross-peer undo arbitration — already on the standing
backlog. Ship it; name it in the module comment.

`''` is excluded: Lean uses `'` as a primed-identifier suffix (`h'`,
`foo'`), so pairing it would fight the user constantly. Same reasoning that
excludes it for Rust.

`⦃⦄` (strict implicit binder) and `⟮⟯` ride along — one list entry each,
same degradation, and both have abbreviation keys (`\{{}}`, `\([])'`) so
omitting them would make the input method produce brackets the pair set
does not understand.

### Q#LN7 — `lake serve` by default, with a lazy probe **and** a failure latch

```lua
pmacs.lsp.config.lean4 = pmacs.lsp.config.lean4 or {
  command = "lake",
  args = { "serve" },
}
```

`lean4-mode` probes Lake's version and falls back to `lean --server` below
3.1.0. This lane does the same, but **lazily and asynchronously**, and
pairs it with a failure latch — because §2.9 makes probe-alone
insufficient in two independent ways.

**Where the probe runs.** Not at init: `pmacs.lsp.config` is a declarative
table, and spawning a process at startup for every user, Lean-using or
not, is the cost rev 1 refused. It runs on the first `.lean` attach, in
`builtin/runtime/lean.lua`, cached for the session.

**Why the probe cannot gate the first attach.** There is no blocking
process run (§2.9). `pmacs.process.spawn` + `events_take` off
`process.after-tick` is the only shape available, so the probe's verdict
arrives *after* `ensure_server` has already had to decide. This is the
correction to the round-2 request, which assumed the verdict could be
consulted before configuring.

**The design that follows:**

1. First `.lean` attach spawns `lake serve` optimistically and fires
   `lake --version` alongside it, with `cwd` at the resolved Lake root.
2. If the probe reports a version below 3.1.0, or the server dies before
   `initialize` completes, a **one-shot latch** swaps in `lean --server`
   and restarts once. The latch is per session and never re-arms.

   **How the latch observes failure.** There is no event for "exited
   before initialize" — the drain ignores state events. The latch polls
   `pmacs.lsp.list()` for the server's `state.kind` on the same
   `process.after-tick` cadence Q#LN13 uses, and treats
   `crashed`/`stopped` reached without an intervening `initialized` as
   the trigger. (Q#LN9's pending-response purge fires on the same
   transition, so anything already awaiting a reply fails cleanly rather
   than hanging.)

   **Interplay with `RestartPolicy`.** The manager will otherwise respawn
   the same broken command underneath the latch, producing a loop the
   latch cannot see the end of. So the latch calls `pmacs.lsp.stop` on
   the failing server *first*, then swaps the config, then spawns — the
   fallback is a fresh server, not a restart of the old one.

   Round 4 verified this is necessary rather than defensive. The spec
   default is `LspRestartPolicy::OnCrash` (`src/lsp.rs:165`), and the
   termination handler calls `should_restart(policy)` — which, unlike the
   `termination_warrants_restart` helper beside it, never consults the
   exit code. `maybe_restart` re-fires on every elapsed backoff with **no
   attempt ceiling**, so a broken `lake` respawns indefinitely.
   `pmacs.lsp.stop` sets `restart = Never` on the way out
   (`src/lsp.rs:1349`), which is precisely what disarms it. Acceptance 36
   is pinning a live mechanism, not a hypothetical one.

   **Why the latch does not just set `restart = "never"` on the spawn.**
   It cannot: `ensure_server` never forwards `cfg.restart` to
   `pmacs.lsp.spawn` — `lua_to_lsp_spec` reads the key but the spawn
   table never sets it — so the field is silently dropped on every
   auto-attach today. That gap was found landing #161 and is not Stage
   3's to close (it changes behavior for every language that has set
   `restart` believing it worked; `statusline_segments_acceptance` a12 is
   one such caller). The stop-then-spawn ordering is correct regardless of
   how that gap is eventually resolved, which is the reason to prefer it
   over a fix that depends on the gap closing first.

   **The swap is a field update, not a table replacement.** It rewrites
   only `command` and `args`, preserving any user-supplied `env`,
   `settings`, `init_options`, and `root` on `pmacs.lsp.config.lean4`. A
   wholesale table replacement would silently discard a user's
   `init.lua` configuration at exactly the moment they are least likely
   to notice.
3. If `lean --server` also fails, the error surfaces through the ordinary
   `pmacs.lsp.last_error` path. pmacs does not attempt to install a
   toolchain.

**Why a latch and not just a probe.** Per §2.9 the failure modes are lake
absent, lake shimmed-with-no-toolchain, lake too old, and
not-a-Lake-package. **Only the third is a version question**, and the
scouting machine exhibits the second — `lake --version` there exits
non-zero with `error: no default toolchain configured`. Since the failure
path must exist regardless, the probe's job shrinks to the one case
failure detection would otherwise handle slowly (an old-but-working lake
that starts a useless server). Probe and latch are complements, not
alternatives.

**Named risk.** The optimistic first spawn means a user on a
lake-less-but-lean-ful toolchain sees one failed spawn before the
fallback. That is a one-line status message, once per session, and it
buys not blocking every other user's first attach behind a process
round-trip.

**Attribution (COHERENCE §9).** The probe is background work that spawns
an OS process, and `ProcessSpec.label` is the only identity a process
carries — caller-supplied and unvalidated, but it is what
`pmacs.process.list` renders. The probe spawns as `lean:lake-version-probe`
rather than inheriting a default, so a user who looks at the process list
while wondering why their editor touched `lake` finds an answer with an
owner in it. Both the probe's verdict and the latch firing report through
`pmacs.editor.set_status` — the channel that exists — per §1.2's rule and
its corollary: each is pinned by a test that observes the channel, since a
report through `pmacs.error` would be a dead sixteenth call site.

No `init_options`. Per §2.8, `hasWidgets?` defaults to false and that is
the correct value for a client that reads plain goals out of standard
messages.

### Q#LN8 — Lake-aware root via a **function-valued** `config.root`

**The generalization landed in Stage 2 (#161).** `project_root_for` is
now `builtin/runtime/lsp.lua:592` and returns `root, source`;
`config[lang].root` already accepts a `function(path) -> string|nil`,
with per-directory memoization keyed weakly on the resolver itself. What
remains for Stage 3b is Lean's resolver in `builtin/runtime/lean.lua`:
walk up from the file's directory collecting every ancestor containing
`lean-toolchain`, and return the **outermost**; decline (return nil) when
there is none, which falls through to `pmacs.project.detect` and then the
file's directory.

**How the walk tests for the marker — and why not the obvious way.**
`pmacs.fs.stat` is asynchronous: it returns an awaitable handle
(`builtin/runtime/fs.lua:133`) that only settles under `:await()` inside a
coroutine. The resolver has no coroutine. It runs synchronously inside
`ensure_server` ← `attach_buffer` ← the `buffer.after-load` hook, so
awaiting is not merely slow there, it is unavailable — and blocking the
attach on filesystem I/O is the cost rev 1 refused for the probe. The
walk therefore uses the **Lua stdlib**: `io.open(dir .. "/lean-toolchain",
"r")`, which returns nil for a missing path. Round 4 probed that `io` and
`os` are exposed in the sandbox rather than assuming it; `terminal.lua`
already depends on `os.getenv`.

One edge, probed: **`io.open` succeeds on a directory** (the handle opens;
`read` returns nil without raising). A `lean-toolchain` *directory* would
therefore read as a marker under an `io.open` truth test — wrong, and
wrong silently.

The fix is **not** "read a byte and require it to be non-nil", which was
this section's first answer and is wrong in the other direction: an
**empty** `lean-toolchain` file also reads nil at EOF, so that rule
declines a marker that exists. Marker semantics here are `lean4-mode`'s
`locate-dominating-file` semantics — *existence*, not content — and a
`lean-toolchain` can legitimately be empty. The discriminator is
`read`'s **second** return, probed on LuaJIT 2.1:

| Path | `io.open` | `f:read(1)` | Verdict |
|---|---|---|---|
| file with content | handle | `"l"`, no error | marker |
| **empty file** | handle | `nil`, **no error** | **marker** |
| directory | handle | `nil`, `"Is a directory"` | decline |
| missing | `nil` | — | decline |

So: `local data, err = f:read(1)` and decline only on a non-nil `err`.
The rule is robust across platforms without needing to be re-probed on
each, because both directory behaviors are declines — a platform whose
`fopen` refuses a directory outright fails at `io.open`, and one that
opens it fails at `read`. There is no platform on which a directory both
opens and yields a byte.

Acceptance 24a and 24b pin the two halves, and each must be shown to
fail against the implementation that satisfies only the other —
otherwise "handles directories" is satisfiable by the version that
breaks empty files, which is exactly how this section's first answer got
written.

**The result must be canonical.** #161's contract: a configured root
reaches `file_uri_for` verbatim and that URI is the affinity key, so two
spellings of one package are two servers. The path handed to the resolver
is *not* canonical (round 4, finding 2), and Lua had no canonicalizer —
hence Q#LN20. The resolver canonicalizes the file's directory **once**,
before the walk, and strips components from there: every ancestor of a
canonical path is itself canonical, so one call suffices. If
canonicalization fails (a deleted file, a broken symlink), the resolver
declines rather than returning a path it cannot vouch for.

**The walk stops at `pmacs.project.search_boundary()`.** This is not
optional politeness: `detect_project_within` (`src/project.rs:213`) exists
precisely so a stray marker above a temp fixture cannot leak into
detection, and a Lua walk that ignores the boundary breaks that contract —
including for acceptance 23, whose outermost-root assertion is otherwise
non-hermetic against any `lean-toolchain` that happens to sit in an
ancestor of the test's tempdir.

Why this and not the two alternatives:

- *Adding `lean-toolchain` to Rust's `default_markers()`* does not work —
  `detect_project` is innermost-wins by construction (§2.5), and inverting
  it globally would change Rust/Go/Node root detection for every user.
- *A `pmacs.project.add_marker` Lua binding* is a bigger new surface than
  this lane needs and still leaves the innermost/outermost problem.

The function-valued `root` is ~3 lines in `lsp.lua`, is a strict
generalization (a string still works), and puts the Lean-specific rule in
the Lean module where it belongs.

### Q#LN9 — Notification **and response** subscription seams in the existing dispatch

Per §2.4 there is exactly one `events_take` consumer, and its `if/elseif`
chain handles five `request` methods plus `initialized`. It ignores
notifications **and responses** — and no Lua anywhere in the runtime
consumes `ev.kind == "response"`. So today a `send_request` reply is
drained and dropped on the floor: **`send_request` is effectively a
write-only API from Lua.**

Rev 2 specified only the notification half. That was a hole, since
Q#LN16 (`waitForDiagnostics`), Q#LN19 (`imports` / `importedBy`), and
Q#LN12's typed goal request all await replies. Both halves ship in
Stage 3a.

```lua
pmacs.lsp.on_notification(method, fn)          -- fn(sid, params); persistent
pmacs.lsp.on_response(sid, request_id, fn)     -- fn(result, err); ONE-SHOT
```

Routed from two new arms of the existing loop:

- `elseif ev.kind == "notification"` → every subscriber registered for
  `ev.method`.
- `elseif ev.kind == "response"` → the one-shot registered for
  `(sid, ev.id)`, **removed before it is invoked** so a raising handler
  cannot be re-entered.

Each handler is `pcall`ed, so one raising subscriber cannot stall the
drain or starve the `request` arms that share it.

**Pending-response lifetime.** A one-shot whose server dies never fires on
its own, leaking the registration and hanging whatever awaits it. On
`crashed` / `stopped` / `restarting` for a `sid`, every pending one-shot
for that `sid` is invoked with an error and cleared. This is what lets
Stage 5's in-flight tracking (acceptance 52) be honest rather than
optimistic, and it is what Q#LN7's latch observes (below).

Explicitly **not** a second `events_take` caller — a second drain would
steal events from `handle_server_requests`. The tests pin that in both
directions: a Lean subscriber must not cause `workspace/applyEdit` to be
missed, and a raising subscriber must not stop later events in the same
drain.

**The seam's contract, stated because round 4 found it narrower than rev
4 implied: subscribers fire only for servers with a live buffer
attachment.** `handle_server_requests` builds its sid list from
`attachments`, so a server with no attached buffer is never drained — and
`push_event` appends with no cap, so that server's queue grows
unboundedly. Both facts are pre-existing and neither is Stage 3a's to
fix. What they change is where the purge may be wired.

**The purge must not ride the drain.** `attach_buffer` removes a sid
from `attachments` as soon as `server_is_live` reports false and rebuilds
the attachment against a fresh server, so a `crashed` / `stopped` event
is the event *least* likely to be drained — the drain stops visiting
that server at almost exactly the moment the event is queued. A purge
triggered by observing that event therefore leaks in the case it exists
to handle.

So the purge polls **`pmacs.lsp.list()`** after each drain instead. That
call enumerates the manager directly and is unaffected by attachment
bookkeeping, which is what makes it the right authority: a sid that is
absent, terminal, or running a new generation settles its pending
one-shots with an error, whether or not anything ever drained it.
Acceptance 34's second half exercises a server that is in **no**
attachment, because that is the shape an event-driven purge fails and a
polled one survives.

The uncapped queue is recorded as a named deferral (§6) rather than fixed
here: bounding it is a policy question about which events may be dropped,
and answering it inside a seam PR would be the kind of smuggling §4
forbids.

Stage 3b registers `$/lean/fileProgress` on the notification seam and
`waitForDiagnostics` on the response seam; stages 5 and 7 use the response
seam for `plainGoal` and the hierarchy calls.

### Q#LN20 — `pmacs.fs.canonicalize` (Stage 3a)

A synchronous binding wrapping `std::fs::canonicalize`, returning the
resolved absolute path or nil. Roughly fifteen lines.

It exists because #161 documented an obligation Lua cannot discharge. A
configured root — string or resolver return — is fed to `file_uri_for`
verbatim, and that URI is the server-affinity key; the `"detected"` arm is
canonicalized for free because `pmacs.project.detect` canonicalizes before
walking, but the `"config"` arm is not. Round 4 probed that
`pmacs.editor.file_path()` collapses `.` and `..` lexically while leaving
symlinks intact, so a resolver walking up from it returns a non-canonical
root. Opening one Lake package through a symlinked path and through the
real path would spawn two `lake serve` processes — the bug Stage 2 was
built to prevent, re-entered through Stage 3b's door.

**Synchronous, deliberately, and this is the one thing to get right.**
The whole reason `pmacs.fs.stat` cannot serve here is that it is async
(Q#LN8), so a canonicalizer that returned an awaitable would fail for the
same reason and leave the obligation undischarged. It is one `stat`-class
syscall on a path the editor is already opening; `pmacs.project.detect`
performs the same work synchronously today, on the same hook, so this
adds no blocking class that the attach path does not already have.

Why this rather than the two alternatives considered in round 4:

- *Accept it as a named degradation* — document that a symlinked open
  spawns a second server and pin the behavior. Rejected: it reopens the
  defect Stage 2 closed, and the failure is invisible (two servers, both
  apparently working, twice the memory, diagnostics split between them).
- *Anchor the walk on `pmacs.project.detect`'s canonical root* — free, no
  new surface. Rejected as incorrect, not merely inelegant: `detect` is
  innermost-wins over its own marker set, so with `.git` at `~/code` and
  the Lake package at `~/code/proj`, anchoring at `~/code` and walking
  *up* never sees `~/code/proj/lean-toolchain`. It resolves the wrong root
  in a layout that is entirely ordinary.

The binding is general, not Lean-shaped: it serves every future
function-valued `root`, and it is what lets #161's doc comment stop
warning about a footgun and start naming a fix.

### Q#LN10 — Stage 4a: one shared provenance read, not two

The hazard is §2.6 — `take_typed_edit()` is one-shot and `pair.lua`
already consumes it. A second independent caller in the same
`buffer.after-edit` fan-out gets nil or steals the record, depending on
hook order, and hook order is not a contract.

Decision: **`pair.lua` stops being the sole consumer.** Extract the
provenance read into a single `buffer.after-edit` subscriber owned by a
small shared module — `builtin/runtime/typed_edit.lua`, loaded
immediately before `pair.lua` — which takes the record once and offers
it to registered consumers in a defined order. A consumer returns
whether it **claimed** the edit; the first that claims stops the chain.

`pmacs.typed_edit.add_consumer { name = <string>, priority = <number>,
fn = function(rec) ... end }`, lowest priority first, ties broken by
registration order. Priority is an explicit number rather than
load-order-implied because Q#LN22's collision makes ordering
load-bearing, and rev 5's "the abbreviation consumer runs first" is a
claim a reader must be able to check without reconstructing
`src/editor.rs`'s include list.

**Stage 4a ships this and nothing else.** Its whole content is:

| File | Change |
|---|---|
| `builtin/runtime/typed_edit.lua` | new — the chain owner |
| `builtin/runtime/pair.lua` | re-expressed as one registered consumer |
| `src/editor.rs` | one `include_str!` line, before `pair.lua`'s |
| `tests/typed_edit_chain_acceptance.rs` | new — criteria 46a–46e |
| `tests/auto_pair_acceptance.rs` | **unchanged, zero lines** |

Rev 6 listed only the first three and then required criteria 46a–46e,
which no existing suite can host: the auto-pairing suite must stay
untouched (that is the whole point of criterion 46), so the chain's own
behavior — take-once, priority order, claim-stops-chain, throw
containment — has nowhere to live. A declared footprint that excludes
the tests its own acceptance demands is not a footprint. The new suite
joins the required gate list for this PR alongside
`tests/auto_pair_acceptance.rs`.

Round 5's finding 1 is why this is a PR and not a first commit —
`pair.lua` is every language's auto-pairing, and a reviewer looking at a
Lean PR should not have to also review a rewrite of it.

**The no-behavior-change claim must be pinned, not asserted.** The full
`tests/auto_pair_acceptance.rs` suite is a required gate for 4a and must
pass **unmodified** — a suite edited to accommodate the refactor proves
nothing (the recorded lesson: what a test suite pins is its assertions).
Three assertions the existing suite already makes are the load-bearing
ones, because they are what a chain could plausibly break: that a second
`take_typed_edit()` in the same fan-out yields nil, that pairing still
sees the exact record via `_capture_records`, and that the Q#AP7 ordering
against `lsp.lua`'s `didChange` flush still holds.

**What 4a deliberately does not do.** It does not change the `all-must-
succeed` contract, so a consumer that throws still fails the fan-out for
everyone. The chain owner therefore `pcall`s each consumer and reports
through `pmacs.editor.set_status`, matching `pair.lua`'s existing
never-throw-from-after-edit discipline — this is behavior-preserving for
pairing (which already never throws) and is the guardrail 4b needs.

### Q#LN11 — Stage 4b data: vendor the table, generated, attributed

`abbreviations.json` in `leanprover/vscode-lean4` is a flat
`string → string` object of **1,855 entries**, verified at commit
`17d1d08` (2026-05-29), 36,861 bytes, all keys ASCII, longest key 25
characters. The counts the algorithm depends on, all re-derived from the
file rather than estimated:

| Count | What it drives |
|---|---|
| 64 keys containing a `lean4` pair-set char | Q#LN22's ordering |
| 305 keys that are proper prefixes of another | which keys can expand eagerly |
| 1,550 keys uniquely-and-completely matching | the eager-expansion set |
| 26 values containing `$CURSOR` | point placement |
| 119 multi-codepoint symbols (26 of them `$CURSOR`-bearing) | the replace is not one-char-for-many |
| 101 prefixes with disagreeing equal-shortest ties | why the format carries source rank |

(Rev 6 gave the multi-codepoint figure as 93, which was the count
*excluding* the `$CURSOR` entries — a subset reported as a total.)

vscode-lean4 is Apache-2.0.

**Format: an ordered array, not a map.** §2.11's tie rule makes source
order semantic, and a Lua `{ [key] = symbol }` table iterated with
`pairs` cannot carry it. The generated file emits a **sequence** —
`{ {key, symbol}, ... }` in `abbreviations.json` order — plus a derived
`key → index` lookup built at load time for the exact-match case.
Resolution sorts candidates by `(#key, index)`, so the 101 ties resolve
the way upstream resolves them and the file's own line order is the
audit trail. A map-shaped emit would be nondeterministic across builds
and, once a hash order happened to be stable, *stably wrong*.

Vendor it as a generated `builtin/runtime/lean_abbrev.lua` with a header
recording source repo, commit, license, entry count, and the
regeneration command — the `builtin/queries/latex/highlights.scm`
precedent (#144) for third-party data, extended with provenance because
this is a much larger artifact under a named license.

Not fetched at runtime, not a package-manager dependency: the input
method must work offline and on first launch.

**Embedded, not lazily loaded.** ~45 KB of generated Lua joins the 414 KB
of builtin runtime already compiled in by `include_str!`, of which
`lsp.lua` alone is 111 KB. Inventing a lazy-load path for an 11% increase
would be new machinery bought with no measurement, and the arithmetic is
stated here so a reviewer can disagree with it on numbers.

**Upkeep is a documented manual process, not code.** There is no
automatic sync and none is wanted — an editor that silently re-downloads
its input method has a supply-chain problem, not a feature. The generator
script lives at `scripts/regen-lean-abbrev`, takes a vscode-lean4 commit
as its argument, and rewrites the file including its provenance header,
so the file is self-describing to whoever next touches it. A refresh is
an ordinary PR with a visible diff — which is the point: the diff is the
review.

**Escaping is canonical and lossless, not a rejection trigger.** Rev 6
said the generator aborts on "a key containing a character the emitted
Lua would have to escape." **That rule rejects the current table**: `\`
is a key, `"` begins eleven keys (`"A` → `Ä` …), and acceptance 45d
requires `\` to work. The generator instead emits every key and symbol
through one canonical Lua string escaper — `\\`, `\"`, `\n`, `\r`,
`\t`, and `\ddd` for any other control byte, everything else literal
UTF-8 — chosen so the emit is byte-deterministic across runs.

What the generator *does* abort on, because these are real corruption
rather than syntax:

- a duplicate key after decoding (JSON permits it; the table must not),
- a key or symbol that is not well-formed UTF-8,
- a round-trip mismatch: the generator re-parses its own output and
  compares the full ordered sequence against the source, entry for
  entry, and fails if they differ anywhere.

That last check is what makes the artifact trustworthy, and it belongs
in the generator rather than in the acceptance suite — the suite cannot
see `abbreviations.json`, which is not shipped. Same discipline as
Q#LN20's refusal to hand back a lossy path: refuse rather than emit
something plausible.

### Q#LN21 — Stage 4b: the expansion's undo is cross-peer-degraded; ship it, name it

`classify_key` (`src/optimistic.rs:144`) returns `Insert(c)` for `\` and
for every ASCII letter — only the nine built-in pair chars are excluded
(Q#AP1). So on a CRDT frontend the user's `\alpha` arrives as six
**source-peer** optimistic inserts, while the expansion is a single
**daemon-peer** `buf:replace` spanning all six. Undo across that boundary
is not chronologically arbitrated; this is the same defect Q#LN6 already
accepts for `⟨⟩`, `⦃⦄`, `⟮⟯`, one order of magnitude wider.

Considered and rejected: `pmacs.buffer.set_round_trip_input(buf, true)`,
which exists, is per-buffer, and would fix this exactly. Its six current
callers are all read-only generated buffers — listview, compile, dired,
terminal — and it does considerably more than disable optimistic insert:
per `src/editor_core.rs:505`, `dispatch_idle` reports false, so RET
reaches buffer-local bindings instead of inserting a newline. Turning it
on for every ordinary editable Lean source file would trade a known undo
degradation for an unknown behavior change across the whole editing
surface, and would make Lean the one language whose typing has a
different latency profile.

Also rejected: adding `\` to the always-round-trip set. It is
frontend-side and language-blind, so this would tax LaTeX, C, shell, and
every string literal in the editor to fix one language.

Decision: **accept the degradation, name it in the module comment, and
do not paper over it.** The general fix is chronological cross-peer undo
arbitration — already on the standing backlog, and the same fix Q#LN6
points at. What Stage 4b owes is honesty about scope: this is not "a few
brackets," it is every abbreviation the user types on a CRDT frontend.

### Q#LN22 — Stage 4b mechanism: lazy abandonment, explicit ordering

**Ordering.** The abbreviation consumer registers ahead of auto-pairing.
The collision is real: 64 keys contain a `lean4` pair-set character —
`\[[]]` → `⟦⟧`, `\(())` → `⸨⸩`, `\{{}}` → `⦃⦄`, `\{}` → `{$CURSOR}`.
With pairing first, typing `\[` inserts `[]` with the point between, so
the pending key is corrupted to `\[]` before the second `[` is typed and
`\[[]]` becomes unreachable.

(Rev 1 justified this with `\<>`, which was wrong: `<` is not in the pair
set per Q#LN6, so that key is safe under either order.)

**The contract the collision exposes:** the consumer must claim a
self-insert that **extends an open pending abbreviation**, not only one
that completes an expansion. A consumer that claims only completed
expansions hands every intermediate keystroke to auto-pairing, which is
exactly how `\[` gets corrupted. "Claimed" means the chain stops, not
that an edit was made.

**State machine**, per §2.11's ground truth rather than rev 5's
reconstruction of it:

- `\` typed in a `lean4` buffer opens a pending abbreviation: `{ buffer,
  window, start_offset, text = "" }`, keyed on **`(frontend, buffer)`** —
  see below.
- A subsequent self-insert `c` is claimed iff at least one key has
  `text .. c` as a prefix; then `text = text .. c`. If it is also
  uniquely-and-completely matching (one of the 1,550), expand now.
- If no key extends `text .. c`, expand `text` **first**, then let `c`
  land normally — the chain does *not* claim `c`.
- **A terminating `c` that is itself `\` is then reprocessed as a new
  leader**, opening a fresh pending abbreviation at its position. This
  is the rule acceptance 45d depends on (`\alpha\to` → `α→`) and rev 6
  specified the acceptance without specifying the rule; upstream gets it
  from `processChange`, where a `finished` abbreviation reports
  `isAffected = false` and so does not suppress the new-leader branch.
  Note this is *not* the `\\` case: there the pending text is empty, `\`
  extends rather than terminates, and the result is one literal
  backslash with no pending state left open.
- Expansion resolves through §2.11's rules — shortest key wins, ties
  broken by source rank, unmatchable tail appended (`\alp7` → `α7`).
- `$CURSOR` is stripped from the symbol and its index becomes the point.

**Ownership is per frontend, not per buffer** (§2.11). The key is
`(pmacs.frontend.id(), rec.buffer)`, and the stored `window` must still
match `rec.window` for the state to be usable — a frontend that moved
the same buffer into a different window is no longer typing where the
pending span is. Two consequences the buffer-only design got wrong:

- `buffer.after-switch` fires with **no arguments**, so it cannot say
  whose switch it was. The subscriber reads `pmacs.frontend.id()` at
  callback time — documented as "the frontend that produced the most
  recent dispatched input event" — and clears **only that frontend's**
  entries. A blanket clear would let one frontend's navigation discard
  another's half-typed abbreviation.
- `frontend.detached` fires with the raw frontend id and is the purge
  seam, exactly as `killring.lua` uses it (Q#KR11). Without it a
  detached frontend's pending state leaks for the life of the session.

This costs one table level and buys correctness in the ordinary
TUI-plus-GPU configuration, which is not an exotic setup — it is the
one this project ships two frontends for.

**Abandonment is lazy, because there is no cursor-motion hook** (round-5
finding 3). Pending state is validated at the next typed edit and
discarded when any of these no longer holds: the record's buffer and
window are the pending ones; `rec.effective_start` equals `start_offset
+ 1 + #text` (the point is still at the end of the pending span); and
the buffer's `revision()` advanced by exactly the pending edit.
`buffer.after-switch` clears the acting frontend's entries eagerly,
since that hook *does* exist. The
practical difference from upstream: a user who clicks away mid-`\alp`
and types elsewhere gets the pending state dropped rather than expanded.
Upstream expands it. **This is a deliberate divergence** — expanding
into a region the user has left is the worse failure, and pmacs cannot
detect the departure at the moment it happens.

**One `buf:replace`** for the whole expansion — one undo step, one CRDT
op, one effective-edit verification, with the same
rejected/altered-by-intercept reporting as `comment.lua`'s Q#CT5 and
`pair.lua`. A rejection drops the pending state; it does not retry.

**Gate:** `pmacs.config.define{ name = "lean.abbrev", type = "boolean",
default = true, mutability = "live" }`, read against the **source**
buffer of the typed edit — the `editing.auto-pair` precedent
(`pair.lua:46`), including its round-2 correction to resolve
`rec.buffer` rather than `pmacs.window.buffer()`.

**Language gate:** the consumer opens no pending abbreviation outside a
`lean4` buffer, resolved from `rec.buffer` for the same reason. `\` in a
Rust buffer is an ordinary character and `\[` there still pairs.

### Q#LN12 — Stage 5 sends `$/lean/plainGoal` through a typed Rust request

Per §2.4, `send_request` does **not** route positions through
`outbound_position`. Lean negotiates UTF-16 and Lean source is
overwhelmingly non-ASCII, so a Lua-built byte column would be wrong
wherever it matters most.

Stage 5 therefore adds a typed request that reuses `outbound_position`
unchanged. It spans **two files**, because the `_raw` naming is a layer
boundary, not a module:

- `src/lsp.rs` — `request_plain_goal`, alongside `request_hover`
  (`src/lsp.rs:1690`) and `request_definition` (`:1733`), which is where
  `outbound_position` is actually applied.
- `src/lua_bindings/mod.rs` — the `_request_plain_goal_raw` binding,
  alongside the `_request_hover_raw` family (`:9501`–`:9823`).

It is a thin builder — the result is passed through as JSON and parsed in
Lua, since `PlainGoal` is two fields and does not warrant a typed store.

It exists specifically to honor handoff §4's standing invariant rather
than quietly reintroduce the bug it was written to prevent.

**Where the arc's Rust actually lives** (rev 2 stated this in pre-renumber
stage numbers and was wrong three ways):

| Stage | Rust |
|---|---|
| 1 | `Cargo.toml` + `BUILTIN_LANGUAGES` entry + Q#LN4's four capture entries |
| 2 | `lsp.list()` row builder (`mod.rs:9926`) |
| 3a | `pmacs.fs.canonicalize` (Q#LN20) — the seams themselves are Lua only |
| 3b | **none** — Lua only |
| 4 | **none** — Lua only |
| 5 | `request_plain_goal` + its binding |
| 6 | `LspServerSpec` severity-policy field and its publish-path honoring |
| 7 | `request_prepare_module_hierarchy` + its binding |

Stages 3 and 4 — the two largest Lean-specific stages — are entirely Lua.

### Q#LN13 — Stage 5 goal panel shape

- `*lean-goal*`, read-only via the erroring-intercept idiom, module writes
  with `{ bypass_intercept = true }` (§2.7).
- Displayed with `pmacs.window.display(buf, { side = "bottom", select =
  false })`. `select = false`: a goal view that steals focus on every
  cursor move is unusable.
- **Refresh mechanism, named explicitly because there is no motion hook.**
  The complete hook inventory is `buffer.{after-edit,after-load,
  after-save,after-switch,before-save}`, `editor.before-quit`,
  `frontend.detached`, and `process.after-tick`. Nothing fires on cursor
  movement. Stage 5 therefore refreshes from a **debounced poll off
  `pmacs.hook.add("process.after-tick", …)`** — the cadence pattern
  autosave (`autosave.lua:139`) and compile (`compile.lua:687`) already
  use — comparing the point against the last position it queried and
  issuing at most one in-flight `$/lean/plainGoal` at a time.

  This is written down so Stage 5 cannot quietly grow either an
  unframed polling loop or new hook substrate. A `cursor.after-move` hook
  would be the better long-term answer; it is out of scope here and is
  named in §6.
- Also refreshed on a `$/lean/fileProgress` notification whose
  `processing` array no longer covers the point's range.
- Content: `PlainGoal.rendered` when present, "no goals" when the result is
  null with the file elaborated, "elaborating…" when file-progress still
  covers the point. The three states are distinct and the middle one is the
  one users actually need to trust.
- Keys under `pmacs.keymap.bind { scope = "mode", mode = "lean4", … }`
  (#129's mode-scoped keymaps).

### Q#LN14 — No protocol change in any stage

Stages 1–4 and 7 touch no wire surface at all. Stage 5's panel rides #155
Stage 1, which is grid-only and bumped nothing; Stage 6 adds a field to
the in-process `LspServerSpec`, which is not wire. A GPU-rendered goal band
needs bottom-panel Stage 2, which is itself unframed — so the GPU half is
deferred, not attempted. Protocol stays v20.

### Q#LN15 — Multi-root server affinity (Stage 2, substrate)

Today `ensure_server` reuses any live server whose `language_id` matches,
**regardless of project root** (`builtin/runtime/lsp.lua:524`–`536`, whose
own comment documents this as a known limitation). For Lean this is not a
rough edge but a correctness failure: `lake serve` is bound to one Lake
package, so the second package a user opens gets a server that cannot
resolve its imports.

The change was small and spanned two files (Stage 2, landed as #161):

- **`src/lua_bindings/mod.rs:9919`** — the `lsp.list()` row builder sets
  `id`/`label`/`language_id`/`command`/`state`/`attempt`. Add `root_uri`
  from `spec.root_uri` (already `Option<String>` on `LspServerSpec`,
  `src/lsp.rs:125`) and `cwd`. Bump the `create_table_with_capacity`
  hint.
- **`builtin/runtime/lsp.lua:521`–`551`** — hoist `local root =
  project_root_for(language, path)` **above** the reuse loop and match on
  the `(language_id, root_uri)` pair.

**Correcting the round-2 request:** `root` is currently computed at
`:537`, *after* the loop, not before it. Hoisting is therefore part of the
change, and it has a consequence: `project_root_for` begins running on the
reuse path, where it previously ran only on spawn. For Q#LN8's
function-valued Lean resolver — which walks the filesystem — that means
once per attach rather than once per spawn. The resolver memoizes per
directory for the session.

**Comparison rule, part 1 — hand-spawned servers.** Compare
`info.root_uri` against the request's affinity key, with nil matching nil.
A server spawned directly from `init.lua` with only `cwd` set has
`root_uri = nil` and will therefore *not* match a root-bearing request —
it gets a new server rather than being silently adopted. Conservative and
deliberate, but a behavior change, so acceptance asserts it.

**Comparison rule, part 2 — markerless files must not fragment.** Per
§2.5, `project_root_for` **never returns nil for a file with a path**: its
last fallback is `dir_of(path)`. A naive `(language_id, root)` key
therefore gives *every directory of markerless scratch files its own
server*, for **every language** — two loose `.py` files in different
directories would spawn two pyrights where today they share one. That is a
silent regression for Python, Go, TypeScript and everyone else, caused by
a change made for Lean.

Ruling: **the affinity key is the root only when a root was actually
detected.** `project_root_for` returns `(root, source)` with `source` one
of `"config"`, `"detected"`, or `"fallback"`; the affinity key is `root`
for the first two and **`nil` for `"fallback"`**. The directory is still
passed as `cwd` / `rootUri` exactly as today — only the *matching* key
changes.

Consequences, both intended:

- Files in a real project (Cargo/Lake/go.mod/…) get one server per root —
  the fix.
- Markerless loose files keep today's single shared server per language —
  no change, which is the point.

**Rejected alternative:** keying on the fallback directory anyway and
accepting per-directory servers. It fragments the common scratch-file case
for every language in the editor to buy nothing for Lean, whose files are
essentially always in a Lake package.

**Blast radius, stated plainly.** This is the central server-affinity
function for *every* LSP language in pmacs. A bug here routes a file to
the wrong server: diagnostics land on the wrong buffer, or a redundant
server spawns. This is why it is Stage 2 and its own PR, with no Lean
content in the diff — a cross-cutting change to every language's server
affinity must not be reviewable only as a Lean feature.

**Named risk: unbounded server growth.** Per-root affinity means opening
files across N Lake packages spawns N `lake serve` processes, and Lean
elaboration is memory-hungry. rust-analyzer has the same property and no
editor caps it by default. No cap ships here; `pmacs.lsp.stop` is the
manual escape, and an LRU reaping policy is named in §6.

### Q#LN16 — `textDocument/waitForDiagnostics` (Stage 3b)

A plain request (no position, so no `outbound_position` concern — Q#LN12
does not apply). It resolves when the server has finished elaborating the
document.

Two uses, in order of importance:

1. **Deterministic acceptance.** Lean elaboration is slow and
   asynchronous; a test that sleeps is flaky and a test that polls is
   slow. This is the seam that makes a live `lake serve` smoke
   deterministic when a toolchain happens to be present.
2. A `M-x lean-wait-for-diagnostics` command, and a gate for the goal
   panel's "elaborating" state (Q#LN13) that is cheaper than parsing
   `$/lean/fileProgress` ranges.

Sent through `pmacs.lsp.send_request` and awaited through the Q#LN9
notification/response seam. ~20 lines.

### Q#LN17 — Lean in markdown fences (Stage 1)

The injection engine (#122) resolves fence names through
`pmacs.parse.injection_aliases`, a case-folded, Lua-extensible map
snapshotted into `ParseRequest`. Register `lean` and `lean4` → `lean4`.

Two lines, and it is the one place where the Lean 3 spelling is
deliberately *not* normalized away: a ` ```lean ` fence is overwhelmingly
Lean 4 in practice, and mapping it to the `lean4` grammar is right.

`lean4-mode` does the equivalent through `markdown-code-lang-modes`. Being
in Stage 1 means Lean blocks in this repo's own docs highlight from the
first PR.

### Q#LN18 — `#eval` / `#check` output channel (Stage 6)

Per §2.10, Lean's command output arrives as information-severity
diagnostics and pmacs squiggles them, signs them in the gutter, and counts
them in the modeline. VS Code shows them in the infoview instead. This
stage routes them.

Decision: **a per-server severity policy on the spec, not a Lua filter.**
The publish path absorbs into the Rust `DiagnosticStore` before Lua sees
the notification, so a Lua-side filter would suppress the *display* while
leaving the store's counts wrong. Add an optional
`diagnostic_severity_policy` to `LspServerSpec` — default "all severities
to the store", which is a no-op for every existing language — and have the
Lean config route `Information` to the output channel only.

The channel itself is a `*lean-output*` buffer using the same read-only
generated-buffer idiom as Q#LN13, appended to in position order and
cleared per publish for the owning document.

Deliberately *not* merged into the goal panel: a goal is a property of the
point, output is a property of the file, and the two refresh on different
triggers. Merging them is what makes VS Code's infoview complicated.

### Q#LN19 — Module hierarchy (Stage 7)

`$/lean/prepareModuleHierarchy` at the point returns hierarchy items;
`$/lean/moduleHierarchy/imports` and `.../importedBy` expand one in either
direction. Rendered with `pmacs.listview.open{ name, header, rows,
on_visit, on_refresh, display = "panel" }` (`listview.lua:111`) — the same
panel the LSP references/outline views already use.

`prepareModuleHierarchy` is position-bearing, so it goes through the
Q#LN12 typed-request path; the two expansion calls take an item, not a
position, and can use `send_request` directly.

Last stage because it is the least load-bearing: it is navigation
convenience, and nothing else in the arc depends on it.

## 4. Stage boundaries and why this order

Each stage is one branch, one PR, and is independently useful if the next
never lands.

| Stage | Ships | Substrate risk | Depends on |
|---|---|---|---|
| 1 | grammar, mode, comments, pairs, md fences | new crate; **global capture table** | — |
| 2 | multi-root server affinity | **`ensure_server`, shared by every language** | — |
| 3a | notification/response seams + purge; `pmacs.fs.canonicalize` | **the shared event drain, run by every language** | — |
| 3b | `lake serve` + probe/latch, Lake root, `waitForDiagnostics` | none — Lean-only files plus one config entry | 1, 2, 3a |
| 4a | typed-edit consumer chain | **refactors `pair.lua`'s provenance read, shared by every language** | — |
| 4b | Unicode input method | none — Lean-only files plus one chain consumer | 1, 4a |
| 5 | goal panel | new typed LSP request; panel adopter | 3a, 3b |
| 6 | `#eval` / `#check` output channel | **new `LspServerSpec` policy field** | 3b, 5 |
| 7 | module hierarchy | listview adopter + one typed Rust request | 3a, 3b |

Five of the nine carry risk that is *not* about Lean — stages 1, 2, 3a,
4a, and 6 each change something every language touches. That is the
organizing principle of the split: **no PR in this arc mixes a
cross-cutting substrate change with Lean feature content.** A reviewer
looking at Stage 2 sees only `ensure_server`; a reviewer looking at Stage
3b sees only Lean.

Round 4 found Stage 3 breaking that rule while stating it — the row above
used to read "two `lsp.lua` generalizations" for a stage the prose called
Lean-only. One generalization shipped as Stage 2; extracting the other as
3a is what makes the claim true again. The rule is only worth writing
down if it survives contact with a stage that is inconvenient to split.

Round 5 found the *same* rule broken again, by Stage 4, whose risk column
read "refactors `pair.lua`'s provenance read" — every language's
auto-pairing — for a stage described as the Lean input method. Rev 5 had
noticed the shape and answered it with a commit boundary; a commit
boundary is not a review boundary. Twice in two re-scouts is the
interesting part: **this rule is not self-enforcing, and a stage only
looks Lean-only until someone re-reads its own risk column.** Every
remaining stage should be re-checked against it at scout time, not
assumed.

Ordering notes:

- **Stage 2 has no Lean in it and could ship independently of this arc.**
  It is sequenced here because Lean is the language that makes its absence
  a correctness bug rather than an inconvenience, and because Stage 3b's
  acceptance would otherwise have to encode the broken behavior.
- **Stage 3a likewise has no Lean in it**, and the same reasoning applies
  one level down: the response seam is a hole in `send_request` for every
  language — Lean is merely the first caller that needs a reply. It is
  sequenced before 3b because 3b's `waitForDiagnostics` and file-progress
  subscription both consume it, and because a Lean PR that also rewrote
  the shared drain could not be reviewed on either axis.
- **3a and 3b cannot run as sibling worktrees.** 3b's Lean subscriber is
  written against the seam 3a adds, and both touch
  `builtin/runtime/lsp.lua`. Unlike stages 1 and 2, this pair is strictly
  sequential — recorded here, per the #126/#127 lesson, rather than
  discovered in a rebase.
- **Stage 4a depends on nothing in this arc** — not even Stage 1. It is
  a pure runtime-substrate change whose only content is `pair.lua` and a
  new module beside it, and it would be worth landing if the Lean arc
  were abandoned tomorrow, because "the typed-edit record has exactly
  one consumer forever" is not a property anyone chose.
- **4a and 4b cannot run as sibling worktrees**, for the 3a/3b reason:
  4b's consumer is written against the registration API 4a adds. Strictly
  sequential, recorded before either starts.
- **Stage 4b depends on stages 1 and 4a and on nothing else** — not on
  2, 3a, or 3b. The input method is useful with no language server at
  all, which is the honest ordering argument for putting it this early:
  a user with no Lean toolchain installed still gets a Lean editor that
  can type Lean. It could run in parallel with the 5/6/7 lane, but
  should not, per the #126/#127 lesson that parallel-safety requires the
  file split be agreed *before* either lane starts.
- **Stage 6 depends on Stage 5** only for the read-only generated-buffer
  and panel machinery, which Stage 5 establishes. If Stage 5 slips, Stage
  6 can carry that machinery itself at the cost of duplicating it.

Stage 1 is deliberately shippable alone. If the arborium grammar turns out
to be worse in practice than its query suggests (see §5, bet 3), that is
discovered at Stage 1 for the cost of Stage 1 — and stages 2 through 7 are
almost entirely independent of grammar quality, since they are driven by
the language server rather than the parse tree.

## 5. Categorical bets

Stated so they can be scored, per house style.

1. **`arborium-lean`'s ABI-15 parser loads under `tree-sitter 0.26` with a
   single `tree-sitter` in the graph.** Falsified by `cargo tree -d`
   showing a duplicate, or by the loader failing `Parser::set_language`.
   Confidence: high — `tree-sitter-language 0.1` exists precisely for this
   and roughly fifteen shipped grammars already rely on it.
2. **No protocol change in any stage.** Falsified by any new wire variant.
   Confidence: high.
3. **The grammar is good enough that highlighting reads as correct on
   ordinary Lean, including Mathlib-style files.** This is the weakest bet
   in the lane, and the upstream author's own warning is the reason: Lean's
   syntax is user-extensible via macros, so a static grammar necessarily
   mis-parses custom notation. Scored against a real fixture set at Stage 1
   acceptance. If it fails, Stage 1 still ships — degraded highlighting on
   exotic notation is strictly better than none — but the framing is
   revised to say so plainly rather than overselling it.
4. **`$/lean/plainGoal` alone is a useful goal view, without the
   `$/lean/rpc/*` widget stack.** Confidence: medium-high — it is exactly
   what `lean4-mode` shipped for years before infoview widgets, and
   `hasWidgets? = false` is a supported client posture, not a hack.
5. **The abbreviation expander needs no Rust.** Falsified if the one-shot
   provenance refactor (Q#LN10) cannot be done in Lua, or if `buf:replace`
   inside `buffer.after-edit` re-enters the hook in a way pairing does not
   already survive. Confidence: medium — pairing does the same thing, but
   over a single codepoint rather than a multi-byte span.
5a. **Lazy abandonment is good enough without a cursor-motion hook**
   (rev 6, Q#LN22). Falsified if a user in normal editing hits a case
   where stale pending state produces a *wrong* expansion rather than a
   dropped one — the failure mode this design chooses. Confidence:
   medium-high, because every path that can invalidate the state either
   goes through `buffer.after-edit` (where it is checked) or through
   `buffer.after-switch` (where it is cleared), and the residual is a
   cursor move with no intervening edit, which the next typed edit
   catches by position. If it fails, the fix is a cursor-motion hook —
   substrate work with its own framing, not a patch to this stage.
5b. **Stage 4a is behavior-preserving.** Falsified by any change to
   `tests/auto_pair_acceptance.rs` being needed to make it pass.
   Confidence: high, and cheap to score — it is a diff-level check, not
   a judgment call. This bet is stated separately from bet 5 because it
   is the one a reviewer can falsify in ten seconds.
6. **These nine stages reach rough VS Code parity for everything except
   the interactive infoview.** Scored honestly rather than aspirationally.
   What lands: highlighting, goal view, Unicode input, diagnostics,
   hover, completion, goto-definition, symbols, semantic tokens, `#eval`
   output, module hierarchy, correct multi-package roots. What does
   **not**: interactive/collapsible goals, `Try this` code-action
   suggestions, widgets, the term-mode goal on hover, and the
   `$/lean/rpc/*` session that powers all of them. That gap is real and
   is the arc's eventual destination (§6) — a framing that claimed parity
   without it would be overselling.
7. **Stage 2's affinity change breaks no existing language.** Falsified by
   any regression in the Rust/Python/Go/TS acceptance suites, or by a
   user's hand-spawned server no longer being adopted in a way they
   relied on. Confidence: medium-high for the suites, deliberately lower
   for hand-spawned servers — Q#LN15's comparison rule changes that case
   on purpose, and the acceptance pins it rather than hiding it.

## 6. Deferred (named)

Pruned in round 2 — seven former entries are now stages 1–7 (see §0.1).
What remains deferred:

- **Interactive infoview** — `$/lean/rpc/{connect,call,release,keepAlive}`,
  widgets, collapsible goal trees, `Try this` code actions, term-mode goal
  on hover. **This is the arc's eventual destination, not a rejection.**
  It needs `hasWidgets? = true`, a real RPC session lifecycle with
  keep-alive, and a rendering surface for structured rather than plain
  goals — plausibly its own multi-stage arc once stages 1–7 are in. Bet 6
  scores what its absence costs.
- **GPU goal band** — blocked on bottom-panel Stage 2 (Q#LN14). The panel
  is grid-only until then.
- **A `cursor.after-move` hook** — there is none (Q#LN13), so Stage 5
  polls off `process.after-tick` and Stage 4b abandons pending
  abbreviations lazily rather than on departure (Q#LN22, round-5 finding
  3). A real motion hook would serve the goal view, the input method,
  `completion.lua`'s cursor-delta heuristic, and the outline/hover panels
  alike; it is substrate work that should not be invented inside a
  language lane. Two consumers in this arc now want it, which is worth
  recording as evidence for whoever frames it.
- **Chronological cross-peer undo arbitration** — the general fix for
  Q#LN6's bracket pairs and Q#LN21's abbreviation expansions alike.
  Already on the standing backlog; named again here because Stage 4b
  widens the exposure from three pair characters to every abbreviation a
  user types on a CRDT frontend, which changes how often the existing
  defect is met without changing what it is.
- **Per-buffer optimistic-apply policy** — the narrower thing Q#LN21
  actually wanted and did not build. `set_round_trip_input` is the only
  existing lever and it is too blunt (it also changes RET dispatch); a
  frontend-side, language-aware round-trip character set would fix the
  undo degradation for Lean without taxing every other language, and
  would retire Q#AP1's limitation too. Frontend + protocol work, so
  Q#LN14's no-protocol-change rule keeps it out of this arc entirely.
- **LSP server reaping / LRU** — Q#LN15's per-root affinity makes
  unbounded `lake serve` growth possible. No editor caps this by default
  and pmacs will not either in this arc, but the policy question is now
  live in a way it was not before.
- **The uncapped LSP event queue** — `push_event` appends without a
  bound, and `handle_server_requests` drains only servers with a live
  buffer attachment, so an unattached server's events accumulate for the
  life of the session (round 4, finding 6). Bounding it means deciding
  which events may be dropped, which is a policy question with
  user-visible consequences for diagnostics and progress; Stage 3a states
  the seam's contract around the behavior rather than changing it.
- **`LspManager::stop` on an already-terminal server strands it.** The
  not-initialized branch terminates the (already-dead) process and sets
  `ShuttingDown { shutdown_request_id: None }` on the premise that "the
  next exit observation cleans up" — but for a `Crashed` client the exit
  has already been observed, which is what produced that state. No
  further event arrives, so the client sits in `ShuttingDown`
  permanently: `server_is_live` counts it as live (neither crashed nor
  stopped), so `attach_buffer` never rebuilds against it, and
  `LspManager::forget` refuses it for not being terminal. **Stopping a
  dead server is what makes it un-replaceable.** Found implementing
  Stage 3b's latch, which works around it by dispatching on state:
  `forget` for a terminal server (it requires terminal state, and
  removing the client also drops the `next_restart_at` the crash armed),
  `stop` for a live one. Merely *skipping* the call is not enough — that
  leaves the restart timer running and the broken command respawns
  underneath the fallback. The fix belongs in `stop` (treat an
  already-terminal client as a no-op, or drive it straight to `Stopped`)
  and changes behavior for every language, so it does not ride a Lean PR.
- **Forwarding `cfg.restart` through `ensure_server`** — read by
  `lua_to_lsp_spec`, never set by the spawn table, so silently dropped on
  every auto-attach (found landing #161). Fixing it changes behavior for
  every language whose config sets the field believing it works. Q#LN7 is
  designed not to need it.
- **Block-comment toggle** (`/- -/`) and **docstring awareness**
  (`/-- -/`) — confirmed as owned by the comment arc's framing, not this
  one.
- **`.olean` / `.ilean` handling** (Q#LN3).
- **Lean 3 support** — `.lean` files predating Lean 4 will mis-parse.
  Out of scope permanently; Lean 3 is end-of-life.

## 7. Acceptance

**Stage 1**

1. `cargo tree -d` shows exactly one `tree-sitter` version after adding
   `arborium-lean`.
2. A `.lean` fixture parses: the loader produces a tree whose root node is
   `module` and which is not all-ERROR.
3. Opening `foo.lean` sets `pmacs.buffer.major_mode` to `lean4`.
4. An Emacs `-*- mode: lean -*-` modeline and a Vim `ft=lean` modeline both
   resolve to `lean4`.
5. Highlighting produces non-default styles for a comment, a `def` name, a
   `theorem` name, a string, and a numeric literal in the fixture.
6. `(sorry)` picks up the `warning` style.
7. **Reverse-direction positive pin (#146).** Every language the four new
   capture entries reach asserts its *expected delta* — not that nothing
   moved, since these languages necessarily move:
   - `rust`, `python`, `javascript`, `javascriptreact`, `typescript`,
     `typescriptreact`: a capitalized identifier (`Some`, `MyClass`) picks
     up the `constructor` style. All seven entries are covered because
     `HIGHLIGHT_QUERY` composition means the JS-family entries inherit the
     rule rather than restating it — a regression in composition would
     otherwise go unseen.
   - `lua`: a table literal's `{` and `}` pick up the `constructor` style.
   - `zig`: a character literal picks up `character`; a conditional picks
     up `keyword.conditional`.
   - `cmake`: a conditional picks up `keyword.conditional`.
8. **Reverse-direction negative pin.** Fixtures in languages verified to
   emit **none** of the four capture names render byte-identically to
   their pre-change baseline: `markdown`, `json`, `yaml`, `html`, `css`,
   `c`, `cpp`, `go`, `toml`, `bash`.

   Rev 1 named Lua and Python here, which was a self-contradiction: both
   are retro-painted by `constructor`, so a fixture that did not move
   would have been vacuous — the #155 R2 assertion shape. Whichever
   fixtures ship, the negative pin must be shown non-vacuous by
   confirming it *fails* when a capture the language does emit is added.
9. `M-;` comments and uncomments a Lean line with `--`.
10. Typing `⟨` inserts `⟨⟩` with the point between; likewise `⦃` and `⟮`.
    Typing `'` after an identifier does **not** pair.
11. A ` ```lean ` fence and a ` ```lean4 ` fence in a markdown buffer both
    highlight as Lean (Q#LN17); a fence with an unknown name still does
    not.
12. **No live toolchain required.** The whole Stage 1 suite passes on a
    machine with no `lean`, no `lake`, and no configured elan toolchain
    (§2.9) — Stage 1 touches no process at all.

**Stage 2 — multi-root affinity (no Lean content)**

13. `pmacs.lsp.list()` rows carry `root_uri` and `cwd`.
14. Two files of the **same language in different project roots** spawn
    **two** servers, each with its own `rootUri`. Exercised with the fake
    server so it is toolchain-free.
15. Two files of the same language in the **same** root reuse **one**
    server — the pre-change behavior, pinned so the fix does not become
    "always spawn".
16. **Regression pin, per language:** the existing Rust, Python, Go, and
    TypeScript attach paths behave unchanged for the single-root case
    that is all they exercised before.
17. **Hoist pin:** `project_root_for` is called on the reuse path, and a
    function-valued `root` is invoked at most once per directory per
    session (Q#LN15's memoization) rather than once per attach.
18. **Hand-spawned server pin:** a server spawned from `init.lua` with
    `cwd` but no `root_uri` is *not* adopted by a root-bearing attach —
    the deliberate behavior change, asserted rather than discovered.
19. A crashed or stopped server in the matching root is not reused; a new
    one spawns.
20. **Loose-file pin (Q#LN15 part 2).** Two **markerless** files of the
    same language in **different** directories still share **one** server.
    This is the no-change case, and it is the one a naive `(language_id,
    root)` key breaks — `project_root_for` never returns nil for a file
    with a path, so it must be asserted, not assumed.
21. **Fallback-vs-detected pin.** A file under a real project marker and a
    markerless file of the same language get **different** servers, and
    the markerless one's server carries the fallback directory as `cwd`
    while matching on a nil affinity key.

**Stage 3a — dispatch seams and the canonicalizer (no Lean content)**

Driven against `pmacs_fake_lsp` through an already-shipped language, for
the same reason Stage 2's suite was: the drain is shared by every
language, and a suite that reaches it only through Lean would understate
the blast radius.

- **29.** A notification delivered through the fake server reaches a registered
  `on_notification` subscriber.
- **30.** **Dispatch-integrity pin:** with a subscriber registered, a
  `workspace/applyEdit` request in the same drain is still handled — no
  event is stolen.
- **31.** A subscriber that raises does not prevent later events in the same
  drain from being processed.
- **32.** **Response-seam pin (Q#LN9).** A `send_request` reply reaches its
  registered `on_response` one-shot, and the one-shot is **removed
  before** invocation — a raising handler is not re-entered. Bites
  against rev 2, where no Lua consumed `ev.kind == "response"` at all
  and the reply was dropped.
- **33.** **Response dispatch-integrity pin.** With a response subscriber
  registered, `workspace/applyEdit` in the same drain is still handled;
  a raising response handler does not stop later events in that drain.
  Mirrors the notification-side pins above.
- **34.** **Pending-purge pin, both edges.** A server that dies with a
  response outstanding invokes the pending one-shot with an error and
  clears it. **And** a server that is in **no attachment** does the
  same, rather than stranding the registration behind a drain that never
  visits it. The second half must be shown to fail against a purge
  wired to a death event seen in the drain; otherwise this criterion is
  satisfied by the implementation that leaks. (Rev 5 first worded the
  second edge as a killed buffer; there is no buffer-kill hook, so
  nothing removes the attachment and that path does not leak. Corrected
  in round 2 — see §0.1 finding 6.)
- **34a.** **Canonicalizer pin (Q#LN20).** `pmacs.fs.canonicalize` resolves a
  symlinked and dot-segmented path to the same string as the real path,
  and returns nil for a nonexistent one. Fixture builds the symlink
  rather than assuming one exists.
- **34b.** **Affinity-through-canonicalization pin.** With a function-valued
  `root` that canonicalizes, the same project opened by its real path
  and through a symlink reuses **one** server. Falsified by a resolver
  that returns the path verbatim, which yields two — this is the
  regression Q#LN20 exists to prevent, so it is asserted at the
  affinity layer, not just at the binding.

**Stage 3b — the Lean language server**

- **22.** Opening a `.lean` file inside a Lake package spawns one server with
  `cwd` and `rootUri` at the package root.
- **23.** **Outermost-root pin:** a file under
  `<pkg>/.lake/packages/dep/…` whose ancestor chain contains two
  `lean-toolchain` files resolves to `<pkg>`, not to `dep`. Run with
  `pmacs.project.set_search_boundary` at the fixture root so the
  assertion is hermetic.
- **24.** **Boundary pin:** with the search boundary set at the fixture root, a
  `lean-toolchain` planted in an ancestor *above* the boundary is not
  reached — the resolver stops at the boundary rather than walking past
  it.
- **24a.** **Marker-is-a-file pin (Q#LN8).** A `lean-toolchain`
  *directory* does not mark a root. Bites against the bare `io.open`
  truth test, which round 4 probed succeeds on directories — the shape
  that would pass every other criterion here while being wrong.
- **24b.** **Empty-marker pin (Q#LN8).** An **empty** `lean-toolchain`
  file *does* mark a root — marker semantics are existence, not content.
  Bites against the read-a-byte-and-require-non-nil rule, which declines
  it at EOF. 24a and 24b must each be shown to fail against the
  implementation that satisfies only the other; a suite carrying just
  one of them is satisfied by a resolver that is silently wrong for the
  other case.
- **25.** A string-valued `pmacs.lsp.config.lean4.root` still works — the Q#LN8
  generalization is strictly additive.
- **26.** `didOpen` carries `languageId = "lean4"`.
- **27.** **Fallback-latch pin (Q#LN7):** a `lake` stub that exits non-zero —
  reproducing §2.9's shimmed-elan state — causes exactly **one** restart
  against `lean --server`, and a second failure surfaces an error rather
  than looping. The latch does not re-arm within the session.
- **28.** **Probe pin:** a `lake` stub reporting version 3.0.0 triggers the
  fallback; one reporting 3.1.0 does not. A stub that never exits does
  not block the attach — the optimistic `lake serve` spawn proceeds.
- **35.** **Config-preservation pin (Q#LN7).** After the fallback latch fires,
  user-supplied `env` / `settings` / `init_options` / `root` on
  `pmacs.lsp.config.lean4` survive; only `command` and `args` change.
- **36.** **No-respawn-loop pin.** The latch stops the failing server before
  spawning the fallback, so `RestartPolicy` does not respawn the broken
  command underneath it.
- **36a.** **Attribution pin (COHERENCE §9/§1.2).** The probe process
  appears in `pmacs.process.list` under a Lean-owned label, and the
  latch firing leaves a status-line trace. Both assert through the
  channel a user can actually observe; a report added through
  `pmacs.error` alone must fail this.
- **37.** `textDocument/waitForDiagnostics` resolves through the response seam
  (Q#LN16), **carrying both `uri` and `version`** — Lean's
  `WaitForDiagnosticsParams` requires the document version, and a fake
  server that echoes any payload will hide its absence, so the fixture
  must reject a request that omits it.
  **PATH-and-success-gated live smoke:** if `lake serve` starts
  successfully a real elaboration completes and diagnostics arrive;
  skipped otherwise, never failed.

These two sections are bulleted with explicit labels rather than
numbered, because the split leaves each stage's criteria non-contiguous
(3b runs 22–28 then 35–37) and a markdown ordered list renumbers from
its first item regardless of what is written. Keeping the labels literal
means **every rev-4 number still denotes what it denoted in rev 4** —
"acceptance 34", "acceptance 27" — and the four criteria added in this
revision take letter suffixes rather than displacing anything. Round 3's
finding 4 was stale cross-references surviving a renumber; not
renumbering is the cheaper way to not repeat it.

**Stage 4a — the typed-edit consumer chain**

Criterion 46 keeps its number and moves here — it was always the
substrate pin, filed under Stage 4 only because Stage 4 was one stage.
Per the no-renumbering rule above, round 5's additions take letter
suffixes on both sides of the split.

46a–46e live in a **new `tests/typed_edit_chain_acceptance.rs`**, which
is part of Stage 4a's declared footprint (Q#LN10) and a required gate
for its PR. They cannot live in `tests/auto_pair_acceptance.rs`, which
criterion 46 requires to stay byte-identical.

46. **Provenance-refactor pin:** the full `tests/auto_pair_acceptance.rs`
    suite passes **unmodified**. A suite edited to accommodate the
    refactor proves nothing; the diff for 4a must show zero lines
    changed in that file.
46a. The chain reads the record exactly once: with two consumers
    registered, a `take_typed_edit()` from inside either observes nil,
    and both consumers receive the *same* record fields. Bites against a
    chain that re-takes per consumer (which would hand the second one
    nil in production and pass a single-consumer test).
46b. Ordering is by declared priority, not registration order: two
    consumers registered low-priority-last still run
    low-priority-first. Bites against a chain that "works" only because
    `include_str!` order happens to agree with intent.
46c. A claiming consumer stops the chain — a later consumer does not
    run — and a non-claiming one does not.
46d. A consumer that throws is contained: the fan-out still succeeds,
    the other consumers still run, and the failure reports through
    `set_status`. Bites against the `all-must-succeed` contract taking
    the whole fan-out down with one bad consumer (Q#LN10).
46e. **Q#AP7 ordering survives.** The existing `sighelp` fake-server
    test — pairing's closer must be in the buffer before `lsp.lua`
    flushes `didChange` — still holds with pairing behind the chain.
    Falsified by moving the chain's registration after `lsp.lua`'s.

**Stage 4b — the Unicode input method**

38. `\alpha` + space yields `α ` — the space lands first and the
    expansion runs in the following `buffer.after-edit`, so the
    terminator is **retained**, not consumed. The expansion is a single
    undo step: one undo restores `\alpha ` (with its space), not
    `\alph`. Rev 6 wrote the post-undo text as `\alpha`, which would be
    true only if the terminator were swallowed.
39. `\<>` yields `⟨⟩` with the point between them, from the `$CURSOR`
    placeholder.
40. **Pair-collision pin (Q#LN22).** `\[[]]` yields `⟦⟧`: each `[` is
    claimed as an extension of the pending abbreviation, so auto-pairing
    never inserts a closing `]` into the pending key. Bites against an
    ordering where pairing runs first, and against a consumer that claims
    only completed expansions rather than pending extensions — **both
    failure modes must be shown**, since they are distinct bugs with the
    same symptom.
41. `\to` yields `→` eagerly on uniqueness, with no terminator typed.
42. A prefix with no match (`\zzzz` + space) is left as literal text; no
    edit is made.
43. **Lazy abandonment (Q#LN22).** Because there is no cursor-motion
    hook, this asserts what pmacs can actually detect: after `\alp`, an
    explicit `goto_byte` elsewhere followed by typing `h` inserts a
    plain `h` and leaves the `\alp` text untouched — the pending state
    is dropped, not expanded. Plus: `buffer.after-switch` clears pending
    state eagerly. **Rev 5's version of this criterion was not
    buildable**; recorded so the change is visible rather than silent.
44. `pmacs.config.set("lean.abbrev", false)` disables expansion; the
    setting is read against the typed edit's **source** buffer.
45. Expansion does not fire in a non-`lean4` buffer — including that a
    pending abbreviation is never opened there, so `\[` in a Rust buffer
    still pairs normally.
45a. **Shortest-key resolution (§2.11).** `\alp` + space yields `α`, and
    `\al` + space yields `∀` — from `all`, not `alpha`. The second is
    the one that bites: a "longest match" or "unique match only"
    implementation passes the first and fails this.
45b. **Suffix rule.** `\alp7` + space yields `α7`. Bites against an
    implementation that drops unmatchable trailing characters or
    abandons the whole abbreviation.
45c. **There is no terminator list.** `\+` followed by space extends
    rather than terminating, because `'+ '` is a key. Bites against any
    implementation with a hardcoded space/tab/RET terminator set — which
    is what rev 5 specified.
45d. **`\\` yields a single `\`**, by extension-and-eager-match rather
    than by treating the second `\` as a terminator. And after a
    *non-empty* pending key, a second `\` does terminate and open a new
    abbreviation: `\alpha\to` + space yields `α→`.
45e. **No re-arm through inserted text (§2.11).** `\setminus` + space
    yields a literal `\`, and typing an ordinary letter after it inserts
    that letter — the inserted backslash opens no pending abbreviation,
    because the expansion is a programmatic replace that arms no record.
    Bites against a future consumer that infers pending state from
    buffer text instead of provenance.
45i. **Pending state is per frontend (Q#LN22).** Two frontends attached
    to the same `lean4` buffer: A types `\al`, B types `\to` + space in
    the same buffer. B's expansion yields `→` and leaves A's `\al`
    pending and intact; A then typing `l` + space still yields `∀`.
    Plus: B switching buffers does not clear A's pending state, and a
    `frontend.detached` for B purges B's entries only. Bites against the
    buffer-keyed design rev 6 specified — which passes every
    single-frontend criterion above.
45f. **Both producers, and the CI-darkness stated.** The dispatch path
    is pinned by the criteria above. The optimistic CRDT producer
    (round-5 finding 4) is pinned by a separate criterion driving
    `handle_remote_crdt_op`, which is `#[cfg(feature = "crdt")]` and
    therefore **dark in CI and dark in the required gate list**, since
    that list runs `--features crdt` only for `--lib`. The PR must
    either land that coverage as a `--lib` test where the gate reaches
    it, or state in its description that the optimistic path was
    verified only locally and name the command. Silence here is the
    failure mode — a green CI would otherwise read as covering the path
    most users take.
45g. **Table integrity — what the suite can actually check.**
    `abbreviations.json` is not shipped, so the suite cannot diff
    against it and rev 6's "matches byte-for-byte" was unbuildable; a
    count plus seven spot entries could not prove 1,855 round-trip
    anyway. The full source-fidelity check belongs to the generator
    (Q#LN11: re-parse own output, compare the ordered sequence entry for
    entry, fail on any difference). What the suite pins instead are
    self-consistency properties that a corrupt emit breaks:
    - the loaded sequence's length equals the header's declared count,
      and equals the declared count for the recorded upstream commit;
    - every key is unique, and the derived `key → index` lookup has the
      same cardinality as the sequence (a collision would silently drop
      entries);
    - every key and symbol is well-formed UTF-8, and no symbol contains
      `$CURSOR` more than once;
    - the resolution spot-set behaves: `alpha`, `to`, `<>`, `+ `, `\`,
      `n`, `setminus`, and the tie cases from 45h.
45h. **Tie-break by source order (§2.11).** `\f` + space yields `‹` —
    `f<` and `f>` are both length 2, and `f<` is declared first. Same
    for `\"` + space → `Ä`, first of eleven equal-length candidates.
    **This is the criterion that bites a map-shaped vendored table**:
    with `pairs` iteration it passes or fails by hash order, so it must
    also be run against a deliberately reversed sequence and shown to
    fail. 101 prefixes are exposed to this rule.

**Stage 5 — the goal view**

47. `$/lean/plainGoal` is sent with a position encoded through
    `outbound_position` — pinned with a UTF-16 fake server and a
    non-ASCII Lean line, which fails against a raw byte column.
48. A non-null `PlainGoal` renders `rendered` into `*lean-goal*`.
49. A null result with the file elaborated renders "no goals".
50. A point inside a range still covered by `$/lean/fileProgress` renders
    the elaborating state, not "no goals".
51. **Refresh pin (Q#LN13).** Moving the point to a new position and
    driving `process.after-tick` past the debounce issues exactly one new
    `$/lean/plainGoal`; ticking again with the point unmoved issues none.
52. **In-flight pin.** A second point move while a request is outstanding
    does not issue a concurrent request, and the panel ends on the result
    for the *latest* position — a stale response for an abandoned
    position never wins.
53. The panel opens at the bottom without stealing focus.
54. `*lean-goal*` rejects a user edit and accepts a module write.
55. **Teardown pin.** After the Lean buffer is killed or the frontend
    detaches, driving `process.after-tick` issues **no** further
    `$/lean/plainGoal` and writes **nothing** to the panel, and any
    outstanding request's one-shot has been purged.

    Worded as an observable because it must be: `pmacs.hook` exposes
    `add` / `define` / `list` / `run` and **no `remove`**. A subscription
    cannot be torn down, only made inert — so "leaves no subscription"
    (rev 2's wording) is untestable and, taken literally, unimplementable.

**Stage 6 — the output channel**

56. An information-severity diagnostic from the **Lean** server lands in
    `*lean-output*` and **not** in the diagnostic store: no squiggle, no
    gutter sign, and the modeline info count stays zero.
57. Warning- and error-severity diagnostics from the Lean server are
    unaffected and still reach the store.
58. **Cross-language pin:** an information-severity diagnostic from a
    **non-Lean** server still squiggles and still counts — the
    `LspServerSpec` policy defaults to a no-op.
59. Output is cleared per publish for the owning document, so a
    re-elaborated file does not accumulate stale `#eval` results.
60. Output rows appear in source-position order regardless of publish
    order.

**Stage 7 — module hierarchy**

61. `$/lean/prepareModuleHierarchy` is sent through the Q#LN12 typed path
    (position-bearing), pinned against a UTF-16 fake server.
62. `imports` and `importedBy` each render into the listview panel and are
    navigable through the existing `on_visit`.
63. An empty result renders an empty panel with its header, not an error.
64. The panel's `q` returns to the originating buffer, not to another
    panel (`listview.lua:118`'s existing rule).

## 8. Prior art in pmacs

- **#144 (LaTeX)** — the third-party-republish grammar decision and the
  vendored-artifact-with-provenance pattern.
- **#146 (HTML+CSS)** — the global capture table, and the requirement to
  pin retro-paint in both directions. Q#LN4 is that lesson applied.
- **#123 (JSON/YAML)** — declarative `pmacs.lsp.config` entries with a
  fake-server delivery proof plus PATH-gated live smokes. Stage 3b follows
  it, with the extra success-gate §2.9 forces.
- **#110 (auto-pairing)** — `take_typed_edit()` provenance, the fail-closed
  discipline on transformed source edits, and Q#AP1's optimistic-classifier
  limitation. Stage 4a generalizes the first; 4b is built on all three.
- **#127 (config registry)** — `pmacs.config.define` and the
  source-buffer-resolution correction. Q#LN22's gate follows
  `editing.auto-pair` exactly.
- **#129 (mode system)** — mode-scoped keymaps for Stage 5.
- **#155 (bottom panel)** — `pmacs.window.display` and the panel adopter
  shape, for stages 5–7.
- **#113 (compile mode)** — the erroring-intercept read-only generated
  buffer idiom (stages 5 and 6), and `process.after-tick` as a debounced
  cadence source (Q#LN13).
- **#122 (multi-language injections)** — `pmacs.parse.injection_aliases`,
  which Q#LN17 registers into.
- **#94/#95 (LSP panels)** — `pmacs.listview.open` and the
  references/outline panel shape that Stage 7 reuses wholesale.

## 9. Coherence impact (COHERENCE §20)

Required of every framing since #163. Stated for stages 3a and 3b, the
work this revision authorizes; the earlier stages predate the rule and
are not retrofitted here.

**Sections served.** §1.2 (the silence asymmetry) primarily, and §7
(first-class workspaces) indirectly — per-root affinity is the workspace
concern arriving one language at a time. §9 (worker identity) is touched
but not advanced.

**Golden journey (§2).** No step is touched. Neither stage changes what
happens between launching pmacs and editing a file; Lean is not on the
journey's critical path, and 3a is invisible to a user who has no Lean
installed. Stage 3b does make §2's step-3 grade slightly *worse* in one
narrow way, and it is honest to say so: a preconfigured-but-missing
`lake` is one more instance of the silent-spawn-failure class, on a
toolchain many users will not have. Q#LN7's status-line reports on the
probe verdict and the latch cover the Lean-specific paths, but they do
not fix the general failure — that remains Priority 1 work with its own
framing, as §1.2's frequency note already records.

**Interaction islands (§6).** None added. Stage 3b introduces no keymap,
no modal surface, and no dispatch shadow. Its one user-facing command
(`M-x lean-wait-for-diagnostics`, Q#LN16) registers through the ordinary
command table and is reachable from `M-x` like everything else.

**Config registry (§11).** Neither stage adds a `pmacs.config` option.
`pmacs.lsp.config.lean4` joins the existing declarative server table
alongside sixteen other languages — deliberately *not* the typed registry,
because moving one language's entry there while the other sixteen stay
put would fragment the surface rather than unify it. Migrating
`pmacs.lsp.config` wholesale is a config-arc concern; this lane must not
create a precedent that makes it harder. Stage 4b's `lean.abbrev` gate is
where this arc does enter the registry, and Q#LN22 already commits to the
`editing.auto-pair` shape.

**Background-work attribution (§9).** Three pieces of background work,
each with a named owner and an observable trace:

| Work | Identity | Trace |
|---|---|---|
| `lake --version` probe | `ProcessSpec.label = "lean:lake-version-probe"`, visible in `pmacs.process.list` | status line on a verdict that triggers fallback |
| the fallback latch | the server it stops/spawns is already in `pmacs.lsp.list()` | status line on firing |
| root resolution | none — synchronous, inside the attach | status line on resolver failure (shipped #161) |

This is attribution within the identity layer §9 says is absent, not a
fix for its absence: the probe carries a label because
`ProcessSpec.label` is the only field available, and §9's own ground
truth calls that "caller-supplied, unvalidated convention." Owner/purpose
/parent fields remain unbuilt, and nothing here joins the four activity
planes. What this lane commits to is not *worsening* the ratio — every
background action it adds is nameable in some user-visible view on the
day it ships.

**Debt this revision retires.** Q#LN20 closes the gap #161 could only
document: a configured root reaching `file_uri_for` uncanonicalized. That
was coherence debt of exactly §1.3's compounding kind — a correct
substrate with a footgun the next caller was expected to disarm by
reading a comment.

**Debt this revision names rather than pays.** Three, all in §6: the
uncapped event queue, the dropped `cfg.restart`, and — unchanged from
#161 — surfacing the spawn failure itself. Each is a behavior change for
languages other than Lean, and §4's rule is what keeps them out of a Lean
PR.

### 9.1 Coherence impact — stages 4a and 4b (rev 6)

**Sections served.** §6 (interaction islands) primarily, and in the
*preventing* direction rather than the fixing one — see below. §11
(config registry) secondarily, by adding one option in the established
shape rather than a new switch mechanism.

**Golden journey (§2).** No step is touched by 4a. 4b improves **step 5
("Edit immediately")** for Lean specifically and changes nothing for any
other language — rev 6 cited step 4, which is "Understand the visible
interface" and is untouched by both stages: the pending-abbreviation state exists only in `lean4` buffers.
Neither stage changes launch, open, or attach.

**Interaction islands (§6).** **None added, and this is the load-bearing
claim of Stage 4b.** An input method is the archetypal island: a modal
state where ordinary keys mean something else, usually with its own
keymap, its own escape, and its own set of commands that only work
inside it. Stage 4b deliberately has none of those. There is no keymap,
no dispatch shadow, no mode line indicator, no command that only works
mid-abbreviation, and no key that exits. The pending state is invisible
to every other subsystem, is abandoned by ordinary editing, and its
worst failure is that the user's literal text stays literal. The
`lean.abbrev` switch is an ordinary registry boolean, not an island
toggle.

Stage 4a's chain is the mechanism that makes that possible, and it also
retires a smaller island risk: today the only way for a second feature to
react to a typed character is to compete with `pair.lua` for a one-shot
record, and the natural workaround — inferring from buffer text — is how
input methods grow their own private state and, eventually, their own
modal surface.

**Config registry (§11).** One option, `lean.abbrev`, in exactly the
`editing.auto-pair` shape (boolean, `mutability = "live"`, resolved
against the typed edit's source buffer). This is the arc entering the
registry as §9's earlier text predicted, and it is a genuine adoption
rather than a new surface. Stage 4a adds none.

**Background-work attribution (§9).** Neither stage does background work.
Both are synchronous inside an existing hook fan-out; no process is
spawned, no timer armed, no request issued. There is nothing to attribute
and nothing to worsen — recorded explicitly because "none" is an answer
this section should be able to give without ambiguity.

**Debt this revision retires.** The unowned assumption that
`take_typed_edit()` has exactly one consumer forever. That was never a
decision — it was the shape of the only caller — and every future
typed-character feature would have had to rediscover it. Stage 4a turns
an accident into an API with a stated ordering contract.

**Debt this revision names rather than pays.** One, and it is real:
Q#LN21's cross-peer undo degradation, now covering every abbreviation
rather than three bracket pairs. The fix is chronological cross-peer undo
arbitration, already on the standing backlog and already blocking Q#LN6.
Stage 4b makes the existing gap more visible without widening the class
of defect — but "more visible" is the honest word, not "unchanged."
