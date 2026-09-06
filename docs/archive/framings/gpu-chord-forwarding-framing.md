# GPU general chord forwarding — framing + as-built

pmacs-gpu withheld command chords (`Ctrl`/`Alt` + a character) from the
daemon by default, forwarding only a hand-maintained allowlist:
`is_search_entry_chord` (`C-s`/`C-r`/…), `is_clipboard_chord`
(`M-w`/`C-w`/`C-y`), `is_minibuffer_open_chord` (`M-x`/`C-x`). So most of
the Emacs keymap — `C-a`, `C-e`, `C-k`, `C-/`, `M-f`, `M-d`, … — was
unreachable in the GUI, and every arc that added a chorded command had to
bolt on another allowlist entry.

**The blocker was gone.** The reason the GUI withheld command chords was
recorded in the code: *"they drive commands and minibuffer flows the GUI
can't render or interact with yet."* The minibuffer now renders (Q#MB1),
so command chords can forward and the three allowlists collapse into one
general rule. This arc was the payoff of the minibuffer arc — a net
simplification (three predicates + three near-identical handler blocks →
one; **−51 lines**), and **no protocol change** (pure GPU input routing;
the wire and daemon are untouched, so only pmacs-gpu rebuilds).

## The rule (Q#GC1)

**Forward any command chord** — `Char`/`Enter`/`Tab` with `Ctrl` or
`Alt` held — to the daemon, exactly as the allowlists did (mark the
optimistic cursor stale, `send_key`, no optimistic local flip: the
search-entry precedent, so a forwarded chord that changes no daemon
state can't wedge the intercept gate). The daemon's keymap resolves it —
the same path the TUI already drives, which forwards *everything*. Once a
forwarded chord opens a prompt / enters a prefix, `dispatch_idle` flips
false and the intercept gate round-trips the rest.

`is_command_chord` (`matches!(key, Char(_) | Enter | Tab) &&
(mods.contains(CTRL) || mods.contains(ALT))`) replaces
`is_search_entry_chord` / `is_clipboard_chord` /
`is_minibuffer_open_chord` — all three were just `Char + Ctrl/Alt`,
subsumed.

## What stays local (Q#GC2)

- **`Ctrl-V`** — OS paste via `arboard` (Q#CM6), intercepted *before*
  the command-chord block. It never reaches the daemon, so in the GUI
  `C-v` pastes rather than running the keymap's `cursor.page-down` —
  a deliberate, pre-existing GUI divergence (the modern paste
  convention); Emacs page-down is `M-v` / the `PageDown` key.
- **`Escape`** — cancels an active intercept, else quits the window (a
  GUI affordance). Not a command chord, so the rule doesn't touch it.
- **`Meta`/`Super`-only chords** (`Cmd-C`, `Super-…`) — *not* command
  chords (no `Ctrl`/`Alt`), so they fall through to `should_forward_key`,
  which withholds them, leaving OS/WM shortcuts (`Cmd-Q`, `Cmd-C` on
  macOS) to the platform rather than consuming them.

## What didn't change (Q#GC3)

- **`should_forward_key` is untouched.** Command chords are caught by
  the new block *before* it, so it still withholds `Ctrl`/`Alt` chords
  (they just never reach it) — its test still holds unchanged.
- **Motion / `Backspace` / `Delete`** keep their existing path (forwarded
  with any modifiers, through the optimistic/defer logic) — the new
  block matches only `Char`/`Enter`/`Tab`, so `C-Left` (word motion)
  still gets its defer-aware handling, not immediate send.
- **The optimistic text path** is unchanged: a command chord's
  `optimistic_crdt_insert` returns `None` (it requires plain modifiers),
  so command chords never optimistic-apply — they round-trip, as before.

## As-built

Landed as framed — a `is_command_chord` predicate + one handler block
replacing the three allowlist blocks, the three helpers removed, the
"withheld" comment refreshed, and the `search_entry_chord` test swapped
for a `command_chord` test (which also asserts the old allowlist chords
plus `C-a` / `M-f` / `C-Enter` are command chords, and that plain text /
`Meta`-only / motion are not). No divergences from the framing; the one
judgment call baked in from the start — leaving `Meta`/`Super` to the OS
and `Ctrl-V` to local paste — is recorded in Q#GC2.

## Categorical bets (held)

- **The daemon already handles arbitrary chords.** The TUI forwards every
  key; the daemon's keymap + `Action::Unbound` path handle bound and
  unbound chords gracefully. Forwarding command chords just made the GPU
  behave like the TUI for them — no daemon-side change.
- **`Char + Ctrl/Alt` was the right cut.** It captures exactly the
  withheld chords, leaves motion's defer path alone, and leaves
  `Meta`/`Super` to the OS.

## Deferred (named, not silently dropped)

- Rebindable / configurable local exceptions (today `Ctrl-V` and
  `Escape` are hard-coded).
- Forwarding `Meta`/`Super` chords for users who bind them in the daemon
  (currently withheld to protect OS shortcuts).
