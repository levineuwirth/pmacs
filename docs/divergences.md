# pmacs declared divergences

A frontend-only capability, or a deliberate difference between the two
frontends, ships with an entry here stating what diverges, why it was
accepted, and what removes it. An entry is removed when its condition is
met.

The register has no line cap, and that is the point of its being a file
of its own: it grows with every capability that ships ahead of parity
and shrinks as conditions are met, which is the opposite law from
`docs/invariants.md`, whose rules must stay short enough to be read
whole. Under one cap a growing register squeezes the rules.

- **Frontend color slots.** A frontend's overlay color is keyed by the
  connecting peer's Unix uid, so the same user reconnecting keeps its
  color. The daemon reads that uid with `SO_PEERCRED` (Linux and
  Android); elsewhere `peer_uid` returns `None` and the slot falls back
  to the `FrontendId`, stable within a connection and not across one. A
  token the frontend sent instead would let any peer claim another's
  identity on a socket the kernel alone authenticates, so the
  degradation was accepted. Removed when the daemon carries a session
  identity of its own; until then the acceptance asserts equality only
  where the credential exists, and prints both colors where it does not.
- **Line wrap.** Under `ui.line-wrap = "wrap"` both frontends wrap at
  the character. The GPU frontend could wrap by word through
  cosmic-text's Unicode line breaking (UAX #14) and the grid frontend
  cannot match those breaks without a UAX #14 dependency, so a grid
  whitespace wrap would give approximate parity, which is worse than an
  honest difference; the ruling chose character wrap on both and
  accepted that GPU users lose word wrap. Under `"truncate"`, text past
  the edge is reachable by moving the cursor in the grid frontend and
  not yet reachable in the GPU frontend. Removed when a word-wrap mode
  ships on both frontends with the difference in breaking rules stated,
  and horizontal reach of truncated text exists on the GPU.
- **Zoom chords on a grid frontend (D22).** `C-+` and `C-=` zoom in,
  `C--` out and `C-0` resets, as **global** bindings, so a grid frontend
  reaches them too and the terminal's own zoom chords are shadowed while
  pmacs runs. The keymap cannot express "GPU frontends only":
  `keymap_stack::Scope` is `Buffer | Mode | Global` and carries no
  frontend identity, and `FrontendEvent` has no command-invocation
  variant, so a GPU frontend cannot ask for a command by name either.
  The alternative was leaving the zoom commands reachable only through
  `M-x`, which is not a zoom control. What a grid frontend gets is
  therefore an answer instead of silence: the commands change the GPU
  font preference --- live for an attached GPU session, and at the next
  one otherwise --- and the status line names the GPU font and the new
  size, `zoom: GPU font 17.00 px (applies to GPU sessions)`, or `zoom:
  GPU font back to its default (applies to GPU sessions)` on a reset.
  The line does NOT distinguish "now" from "next session", which is a
  shortfall against what this entry asks and not a thing it delivers: no
  frontend-kind fact is reachable from Lua (`pmacs.frontend.id()`
  returns an id and nothing else), and adding an accessor was rejected
  as core surface E1 does not authorize. A grid user is therefore told
  which sessions the change applies to, never when it took effect.
  `Q#Z3` (ship no bindings) and `Q#GA8` are recorded as overruled for
  this entry's duration. Removed when capability-aware keymap
  resolution lands, at which point the chords bind on GPU frontends
  only and the grid frontend keeps its terminal's zoom.
