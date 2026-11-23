# The fake LSP server's modes

Tests drive language-server behaviour through the fake server, never a
real one; `docs/invariants.md` (## LSP) carries that rule and points
here. This file is reference, not rule, so it has no line cap: it moved
out of `docs/invariants.md`, whose 300 lines are kept for the rules a
change must not break.

The fake server `src/bin/pmacs_fake_lsp.rs` is selected by
`PMACS_FAKE_LSP_MODE`. The binary's module doc describes each shape and
is authoritative; this list is a catalogue for finding one, and a copy
here gone stale covers the shape next to the defect.

- Capability modes: `fullonly`, `rangeonly`, `rangeonly16`,
  `semantichold`, `incremental`, `incremental8`, `didsave`,
  `didsavenotext`, `sighelp`, `prepare`, `preprefuse`, `rename`,
  `inlaybounds`, `inlayrefresh`, `semantictokensrefresh`,
  `applyeditplan`, `resourceops`, `posecho`, `defenv`, `wsconfig`,
  `rooturi`, `leanprogress`.
- Failure shapes: `crash`, `crashafteranswer` (with
  `PMACS_FAKE_LSP_ERROR_CODE`, its one answer an error), `crashlater`
  (each generation's lifetime from `PMACS_FAKE_LSP_LIFETIMES_MS`),
  `crashonmarker`, `abortonopen`, `error`, `contentmodified`,
  `clientfault`, `garbage`, `silent`, `stopanswering`.
- Any mode: a publish plan (`PMACS_FAKE_LSP_PUBLISH_PLAN`), a result
  plan (`PMACS_FAKE_LSP_RESULT_PLAN`, held by
  `PMACS_FAKE_LSP_RESULT_HOLD_MS`), and the capabilities and message
  sinks (`PMACS_FAKE_LSP_CAPABILITIES_SINK`,
  `PMACS_FAKE_LSP_MESSAGE_SINK`).
- Watchers: `filewatch` (`RelativePattern` `**/*.txt`), `filewatchabs`
  (absolute glob), `filewatchflat` (no `**/`), `filewatchbare` (bare
  relative), `filewatchrereg` (one id twice), `filewatchjoin`,
  `filewatchretire`.
