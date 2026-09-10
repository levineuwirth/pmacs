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
- **Zoom chords, Ctrl+wheel and the macOS Cmd chords (D22).** `C-+`
  and `C-=` zoom in,
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
  Two GPU-only islands ride the same entry. Ctrl+wheel in the GPU
  frontend sends the zoom chords, one per whole notch, and scrolls
  nothing; the grid frontend has no wheel modifier to give. On macOS
  the GPU frontend translates the six standard Cmd chords --- Cmd-C,
  -V, -X, -Z, -S and -A --- to their Ctrl equivalents before the
  keymap sees them, so they do whatever `C-c`, `C-v`, `C-x`, `C-z`,
  `C-s` and `C-a` do in pmacs (`C-c` and `C-x` are prefix keys, `C-v`
  is the OS paste), and every other Super chord stays withheld from the
  daemon; a terminal on macOS never sees Cmd at all, so the grid
  frontend cannot match it. `Q#Z3` (ship no bindings), `Q#GA8` (no
  frontend-only bindings) and `Q#S1-7` (Super chords withheld) are
  recorded as overruled for this entry's duration. Removed when
  capability-aware keymap resolution lands, at which point the zoom
  chords bind on GPU frontends only, the grid frontend keeps its
  terminal's zoom, and the Cmd table becomes a frontend-scoped binding
  the keymap can see and `describe-key` can name.
- **Vertical scrollbar (E3.1).** The GPU frontend paints a track and
  thumb in its right gutter, draggable, with a press on bare track
  paging one screenful toward it; the grid frontend has none, and
  gets none, because a terminal's scrollbar is the terminal
  emulator's and a second one painted in cells would compete with
  it. The two frontends therefore disagree about what a document
  taller than the window looks like: the GPU shows how far down it
  is and where, the grid says so only in the mode line's `Top` /
  `Bot` / percentage readout, which is the same fact at a coarser
  resolution rather than a missing one. Removed when the grid
  frontend gains a cell-drawn scroll indicator, or when the mode
  line's readout is ruled sufficient and this entry becomes a
  statement of intent rather than a shortfall.
- **Own-caret current-line wash (E3.3).** Under a set
  `ui.current-line` face the GPU frontend washes the line its own
  caret is on; the grid frontend does not, and neither does the
  daemon's semantic projection, which deliberately emits no
  `CurrentLine` decoration because deriving one would force a
  whole-buffer line table on every frame. So the face is honored by
  one frontend and inert in the other, and a user who sets it in
  `init.lua` sees it only in the GUI. The face was chosen over a
  `pmacs.config` setting precisely because it is the channel that
  exists: every config-derived GPU preference on the wire is its own
  typed variant and each took a wire phase, while a face is one more
  name in a `Vec<ThemeFace>` an existing variant already carries.
  Peer presence is unaffected and already washes other frontends'
  lines on both. **Absent means off, and it is absent in every bundled
  configuration**: the word `theme` occurs zero times under `builtin/`,
  so no shipped configuration defines `ui.current-line` or any other
  face, and a user reaches the wash only through `pmacs.theme.merge`
  with a name that appears in no user-facing document. (The only `ui.*`
  names under `builtin/` are config keys, command names, and one
  statusline segment naming `ui.modeline` as the face to render *in* —
  a reference, not a definition.) That is exactly the shape of the
  thirteen faces registered before this one, so it is not this row's
  defect; it is stated here because the audit's F12 complaint — the
  highlight is gone — stays true for a user who does not edit a theme,
  and a row being met is a different fact from a complaint being
  answered. Removed when the grid frontend paints the same wash from
  the same face, which needs no wire work — only a decoration the grid
  renderer synthesizes locally, as this one is.
- **Search-results preview (E4.4).** In `*search-results*`, `n` and
  `p` show the match under the cursor in the *other* document window
  --- split off the results when there is none --- and hand focus
  straight back to the results, Emacs's next-error-no-select. The
  daemon does this for both frontends identically; what differs is
  what each can show. The grid frontend paints every window in the
  layout, so the preview is visible beside the results. The GPU
  frontend paints one document window and the bottom panel (audit
  §3.1's blocker 9: `State.buffer` is single and the panel band is the
  only second region), so there the other window exists in the
  daemon's layout and is never drawn: `n` and `p` move the match under
  an invisible cursor, focus returns to the results, and the user sees
  the results alone until RET visits in place. It is the same
  invisibility `C-x 2` already has on the GPU (audit §6.1: "`C-x 2`
  already splits the GPU's view invisibly"), reached by a new route;
  nothing here makes it worse, and the preview asks for no chord the
  grid does not also have. Removed when E11 puts `WindowTree` and
  `WindowFacts` on the wire and the GPU paints more than one document
  window, at which point the preview is visible on both and this entry
  becomes a statement that the two frontends agree.
