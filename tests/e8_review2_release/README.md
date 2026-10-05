# E8 review round 2 — release-build probes

Behavioural probes against PR #315's head `57a980e`, run on the release
pair built from that head's exported tree (`git archive 57a980e`, then
`cargo build --release` of `pmacs`, `pmacs-gpu` and `pmacs-parse-unit`,
each in a command of its own, into `~/build/e8-review2/rel-target`:
`pmacs 1.1.0`, `pmacs-gpu 1.1.0 (protocol v26)`, `pmacs-parse-unit
1.1.0 protocol 3`). Not a cargo target: the drivers are scripts, and the
probe projects are stored as `.txt` so cargo never mistakes them for
packages (copy them to `Cargo.toml` and `src/main.rs` to use them).
Paths are the laptop's.

The cargo-run probes beside this directory are
`tests/e8_review2_real_server_probes.rs` (rust-analyzer, in-process: the
TUI's dispatch and `paint_frame`, and the semantic producer's frame),
`tests/e8_review2_offset_probes.rs` (the label-offset parse, no server)
and `tests/e8_review2_probes.rs` (a real daemon, the fake server).

## Drivers

Copies of fix round 1's, themselves review 1's, with review 2's paths:
`run-gpu.sh <name> <file> <script>` (the release GPU's
`--headless-probe` under `PMACS_GPU_PROBE_ACTION=popup`) and
`tui_drive.py <name> <file> <steps.json>` (the release `pmacs --attach`
in a 120x40 tmux pane; `askesc` waits for the grid's underline in the
SGR capture). `marked.py` prints each capture's marked text.

## Runs and what they showed

`proj-both` (non-ASCII identifiers whose byte reading of rust-analyzer's
UTF-16 label offsets is also a set of whole runs), GPU script
`gpu-both.script`, TUI `steps-both.json`. Both frontends mark the same
wrong text: `f(1, |)` marks `u8, a` (GPU `active=0:13:18`, 5 glyphs),
`f(3|` marks `名前` (`0:5:11`), `g(1, "x", |)` marks `&str,`
(`0:26:31`) where the parameter is `a: u8`, and
`长度计算::<u8>(1, |)` marks `a: i32` (`0:23:29`), the first parameter,
where the second, `b: i32`, starts at byte 31.

`proj-alpha`, TUI `steps-other.json`: the hover on `alpha`, the caret
moved to the blank line in `beta`, `C-c H`: `*lsp-help*` opens in a
split with the header `hover documentation, the last popup's (none at
the caret)   q quit` and rust-analyzer's own text, `fn alpha(x: u8) ->
u8` among it.

`symlink/real` and `symlink/link -> real` (`proj-symlink`, a three-line project),
TUI `steps-symlink.json`, the needle `e8r2` (the crate line of the hover,
which the source does not contain): by the real path the popup opens in
2.1 s; through the link nothing in 90.7 s, the status `LSP: no hover
info`, the modeline `LSP:ready`. My first run waited for `Alpha adds
one`, which the source's own doc comment shows, and "found" it through
the link; discarded.

`proj-fmt` (`use std::fmt;`), TUI `steps-pointer.json` (fix round 1's
`steps-m3.json`): one wheel notch, three more and a left click inside
the popup each leave it painted with its closing row, the caret at
`L1:C11` and the first row unchanged; `C-c H` opens `*lsp-help*`. GPU
`gpu-pointer.script`: `lines=256`, `omitted=277`, 400 notches clamp at
`scroll=274`, a click keeps it, `C-e` closes it.
