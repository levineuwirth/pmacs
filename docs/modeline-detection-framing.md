# Modeline language detection — side quest

**Status:** Revision 2, approved for implementation by the user on 2026-07-22.
No implementation was present at approval.
**Base:** `githubsucks/main` at `d5d9b9c`; protocol v18.

## Problem

Language inference currently uses four signals, in order:

1. bundled grammar extension;
2. `pmacs.lsp.filetypes` extension;
3. exact basename;
4. shebang.

That chain cannot classify a deliberately misleading or extensionless file when
its author supplied editor metadata such as `-*- mode: python -*-` or
`vim: set ft=python:`. Mode-system wiring #129 gives the result a persistent
per-buffer home and makes it observable through key dispatch, help, and the
statusline, but no modeline parser exists.

The goal is one bounded, non-executing modeline detector whose result drives the
same initial language used by syntax, LSP, and `Buffer.major_mode`. This is a
language-detection feature, not a general file-local settings system.

## Ground truth

### Detection is duplicated today

`builtin/runtime/syntax.lua::resolve_active_language` and
`builtin/runtime/lsp.lua::buffer_language` independently implement the same
extension → filetype → filename → shebang chain. They already have small but
important differences:

- syntax uses `buf:name()`, preserving historical grammar-by-name behavior for
  pathless buffers;
- LSP requires `buf:path()`, because it cannot construct a URI or project root
  for a pathless buffer;
- syntax pins the grammar in `parse_lang_by_buffer` after dispatch, while the
  public LSP language query re-sniffs mutable shebang text on every call.

Adding the fifth signal to both copies would create a third convention and let
syntax, LSP, mode initialization, auto-pairing, and comment commands disagree.
The implementation must consolidate the actual inference in `syntax.lua` and
leave only the LSP path-eligibility guard in `lsp.lua`.

### Hook order is already useful

`src/editor.rs` loads `syntax.lua` before `lsp.lua`. Their `buffer.after-load`
callbacks therefore run in that registration order:

1. syntax detects the language, initializes the major mode, and dispatches a
   grammar when one exists;
2. LSP resolves the same buffer language and attaches a server when configured.

The modeline result must be computed and pinned by step 1 so step 2 cannot
independently reinterpret the file.

### Major mode and parser language have different later lifecycles

At first load, the detected language is the initial major-mode name. Afterward:

- explicit `pmacs.buffer.set_major_mode` changes dispatch/statusline only;
- syntax and LSP remain attached to their initially selected language;
- `buffer.after-switch` restores views without re-detecting;
- edits do not change the selected grammar.

Modelines follow that same load-time contract. Editing a cookie is not a live
mode or parser switch. Close/reopen re-evaluates it. A future true reload that
fires `buffer.after-load` re-evaluates it as a fresh load; explicit mode
overrides and clears are not reload-persistent, matching the #129 framing.

### Existing Lua reads are sufficient

`BufferIdLua` exposes byte-length and byte-slice operations. Detection can read
bounded prefix/suffix windows without copying the whole buffer or adding a Rust
binding. The rope is byte-addressed, so a bounded slice does not need UTF-8
boundary repair before Lua pattern matching.

### Compatibility references

The supported subset follows the documented, non-evaluating pieces of:

- GNU Emacs, “Specifying File Variables”:
  <https://www.gnu.org/software/emacs/manual/html_node/emacs/Specifying-File-Variables.html>
- Vim, `:help modeline` and `:help 'modelines'`:
  <https://vimhelp.org/options.txt.html#modeline>

Compatibility is deliberately bounded below. pmacs does not become an Emacs
file-variable evaluator or a Vim `:set` interpreter.

## Scope

In scope:

- Emacs `-*- mode: NAME -*-` and mode-only `-*- NAME -*-` cookies;
- Vim/Vi `ft=NAME` and `filetype=NAME` modelines;
- first/last-line scanning with fixed byte and line limits;
- canonical aliases for common external filetype names;
- modeline precedence over inferred path/shebang language;
- one shared, pinned per-buffer language decision;
- initial major mode, grammar, LSP, and language-aware Lua consumers agreeing;
- focused parser and end-to-end acceptance on both Lua backends.

