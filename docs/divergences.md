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
