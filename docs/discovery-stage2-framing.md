# Discovery Stage 2 — M-x rows stop being bare names

**Status: framing pass, revision 2. Pre-implementation. Awaiting
approval.**

**Revision 2 fixes two claims revision 1 made about compatibility and
about the TUI, both wrong, both checkable.** An in-place field change
cannot preserve v22 — postcard is not self-describing — and "both
frontends render it" was false, because the grid TUI never reads that
message at all. Verified in the tree, not reasoned about.

---

## 1. The gap, stated exactly

`COHERENCE.md` §5 grades unified discoverability **Partial** after
Stage 1 (#207), and names three things left. This lane takes one:

> `Command` still has no title/category/flags, **M-x rows are still
> bare names**, and the Rust help layer is still orphaned.

**The descriptions already exist.** `Command.description` is a required
field (`src/command.rs:69`), and `help.list-commands` already renders
"every registered command **with its description**"
(`builtin/runtime/help.lua:339`). A user who runs `M-x help` can read
what everything does.

**What they cannot do is see it at the moment of choosing.** `M-x`
shows names alone — so the information exists, is already surfaced
elsewhere, and is missing from the one place it would change a
decision. That is §1.1's *substrate without surface* in its purest
form, and it is felt every time the editor is used.

## 2. Ground truth

Scouted:

- **The wire asymmetry is a single field.**
  `InstanceMessage::MinibufferPrompt` carries
  `candidates: Vec<String>` (`pmacs-protocol/src/message.rs:1113`).
- **The rich pattern is already proven in a sibling variant.**
  `CompletionPopup` carries `rows: Vec<CompletionPopupRow>` — `label`,
  `kind: u8`, `detail: Option<String>` (`:1387`) — and both frontends
  already render it.

  *(Revision note: an earlier read of mine reported two bare-string
  sites. There is one. The second grep hit was `CompletionPopup`'s
  doc comment, which says "candidates" while the field is `rows`.)*
- **`Command` needs no change for this lane.** `description` is
  already there and already required. Title/category/aliases — the
  lane's other Stage-2 candidate — would enrich these rows further and
  are **deliberately not** in scope: they are a ~175-site change and
  this lane can deliver the felt improvement without them.
- **`ADVERTISED_PROTOCOL_VERSION` is pinned at 20**
  (`pmacs-protocol/src/message.rs:1767`) and **must not be edited**,
  per handoff §3/§5.
- **The transport is postcard** (`pmacs-protocol/src/transport.rs:1`),
  which is **not self-describing**: enum variants encode by index and
  fields by position. **Changing a field's type in place is a wire
  break**, not a compatible evolution — a v22 peer would mis-decode the
  bytes rather than ignore them.
- **`MinibufferPrompt` is sent to every peer negotiated `>= 12`**
  (`src/daemon.rs:1472`, "Q#MB1 — MinibufferPrompt gated at v12"). So
  the population that would break is every frontend from v12 to v22.
- **The grid TUI never reads `MinibufferPrompt`.** `src/editor.rs`
  contains **zero** references to it; `paint_minibuffer` reads
  `core.minibuffer` directly and renders the selected candidate as an
  inline suffix, `format!("  [{cand}]")` (`src/editor.rs:5484`), with
  its own `ui.minibuffer.candidate` face. **The rich wire reaches
  `pmacs-gpu` only.**

## 3. The change

**This is a protocol change: v22 → v23**, and it is **additive**, not
an edit.

### 3.1 A new variant, because an in-place change cannot be compatible

Revision 1 proposed changing `candidates` in place. **That breaks every
frontend from v12 to v22**: postcard encodes fields positionally, so a
v22 peer decoding a `Vec<MinibufferRow>` where it expects
`Vec<String>` mis-reads the bytes — it does not skip them.

And gating the changed variant at `>= 23` does not rescue it: the peer
would then receive **no minibuffer message at all**, because there is
only one variant to send. Compatibility means *sending the old shape*,
which requires the old shape to still exist.

So:

- **`MinibufferPrompt` is retained, unchanged, for v12–v22.** Its
  encoding is frozen.
- **`MinibufferPromptRows` is a NEW variant appended to the enum**,
  carrying `rows: Vec<MinibufferRow>` and otherwise mirroring
  `MinibufferPrompt`'s fields.
- **Appended, not inserted.** Variant indices are positional in
  postcard; inserting anywhere but the end renumbers every later
  variant and breaks everything at once.

### 3.2 Per-session selection, and the ordering that matters

- **Selection is per peer, decided from its negotiated version**:
  `>= 23` receives `MinibufferPromptRows`; `12..=22` receives
  `MinibufferPrompt`. This mirrors the existing gates in
  `src/daemon.rs:1472`, which already suppress `MenuPrompt`,
  `MinibufferPrompt` and `LineNumbers` per peer.
- **Exactly one of the two is sent to any given peer, ever.** Sending
  both to a v23 peer would double-render; sending neither is the bug
  gating alone would have caused.
- **Close and cache ordering.** The prompt is cached-compare
  suppressed, so the cache key must be **per variant**, or a v23 peer
  that reconnects at v22 (or vice versa across a restart) can have its
  first message suppressed as a duplicate of one it never received.
  **The close message must use the same variant family as the open** —
  a `MinibufferPromptRows` session closed by a legacy clear is exactly
  the kind of mismatch that leaves a popup on screen forever.

### 3.3 What each frontend does

- **`pmacs-gpu`** renders label + detail from the new variant.
- **The grid TUI does not consume this message at all** and is
  addressed separately in §3.4.

### 3.4 The TUI presentation contract

Revision 1 said "both frontends render label + detail". **The grid TUI
does not read `MinibufferPrompt`** — it paints from `core.minibuffer`
and renders the selected candidate as `format!("  [{cand}]")`
(`src/editor.rs:5484`). The wire change reaches it not at all.

*My vote: **an inline selected form, matching what is already there***:

```
M-x buffer.sa    [buffer.save — Write the buffer to its file]
```

- **Source: local.** The TUI is in-process with the core, so it reads
  `Command.description` from the registry directly. **No wire
  involvement**, which is why this half of the lane is independent of
  the bump.
- **Only the selected candidate**, as today. This is a formatting
  change to an existing suffix, not a new surface.
- **Clipping is explicit**: the suffix is already written against
  `max = term_size.cols` with a running `written` count. The **name
  must survive clipping and the description is what gets truncated** —
  a row that clips to `[buffer.sa…]` would be strictly worse than
  today. If the terminal is too narrow for `name — ` plus one
  character of description, **the description is dropped entirely**
  rather than shown as an ellipsis stub.
- **The `ui.minibuffer.candidate` face already exists** and continues
  to cover the suffix.

**A multi-row TUI chooser is explicitly NOT this lane.** It would be a
new interaction surface, a §6 island risk, and materially larger than
the wire work — it is named here so that "make the TUI match the GPU"
does not quietly become that.

### 3.5 Scheduling consequence, which is not incidental

`PROTOCOL_VERSION` is a strict serialization point — two lanes bumping
it collide, and this session recorded eight broken version assertions
from a single bump. So:

- **This lane holds the bump slot.** Git Stage 1 is deliberately
  no-wire and runs beside it without contention.
- **Git Stage 2 (gutter markers) also needs a bump and must therefore
  wait for this to land.** That ordering should be explicit in the
  ledger rather than discovered when the two collide.

## 4. Coherence impact (§20)

- **§5 unified discoverability — the direct target**, and the specific
  clause "M-x rows are still bare names".
- **Journey step 4** ("understand the interface"): `COHERENCE.md` P4
  says most of it "rides on" discovery. This improves the step without
  adding one.
- **§16 semantic frontend:** a clean instance of the architecture —
  the instance states *what a candidate is*, each frontend decides how
  to draw it. Degradation is the established practice (Q#D2-4).
- **Interaction islands (§6): none added.** No new key interception;
  this changes what an existing prompt carries.
- **Config registry:** no new setting. Whether detail rendering is
  optional is Q#D2-3, and my vote is no setting at all.
- **Background-work attribution (§9): untouched.** No new background
  work.

## 5. Open questions

### Q#D2-1 — reuse `CompletionPopupRow`, or a new type?

Reuse is tempting and I think wrong. `CompletionPopupRow.kind` is an
**LSP `CompletionItemKind` code (1..=25)** with a documented contract;
an M-x command is not an LSP completion item and has no honest value
for that field. Reusing it would mean either inventing a fake kind or
declaring 0/unknown everywhere — a type whose invariant is
"meaningless in half its uses".

*My vote: **a new `MinibufferRow { label, detail: Option<String> }`***
— no `kind`. If a category field is wanted later it arrives with
`Command.category` (the other Stage-2 candidate), typed as what it
actually is rather than borrowed from LSP.

### Q#D2-2 — which prompts get rows?

`pmacs.minibuffer.read` serves many sources, not just M-x: file paths,
buffer names, apropos substrings, settings. Only some have a natural
`detail`.

*My vote: **the field is `Option<String>` per row and the daemon fills
it where it has one.*** Commands get their description; a file-path
prompt leaves it `None` and renders exactly as today. No source is
obliged to invent a detail, and none is prevented from gaining one
later.

### Q#D2-3 — is detail rendering configurable?

*My vote: **no setting.*** §11 grades the registry "partial
(foundation only)"; adding a speculative toggle for a feature nobody
has yet asked to disable is how a registry becomes noise. If somebody
wants it off, that is use evidence and a later one-line addition.

### Q#D2-4 — older frontends — **RESOLVED in rev 2, in §3.1–3.2**

No longer open, and the revision-1 answer was wrong. "Gate the richer
form at `>= 23`" would have **removed the minibuffer entirely** from
every v12–v22 peer, because there would have been only one variant to
gate. Compatibility requires the legacy shape to still exist and still
be sent — hence the additive `MinibufferPromptRows` variant, per-peer
selection, per-variant cache keys, and matched open/close families.

The `CompletionPopup` gate I proposed copying (`daemon-gated >= 15`)
**is** the right precedent for *how to select per peer*; it is not a
precedent for changing a live variant's shape, because that variant was
new when it was gated.

### Q#D2-5 — does this tempt closed-set acceptance? **(a trap)**

The discovery lane's own handoff note warns: **completion is
assistance, not validation** — `resolve_accepted_value` returns the
literal typed text when no candidate is selected, so closed-set
acceptance is unbuilt Rust work.

Richer rows make M-x *look* like a closed set, which invites someone to
make acceptance reject unmatched input. **That is out of scope and
would be a behaviour change**, not a rendering one. Stated here because
the temptation arrives with the feature.

## 6. Verification

- **A command's description reaches the GPU row**, asserted through
  the real prompt path rather than by constructing a message.
- **A v22 peer still receives `MinibufferPrompt`, with its old
  encoding** — the case revision 1 would have broken. Asserted by
  negotiating v22 and observing the legacy variant arrive, **not** by
  observing "no error".
- **A v23 peer receives `MinibufferPromptRows` and NOT the legacy
  variant** — the double-render guard.
- **A round-trip encode/decode of the frozen `MinibufferPrompt`**
  pins its shape, so a later field addition to it fails a test rather
  than silently breaking v12–v22.
- **The cache key is per variant**: a session that opens for a v23 peer
  and a later one for a v22 peer are not suppressed as duplicates of
  each other (§3.2).
- **Close matches open**: a `MinibufferPromptRows` session is closed by
  its own family, witnessed by the popup actually clearing.
- **The TUI renders `name — description` for the selected candidate**
  (§3.4), from the local registry, with **no wire involvement**.
- **TUI clipping preserves the NAME and drops the description** at
  narrow widths — witnessed at a width where both cannot fit, because
  a clipped name is worse than today's bare name.
- **A source with no detail renders exactly as before** — the
  file-path prompt is the witness (Q#D2-2).
- **Typed-but-unmatched input is still accepted** (Q#D2-5) — the
  guard against this lane quietly becoming a validation change.
- **The version-bump discipline**: `ADVERTISED_PROTOCOL_VERSION`
  unchanged at 20, and the tripwire assertions updated **knowingly**.
  Handoff §3 requires the strengthened two-configuration sweep for a
  `PROTOCOL_VERSION` change — `scripts/gate --protocol`, which exists
  precisely for this.

**What this will not prove:** that `Command` carries title or category
(not in scope), or that predicates are evaluated (Stage 3+).

## 7. Not in scope

`Command` gaining title/category/aliases/flags/arg-schema — the
~175-site change, and the lane's next candidate. **A multi-row TUI
chooser** (§3.4) — a new interaction surface and materially larger than
this lane. **Changing `MinibufferPrompt`'s existing shape** — it is
frozen for v12–v22. Predicate evaluation,
which makes commands stop being invocable and needs its own decision at
each call site. Help-layer unification (`src/help.rs` is still
orphaned). The help prefix key — `C-h` is **not** free, since non-kitty
terminals cannot disambiguate Ctrl+Backspace from Ctrl+H (both are
byte 0x08). Closed-set acceptance (Q#D2-5).