Out of scope:

- Emacs `Local Variables:` tail blocks;
- variables other than Emacs `mode` or Vim `ft`/`filetype`;
- `eval`, Vim commands, option mutation, directory-local variables, or project
  trust prompts;
- Vim `ex:` markers, version predicates, escaped option values, and combined
  dotted filetypes;
- live re-detection after edits, saves, renames, or buffer switches;
- `buffer.after-mode-change`, minor modes, mode-scoped settings, `describe-mode`,
  or session persistence of explicit mode overrides;
- a protocol change or frontend-specific work.

## Decisions

### Q#MD1 — Scan only bounded edge lines

Read at most 8 KiB from each end of the buffer and inspect:

- Emacs: line 1, or line 2 only when line 1 begins with `#!`;
- Vim/Vi: the first five and last five logical lines, matching Vim's default
  `'modelines'=5` behavior.

When the prefix and suffix overlap, deduplicate lines before parsing. Strip one
trailing `\r` so CRLF and LF behave identically. Discard the suffix window's
leading fragment when its line begins before the 8 KiB boundary; that fragment
does not count toward the five complete logical lines counted backward from
buffer end. No truncated candidate is parsed. Detection therefore allocates at
most 16 KiB per fresh load, independent of file size, and an adversarial giant
edge line cannot force a whole-buffer copy.

The Emacs 3000-character tail `Local Variables:` mechanism is a separate parser
with comment-prefix/suffix rules and is excluded.

### Q#MD2 — Recognize a conservative syntax subset

Emacs:

- require a complete pair of `-*-` delimiters on the eligible line;
- accept `mode: NAME` in a semicolon-separated property list;
- accept a mode-only payload such as `-*- Lisp -*-`;
- ignore every property except `mode`;
- when a cookie contains multiple valid `mode` properties, the last wins.

Vim/Vi:

- accept `vim:`, `vi:`, and `Vim:` at line start or preceded by ASCII space or
  tab; uppercase `Vim:` requires literal `set` rather than abbreviated `se`,
  matching Vim;
- accept only exact `ft=NAME` and `filetype=NAME` assignments; `ft:NAME` and
  `filetype:NAME` are not modeline assignment forms and are rejected;
- in the direct form, split option tokens on ASCII whitespace and `:`, so the
  common `vim:ft=python:sw=4:` form yields `ft=python`;
- in the `set` / `se` form (`se` is lowercase-marker-only), end the option
  section at the first `:` and split only the preceding text on ASCII
  whitespace; `vim: set sw=4: ft=python` therefore contains no live filetype
  assignment;
- ignore all other live option tokens rather than interpreting them;
- require that terminating colon for the `set` / `se` form, so a comment suffix
  is never consumed as an option value;
- reject `ex:`, Vim version predicates, and marker substrings embedded in a
  word.

Across all eligible lines, the last valid mode assignment in document order
wins. This matches Emacs's “final defined mode” behavior and ordinary sequential
option assignment. A footer Vim modeline can intentionally override a header
Emacs cookie; conflicting metadata does not depend on Lua table iteration.

### Q#MD3 — Modeline names are data, never code

Trim ASCII edge whitespace, ASCII-lowercase the name, require
`[a-z0-9][a-z0-9+_-]*`, and cap it at 128 bytes. Empty, non-ASCII, control,
whitespace-containing, or overlong values are ignored silently.

The restriction is intentionally narrower than `pmacs.buffer.set_major_mode`,
which continues accepting arbitrary Lua strings for trusted configuration.
Untrusted file content receives no path to control characters, huge statusline
values, Lua evaluation, or option mutation.

### Q#MD4 — Normalize common external names through one alias table

Expose a user-extensible Lua table:

```lua
pmacs.parse.modeline_aliases = {
  ["c++"] = "cpp",
  cxx = "cpp",
  sh = "bash",
  shell = "bash",
  ["shell-script"] = "bash",
  zsh = "bash",
  py = "python",
  js = "javascript",
  js2 = "javascript",
  jsx = "javascriptreact",
  ts = "typescript",
  tsx = "typescriptreact",
  yml = "yaml",
  makefile = "make",
  docker = "dockerfile",
}
```

