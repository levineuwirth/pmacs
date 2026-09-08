# GUI zoom — QoL Stage 2

**Status: revision 5 — APPROVED, IMPLEMENTED, in review as PR #220.**
Revision 5 closes a gap review found in the shipped code: the
round-trip guarantee in §3.1 needed the step to be **quantized where it
is used**, because the registry cannot enforce precision (§3.2). Q#Z1 approved as
**(c)** (configured base, `None` preserved) and Q#Z2 as **additive**.
Revision 4 fixes a self-contradiction in §5a1: the specified parser
`^(%d+)$` rejects the newline-terminated format specified beside it.
Whole-file `^(%d+)\n$`, with a verified case table and a valid-restore
witness. Revision 3 added the contract details a second pass asked for: §1 and §7
no longer claim a keybinding this stage does not ship, §3.1 bounds and
quantizes both settings, and §5a1 defines the persisted format and what
corrupted state does. Revision 2 answered four review findings that revision 1 got wrong or
left out: Z3 was **not implementable as written** (§4), Z4 had **no
working restore seam** (§5), every size write **silently clears a
configured family** (§5a), and reset/precedence semantics were
unspecified (§5b). Reported from daily-driver use
alongside Stage 1: `Ctrl +/-` has *no effect whatsoever* in the GPU
frontend. Stage 1 (#219) fixed what zoom did to the TUI; this stage is
the other half — making the GUI zoom at all.

**Most of this already exists**, which is why it is a short stage. It
is not, however, a trivial one: relative zoom needs a starting point,
and the daemon is deliberately built never to know one (§3).

---

## 1. What is already built

The whole mechanism ships today and is exercised by tests:

| piece | where | state |
|---|---|---|
| Scale that drives **everything** | `FontMetrics { scale, advance_ratio }`, `pmacs-gpu/src/main.rs:129`+ | code size, line height, status band, divider, menu rows, minibuffer dropdown, gutter advance all derive from it |
| Daemon-side preference | `src/font_pref.rs` — family + `size_centi_px` + monotonic `epoch` | written by `pmacs.gpu.set_font`, read per-frame by the producer |
| Wire relay | `InstanceMessage::FontFacts`, protocol v17 | bufferless; v16 peers get none |
| Frontend application | `apply_font_facts`, `pmacs-gpu/src/main.rs:8132` | one transaction: validate, resolve family, re-metric all seven buffers, reshape at retained scroll, re-follow caret |
| Getter | `pmacs.gpu.font()` | fresh table, `size` in logical px, key absent when unset |
| Validation | `600..=7200` centi-px (6.0–72.0 px), checked at both ends | Lua range-check is UX; the frontend re-checks because it is wire input |

There is even a byte-identity test that `(None, None)` reproduces the
never-set frame exactly.

**So this stage adds no rendering work.** It adds **commands**, a
starting point, and remembering — **and deliberately no default
keybindings**, per Q#Z3 (§4): the keymap cannot express "GPU frontends
only", and neither a protocol variant nor a rushed keymap change is
worth a default chord.

---

## 2. Q#Z1 — where does a *relative* zoom start? (the real question)

`src/font_pref.rs` is explicit:

> *"The daemon relays a PREFERENCE: it never learns metrics, advances,
> or what resolves (the no-pixels invariant); the frontend owns
> resolution and every pixel consequence."*

And `size_centi_px: None` is **a real state, not an absence** — "the
frontend's built-in default", which Q#TH7 established must never be
inferred from silence.

Zoom-in is `current + step`. On a fresh session `current` is `None`,
and the daemon cannot ask what the frontend resolved. Three ways out:

- **(a) Hard-code the frontend's default (16.0) as the origin.** One
  line, and it puts a pixel constant on the daemon side — the exact
  thing the no-pixels invariant forbids. It also silently breaks for
  any frontend whose default differs, which is the situation the
  invariant exists to allow.
- **(b) Always send an explicit size**, sourced from a config setting
  defaulting to 16.0. Simple, but it **destroys the `None` state for
  everyone**: a user who never zooms now gets an explicit size, the
  frontend's own default query never runs, and Q#F5's always-shipped
  all-default baseline stops being reachable.
- **(c) A configured *base*, with `None` preserved until the user
  actually zooms.** `ui.gpu-font-size-base` (logical px, default 16.0)
  is the documented origin for the *first* zoom step. Until a zoom
  happens the preference stays `None` and the frontend's default is
  used exactly as today.

**Recommendation: (c).** It is the only one where the daemon still
infers nothing — the base is a *user-facing preference about zooming*,
not the daemon guessing a frontend metric. A user whose frontend
default is not 16.0 changes one setting, which is a real answer rather
than a broken assumption. And the untouched path is byte-identical to
today, so the existing `(None, None)` identity test keeps its meaning.

**Q#Z1 — DECIDED: (c)**, on review.

---

## 3. Q#Z2 — step shape

Additive (`+1.0 px`) or multiplicative (`×1.1`)?

**DECIDED: additive**, on review — via `ui.gpu-zoom-step` (logical px,
default 1.0).

### 3.1 Both settings are validated, and the bounds are not cosmetic

The registry validates at `define` time (`min`/`max`, as
`autosave.interval-ms` does), so both settings are constrained rather
than checked at use:

| setting | bounds | why |
|---|---|---|
| `ui.gpu-font-size-base` | **6.00 – 72.00** logical px | exactly the wire range (`600..=7200` centi-px). A base outside it could never be sent, so the first zoom step would fail from a value the user was allowed to set |
| `ui.gpu-zoom-step` | **0.01 – 66.00** logical px, strictly positive | see below |

**The step's lower bound is the quantizer.** `validate_font_size`
range-checks the original value and then rounds to the nearest
hundredth, so a step below `0.01` quantizes to **zero** — "zoom in"
would do nothing, forever, with no error anywhere. `0.01` is one
centi-pixel, the smallest representable step.

**Zero and negative are excluded for a stronger reason than tidiness.**
A negative step **inverts** the commands: `gpu.zoom-in` would shrink.
That is not a malfunction the user can diagnose from the outside — the
command does something coherent, just the opposite of its name.

**The upper bound is the range span** (72.00 − 6.00). A step larger
than the whole domain can only ever clamp or be rejected, so permitting
it buys a setting that cannot be used.

### 3.2 The bounds are not sufficient on their own

Review found the gap in the implementation: `ConfigKind::Number`
validates **finiteness and bounds and nothing else**
(`src/config_registry.rs`), and `on_change` listeners are notified
*after* a value is stored — they cannot veto. So `0.015` is a
perfectly settable step, and nothing in the registry can refuse it.

Used raw it breaks the guarantee below. Each operation rounds
independently, and both intermediates land on an **exact tie** —
`16.015` and `16.005` are `1601.5` and `1600.5` centi-pixels — which the
half-up quantizer sends **up**. Half-up is not symmetric under negation:
rounding up on the way in adds half a centi-pixel, rounding up on the
way out adds another, so the two errors accumulate instead of
cancelling:

```
step 0.015:  16.00 -> 16.02 -> 16.01     round trip broken
step 0.37 :  16.00 -> 16.37 -> 16.00     round trip holds
```

The original test used `0.37` — centi-pixel representable — so it could
not reach this.

**Resolution: quantize the step and the base where they are used.** Not
a workaround: sizes live in integer hundredths end to end, and
`validate_font_size` already range-checks the original and then rounds
to the nearest hundredth. Rounding the step is that same operation one
level up. A step of `0.015` is not a finer step in this domain, it is
`0.02` written imprecisely.

Enforcing at `set` time was considered and rejected: the registry
cannot express it, and a validating wrapper is bypassed by a direct
`pmacs.config.set` — the same seam `autosave` documents about its own
`interval_ms` wrapper. Quantizing at the point of use cannot be
bypassed. Both settings say "quantized to hundredths" in their
`description`, so `describe-setting` shows it.

**These bounds and that quantization are what make the round-trip claim
true.** "n steps in,
then n steps out, returns to exactly the starting value" holds because
the step is centi-pixel representable and addition is exact in that
domain — *provided no clamp occurred*, which is why §6 requires an
out-of-range zoom to leave the preference **unmutated** rather than
pinning it to the boundary. Pinning would silently break the round trip
at the edges, which is precisely where a user is most likely to be
stepping back and forth. The domain is integer hundredths of a pixel over
6.0..=72.0 — a small, bounded, quantized range where additive steps are
predictable and land on round numbers. Multiplicative stepping
accumulates rounding through the quantizer and makes "two in, two out"
fail to return to where you started, which is the property users
actually notice.

---

## 4. Q#Z3 — keys. Revision 1 was not implementable

Revision 1 said "do not install the binding on a non-GPU frontend."
**There is no such thing.** `keymap_stack::Scope` is exactly
`Buffer(BufferId) | Mode(String) | Global`, and neither `resolve()` nor
`keymap_tree` receives any frontend identity or capability. A global
binding is global — it would capture `C-+` / `C--` in the TUI, which is
the outcome §4 claimed to avoid.

Nor is there a way for the GPU to ask for a command by name:
`FrontendEvent` is `Key | Mouse | Resize | Paste | FocusGained |
FocusLost | Detach | CrdtOp`. There is **no command-invocation
variant**, so "the GPU intercepts the chord locally" cannot reach the
daemon-side preference that owns the value and its persistence.

So there are three real options, and none is free:

- **(A) Capability-aware keymap resolution.** Teach `Scope` — or
  `resolve()`'s inputs — about the requesting frontend. Correct and
  general, and it would serve every later frontend-specific binding.
  But it touches the keymap core, every resolve call site, and the Lua
  `bind` surface. That is its own lane, not a step inside a zoom lane.
- **(B) A new `FrontendEvent` for it.** Narrow and direct, but it is a
  **protocol addition** — schema bump and negotiation — which §8
  explicitly scopes out, and which buys a keybinding rather than a
  capability.
- **(C) Ship commands now; bind later.** `gpu.zoom-in` / `-out` /
  `-reset` exist and work from `M-x` and from `init.lua`, where a user
  who runs the GPU can bind them themselves in one line. No core
  change, no protocol change, no half-built capability.

**Recommendation: (C), with (A) recorded as the follow-on it implies.**
The value the report asked for is *zoom working at all*; the default
keybinding is the smaller half, and buying it with either a protocol
variant or a rushed keymap change trades a large permanent surface for
a small convenience. **(A) is the right eventual answer** — "this
binding applies to frontends with capability X" is a question this
project will keep asking — and it deserves its own framing rather than
being smuggled in here.

**What must NOT happen** is a global binding plus a command that
reports "not applicable" in the TUI. That is the Q#P3 fall-through
lesson and #217's flat-panel `TAB` finding: capturing a key to deliver
an apology removes a working behaviour — and here the working
behaviour is *the terminal's own zoom*, which is exactly what the user
is pressing the key for.

*Tempting and rejected:* "terminals swallow `C-+` anyway, so a global
binding is harmless in practice." Most do. Relying on most is how a
binding becomes a bug report from whoever runs the terminal that
doesn't.

## 5. Q#Z4 — remembering it, and the seam revision 1 did not have

The config registry has **no write-back** — settings are declared, not
saved. The project's remembered-state mechanism is `pmacs.state.read` /
`write` / `available` (`docs/archive/framings/persistence-framing.md`), used by
`saveplace` and `recentf`. The split is the right one:

- **`pmacs.config`** — what you *declared*: base size, step.
- **`pmacs.state`** — what you *arrived at*: the current zoom level.

### 5.1 Reading at module load is inert, and revision 1 assumed it was not

`EditorState::new()` / `open()` load the builtins and run `init.lua`.
**`install_state_dirs()` runs after that** — `src/editor.rs:3838` on
the local path, `src/daemon.rs:480` on the daemon path. A
`pmacs.state.read` at Lua module load therefore returns nothing, always.

`saveplace` and `recentf` never hit this because **both read lazily**,
inside functions called long after startup (`recentf.lua:27` in
`load_list`, `saveplace.lua:33` behind its `available()` guard). **Zoom
cannot be lazy**: its whole purpose is to apply with no user action,
before the first frame the GPU paints. It is this project's **first
eager state consumer**, which is why the seam has to be named rather
than assumed.

### 5.2 The seam

Restore must run **after `install_state_dirs()` on both startup paths**.
Neither existing post-install hook qualifies: `restore_desktop_if_armed`
is local-only by design (Q#DS9 keeps desktop restore out of the daemon,
which has a layout per attached frontend and none at construction).

**Recommendation: restore inside `install_state_dirs()` itself**, at its
end. Both paths already call it, exactly once, and it is by definition
the moment state becomes readable — so the restore cannot be ordered
wrongly and cannot be forgotten by a future third startup path. The
alternative, a new call added beside both existing call sites, is two
places a fourth path can miss.

**This must be tested against production ordering, not a direct call.**
A test that calls the restore helper itself proves nothing about when
it runs — the same shape as the `prepare_startup` note at
`src/editor.rs:3820`, where splitting the sequence was what kept a
deleted call from leaving every direct-call test green while shipping
nothing. The witness asserts that a state file written before startup
is reflected in `pmacs.gpu.font()` after the real startup sequence.

### 5a. Every size write must preserve the family

`pmacs.gpu.set_font` replaces **both** fields unconditionally
(`src/lua_bindings/mod.rs`: `pref.family = family; pref.size_centi_px =
size_centi_px;`). So `set_font { size = 18 }` **silently clears a
configured family** — a user with `family = "Iosevka"` in `init.lua`
loses it the first time they zoom, and gets it back only by restarting.

Zoom must therefore read the current family via `pmacs.gpu.font()` and
pass it back with every write. **Pinned by a custom-family → zoom →
`FontFacts` case** asserting the relayed message still carries the
family. This is a property of the setter's whole-message semantics, not
a zoom bug, and the test belongs with zoom because zoom is what makes
it reachable.

### 5a1. The persisted format, and what corrupted state does

**Format**, following `saveplace` and `recentf` rather than inventing
one: a single line of plain text holding the size in **centi-pixels**
as decimal digits, newline-terminated.

```
1800
```

Centi-pixels, not logical px, because that is the wire unit and the
already-quantized one — writing `18.0` would reintroduce a
float-parsing step and a second place for rounding to disagree with
`validate_font_size`.

No version prefix. The existing state files carry none, and a single
integer has no forward-compatibility question that a prefix would
answer; a future format change can be detected by the strict parse
below failing, which lands in exactly the same handling as corruption.

**Only the size is persisted.** Not the family — that is declared
config, and §5b's precedence rule applies to size alone.

**Validity** is two checks, both required:

1. **Strict whole-file parse:** `^(%d+)\n$` against the **entire file
   contents**, not against a line pulled out of it.

   Revision 2 specified `^(%d+)$`, which **contradicts the
   newline-terminated format two paragraphs above it**: in Lua, `$`
   anchors to end-of-subject, so `("1800\n"):match("^(%d+)$")` is
   `nil` and the parser would reject every file it had itself written.

   Borrowing `saveplace`'s `gmatch("([^\n]+)")` line iterator fixes
   that but introduces a different hole — it happily returns the
   **first** line of a multi-line file, so trailing garbage would be
   accepted silently. That is tolerable for `recentf`, where each line
   is an independent entry and skipping a bad one loses one path. It is
   not tolerable here, where the file *is* one value and extra content
   means the file is not what we wrote.

   Whole-file matching closes both, verified against Lua 5.4:

   | contents | `^(%d+)$` | `^(%d+)\n$` |
   |---|---|---|
   | `"1800\n"` — what we write | **nil** ✗ | `1800` ✓ |
   | `"1800"` — no trailing newline | `1800` | nil ✓ |
   | `"1800\n1900\n"` — multi-line | nil | nil ✓ |
   | `"18.0\n"` — decimal | nil | nil ✓ |
   | `" 1800\n"` — leading space | nil | nil ✓ |
   | `"\n"` — empty | nil | nil ✓ |

   The multi-line case needs **no separate line-count check**: the
   trailing `$` after `\n` already requires the file to end there.
2. **Range:** the parsed value is within `600..=7200`. A syntactically
   fine but out-of-range number — a hand-edited file, or a file written
   by a future version with a wider range — must never reach
   `set_font`, which would reject the whole message and leave the user
   with neither the saved zoom nor an explanation.

**Malformed, out-of-range, empty, or multi-line state is treated as
absent.** Startup proceeds exactly as if nothing had been saved: the
`init.lua` size applies if there is one, otherwise `None`.

**Silently, and deliberately so.** It matches the precedent — both
existing consumers skip unparseable lines without comment — and the
failure is already self-evident and self-healing: the user sees their
zoom did not restore, and the next zoom rewrites the file correctly.
A startup diagnostic for a recoverable, self-announcing condition is
noise on every launch for a problem that fixes itself on the next
keystroke.

**The corrupt file is not rewritten on read.** It is left alone until
the next successful zoom overwrites it, so a user who wants to look at
what went wrong still can. Truncating it at startup would destroy the
only evidence of a bug we would then have no way to reproduce.

### 5b. Reset, clearing, and precedence

Revision 1 left all three unspecified. As recommended in review:

- **Reset returns size to `None`**, not to `ui.gpu-font-size-base`.
  `None` is the real "frontend's own default" state (Q#F5/Q#TH7); the
  base is only the *origin for the first step*. Resetting to the base
  would send an explicit size that merely happens to equal the default,
  making the untouched state unreachable once a user has ever zoomed —
  and quietly breaking the `(None, None)` identity property.
- **Reset clears the saved zoom state**, so it does not resurrect on
  the next launch. A reset that leaves the state file behind means
  "reset until restart", which is not what the word says.
- **Valid saved state wins over an `init.lua` size.** The saved value
  is a later, deliberate user action; the init value is the standing
  default it was chosen against. This is the same precedence
  `saveplace` already applies — a remembered position overrides where
  opening the file would otherwise land.
- **The family is retained throughout**, per §5a: precedence applies to
  *size only*, and an `init.lua` family is never overridden by restored
  zoom state, because zoom state does not carry one.

## 6. Verification

- **No binding is installed at all** (Q#Z3 = C), so the witness is that
  `C-+` / `C--` resolve to nothing in a default TUI session — the
  terminal's own zoom is left alone.
- **Restore runs at the production seam.** Asserted through the real
  startup sequence, not by calling the helper directly: a state file
  written beforehand must be visible in `pmacs.gpu.font()` afterwards,
  on **both** the local and daemon paths.
- **A zoom preserves a configured family** — custom family → zoom →
  the relayed `FontFacts` still carries it (§5a).
- **Reset returns `None`, not the base**, and clears the saved state;
  a subsequent launch is back to the frontend's own default.
- **Saved state beats an `init.lua` size, and never its family.**
- **Config bounds are enforced at `define`** — a zero, negative, or
  sub-centi-pixel step is refused by the registry, not discovered as a
  zoom that does nothing or runs backwards (§3.1).
- **A valid newline-terminated file restores.** `"1800\n"` — exactly
  the bytes the writer produces — must round-trip through the real
  startup sequence. This is the case revision 2's own parser would have
  rejected, so it is asserted rather than assumed.
- **Malformed, out-of-range, empty, multi-line, and
  missing-trailing-newline saved state each behave as absent**, and the
  file is left intact for inspection (§5a1). The multi-line case is
  pinned specifically: it is the one a line-iterator parser would have
  accepted.
- **The round trip holds only where no clamp occurred**, so the
  out-of-range case asserts the preference is left **unmutated** rather
  than pinned to the boundary.
- **The round trip holds for a step that is NOT representable** —
  `0.015`, which the registry accepts and which the original `0.37`
  case could not reach. Bitten: with the raw value it lands on `16.01`
  instead of `16.00` (§3.2).
- **Zoom from the untouched state uses the configured base**, not a
  hardcoded 16.0 — bitten by changing the base and asserting the first
  step follows it.
- **Clamping at both ends of `6.0..=72.0`** reports rather than
  silently pinning, and a zoom that would leave the range does not
  mutate the preference at all (the `apply_font_facts` whole-message
  rejection precedent).
- **`n` steps in then `n` steps out returns to the exact starting
  value** — the property that fails under multiplicative stepping and
  the reason Q#Z2 recommends additive.
- **The untouched path stays byte-identical**: with no zoom performed,
  the existing `(None, None)` identity test must still hold.
- **Persistence round-trips**, and writes nothing when
  `pmacs.state.available` is false.

---

## 7. Coherence impact (§20 requirement)

**Concern served: `COHERENCE.md` §16, Productize the Semantic Frontend
Architecture**, which names **frontend-specific typography** in its own
list of properties the semantic protocol should make visible as a
product advantage. A GPU frontend that cannot change its own type size
is that concern under-delivered.

**No scorecard change.** Row 16 reads **Strong** (`COHERENCE.md:112`);
this neither earns nor forfeits it — it uses the mechanism the row
already credits. Per §25 the row moves only with a PR that changes what
it asserts.

- **Journey steps touched:** none.
- **Interaction islands added:** none, and **no keybinding either**.
  Revision 1 said "an ordinary global keymap binding, scoped by
  frontend capability"; no such scoping exists (§4), so this stage
  ships commands only, reachable from `M-x` and bindable by the user in
  `init.lua`. Capability-aware binding is deferred to its own framing
  and named in §8.
- **Config registry:** **yes, two settings** —
  `ui.gpu-font-size-base` and `ui.gpu-zoom-step`. This is the "place
  for GUI settings" the report asked for; it already exists and this
  stage is its next adopter.
- **Background-work attribution:** none.

---

## 8. Not in scope

Long lines (Stage 3). **Capability-aware keymap resolution** — Q#Z3's
option (A), which this stage defers rather than half-builds, and which
should be its own framing because it decides how every future
frontend-specific binding is expressed. A default keybinding for zoom,
which waits on it. Per-buffer or per-window zoom — this is one
global preference, as `font_pref` already is. Family selection, which
`set_font` already exposes and no one has asked to bind. Any change to
`FontFacts`, the protocol, or `apply_font_facts` — this stage drives
the existing mechanism and adds nothing to the wire. Config write-back,
which does not exist and which Q#Z4 deliberately routes around rather
than inventing.