A normalized name absent from the table passes through unchanged. Users may
add, replace, or remove aliases in `init.lua`; defaults use `or`-style seeding
so preconfigured entries are not overwritten. Alias outputs must satisfy the
same 128-byte token rule before use.

This keeps canonical pmacs names stable without pretending that extension maps
and editor-mode names are the same namespace. Do not strip a trailing `-mode`:
GNU Emacs explicitly specifies the value without that suffix, and silent
stripping would make custom names ambiguous.

### Q#MD5 — Explicit modelines override inference

The final fresh-load order is:

1. modeline;
2. bundled grammar extension;
3. `pmacs.lsp.filetypes` extension;
4. exact basename;
5. shebang.

A modeline is explicit file metadata; the remaining signals are inference. Thus
a `template.txt` containing `vim: set ft=python:` selects `python`, and a
misnamed `.py` file containing `-*- mode: lua -*-` selects `lua` consistently
for mode, parser, and LSP.

This interprets the backlog's “fifth layer after extension → filetype → filename
→ shebang” as a layer added after that work, not as a lowest-priority fallback.
Making explicit metadata lose to a suffix would defeat the feature's primary
use case.

### Q#MD6 — One resolver owns the effective language

`syntax.lua` owns:

- `pmacs.parse.language_from_modeline(buf)`: parse current content and return
  the normalized modeline language or nil;
- the private fresh inference chain;
- `pmacs.parse.buffer_language(buf)`: return the language pinned for this
  buffer's current load, resolving once only for an unseen buffer.

The syntax `buffer.after-load` path forces a fresh inference, records either the
language or an explicit “resolved none” sentinel, then uses that same value for
mode initialization and grammar dispatch. `buffer.after-switch` consumes the
pin and resolves only the pre-existing hidden-buffer case that never received
an after-load event.

`pmacs.lsp.buffer_language(buf)` keeps its current path requirement, then
delegates to `pmacs.parse.buffer_language(buf)`. The active-buffer wrapper stays
unchanged. This preserves pathless LSP behavior while deleting the duplicate
extension/filetype/filename/shebang chain.

The pin also closes an existing shebang inconsistency: editing `#!/bin/sh` to
`#!/usr/bin/env lua` no longer leaves a bash parse tree while making later
auto-pair/comment queries report Lua. Raw parser tests may call
`language_from_modeline`; behavior-driving consumers use the pin.

### Q#MD7 — Initial mode, syntax, and LSP share one value

On `buffer.after-load`, set the buffer's major mode to the freshly resolved
language, including nil when no signal resolves. This replaces the current
“only if nil” guard and makes the already-documented reload contract exact:
a fresh after-load decision replaces an earlier explicit override or clear.

Then:

- dispatch a grammar only if `pmacs.parse._has_language(lang)`;
- let LSP attach only if the same language has a configured server and a real
  file path;
- retain a valid unknown language as the major mode, while silently skipping
  grammar/LSP attachment.

After load, explicit `set_major_mode` remains independent: it immediately
changes mode key dispatch and statusline display but does not rewrite the
pinned parser/LSP language. Switches preserve both values.

### Q#MD8 — Malformed or unsupported metadata is fail-closed and quiet

A malformed marker, invalid name, unsupported form, or truncated candidate
never raises from `buffer.after-load` and never emits an `*errors*` entry. The
detector returns nil and the existing inference chain continues.

A syntactically valid unknown name is different: it is a legitimate major mode
and is pinned, but `_has_language` and LSP config gates prevent a bogus parser
or server launch. This preserves #129's custom-mode capability without
executing file content.

A file can already select a configured language—and therefore which LSP server
pmacs starts—through its extension or a content-sniffed shebang. Modelines add
another bounded language selector within that existing capability class; they
do not introduce automatic execution beyond what current language detection
already permits.

No enable/disable setting is added. The supported input can only select a
bounded string already consumed as passive mode/language identity; it cannot
run hooks or set options. If a future `buffer.after-mode-change` hook makes mode
selection executable, modeline trust must be revisited in that feature's
framing.

### Q#MD9 — No Rust or protocol surface is required

Expected implementation touch set:

- `builtin/runtime/syntax.lua` — bounded parser, aliases, shared resolver, pin,
  and after-load initialization;
- `builtin/runtime/lsp.lua` — delegate language inference while retaining the
  path guard;
- `tests/m4_acceptance.rs` — parser, precedence, lifecycle, and end-to-end
  regression coverage;
- this framing and the side-quest/handoff state documents when the feature
  lands.

No changes are expected in `src/buffer.rs`, Lua bindings, frontends,
`pmacs-protocol`, or protocol version 18.

## Bets

1. **Sixteen KiB of edge text is sufficient.** Real modelines are short; files
   with an 8 KiB first/last candidate line are better treated as malformed than
   copied wholesale during load.
2. **One canonical language should drive mode, parser, and LSP initially.** A
   future distinction between editor mode and parser language needs a real
   consumer and an explicit mapping contract, not accidental divergence.
3. **ASCII-lowercased identifiers cover interoperable modelines.** Trusted Lua
   remains available for arbitrary UTF-8 custom mode names.
4. **Load-time detection is enough.** Live cookie edits would require orderly
   parser teardown, LSP `didClose`/`didOpen`, overlay replacement, mode-change
   notification, and failure rollback; that is not a one-shot detector.
5. **Pathless LSP buffers stay ineligible.** Syntax may still infer from a
   buffer name, but spawning a server without a URI/project root remains wrong.

## Acceptance

All end-to-end fixtures clear `pmacs.lsp.config` unless the case intentionally
observes server selection, so opening a test file never starts a machine-local
language server.

1. **Emacs forms:** first-line property and shorthand cookies resolve; a cookie
   on line 2 resolves only after a shebang; unrelated properties are ignored;
   the last `mode` property wins.
2. **Vim forms:** `vim:`/`vi:` direct and `set` forms resolve `ft` and
   `filetype` in the first/last five lines; CRLF works; `Vim:` requires `set`.
   The direct `vim:ft=sh:et:sw=2:` form resolves `sh`, while
   `vim: set sw=4: ft=python` ignores the assignment after the terminating
   colon. `ft:python` and `filetype:python` resolve nothing in either form.
3. **Boundary rejection:** sixth-line, sixth-from-end, middle-of-file,
   word-embedded, unterminated, truncated, invalid-character, and overlong
   candidates do not resolve and do not log errors. A partial line at the start
   of the suffix byte window is discarded without consuming one of the five
   complete tail-line slots.
4. **Conflict order:** overlapping edge windows are deduplicated and the last
   valid assignment in document order wins deterministically.
5. **Alias behavior:** seeded aliases map `sh`/`zsh → bash`, `c++ → cpp`,
   `js2 → javascript`, `tsx → typescriptreact`, and `docker → dockerfile`; a
   user override wins; an invalid alias output is ignored.
6. **Explicit precedence:** a `.py` file with a Lua modeline yields `lua` from
   `pmacs.parse.buffer_language`, `pmacs.lsp.active_buffer_language`, the parse
   tree, and `pmacs.buffer.major_mode`.
7. **Unknown valid mode:** a `.txt` file with `mode: prose` receives major mode
   `prose`, creates no parse view, starts no LSP server, and produces no error.
8. **No-modeline regression:** extension, filetype, filename, and shebang cases
   retain their present precedence and outputs.
9. **Shebang pin regression:** open an extensionless `#!/bin/sh` file, replace
   its shebang with `#!/usr/bin/env lua`, and assert the parse tree and
   `pmacs.lsp.buffer_language` both remain `bash`.
10. **Pinned modeline lifecycle:** changing a loaded modeline does not change
    the pinned language, parser, or major mode; switch-away/back remains stable;
    close and reopen re-evaluates the changed on-disk cookie.
11. **Explicit override independence:** `set_major_mode` after load changes
    dispatch/statusline but not `pmacs.parse.buffer_language`; switches preserve
    the override.
12. **Pathless preservation:** syntax-by-buffer-name behavior remains, while
    `pmacs.lsp.buffer_language` still returns nil without a backing path.
13. **Backend parity:** focused acceptance passes with default LuaJIT and
    `--no-default-features --features lua54`; protocol remains v18.
