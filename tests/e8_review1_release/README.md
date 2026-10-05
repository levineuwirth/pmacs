# E8 review round 1 — release-build probes

Behavioural probes against PR #315's head `1ace8b3`, run on the release
pair built from that head (`cargo build --release --workspace` into
`~/build/e8-review1/rel-target`: `pmacs-gpu 1.1.0 (protocol v26)`,
`pmacs-parse-unit 1.1.0 protocol 3`). Not a cargo target: the drivers
are scripts, and the two probe projects are stored as `.txt` so cargo
never mistakes them for packages (copy them to `Cargo.toml` and
`src/main.rs` to use them). Paths are the laptop's.

The cargo-run probes beside this directory are
`tests/e8_review1_probes.rs` (a real daemon, the fake server) and
`tests/e8_review1_real_server_probes.rs` (rust-analyzer and
basedpyright, in-process: the TUI's dispatch and `paint_frame`, and the
semantic producer's frame).

## Drivers

- `run-gpu.sh <name> <file> <script>`: a release daemon visiting `<file>`
  with real servers, the release GPU's `--headless-probe` under
  `PMACS_GPU_PROBE_ACTION=popup`, which drives `App::apply_keyboard`,
  `apply_wheel` and `apply_left_button`; the report lists the popup's
  facts at each `report:` step.
- `tui_drive.py <name> <file> <steps.json>`: the same daemon, the
  release `pmacs --attach` in a 120x40 tmux pane; keys by `send-keys`,
  SGR mouse input as raw bytes, captures plain and with `-e`.

## Runs and what they showed

GPU, `proj-fmt` (`use std::fmt;`), script: ten `key:C-f`, `ask:h`,
`report:fmt`, `wheel:400`, `report:bottom`, `click`, `report:clicked`,
`key:C-e`, `closed`, `report:gone`. rust-analyzer's hover on `fmt`:
`lines=256`, `omitted=277`, `rows=286`, `shown=12`; 400 notches clamp
at `scroll=274` (rows − shown, the closing row last); a click keeps it;
`C-e` closes it.

GPU, `proj-sig`, script: 28 `key:C-f`, `ask:s`, `report:pair`, `key:C-g`,
`closed`, 25 `key:C-f`, `ask:s`, `report:clamp`, `key:C-g`, `closed`,
`key:C-n`, `key:C-a`, 36 `key:C-f`, `ask:s`, `report:emoji`, `key:C-g`,
`closed`, `key:C-n`, `key:C-a`, 38 `key:C-f`, `ask:s`, `report:cjk`.
`pair.active=0:12:15` (the first `i32` of `struct Pair(i32, i32)`; the
second is 17..20), `clamp.active=0:17:27` (`len: usize` inside
`max_len: usize`; the second parameter is 29..39), `emoji` and `cjk`
`active=0:19:29` (`second: u8`, right), anchors 97 and 150 (their `(`).

TUI, `steps-fmt.json`: the popup closes with
`… 517 more lines · C-c H opens *lsp-help*`. One SGR wheel-down over the
popup (`ESC[<65;40;8M`) scrolls the document, the caret follows (`L6:C1`)
and the popup is gone; `C-c H` then says `LSP: no hover info`.

TUI, `steps-click.json`: a left click inside the popup (`ESC[<0;40;8M`,
`ESC[<0;40;8m`) moves the caret to the text under it and closes the
popup; the GPU swallows the same click (E8.3).

TUI, `steps-sig.json`: the grid underlines the first `i32` for `Pair`
and the `len: usize` tail of `max_len: usize` for `clamp_len`, and
`second: u8` on the emoji and CJK lines, each popup's left edge on its
call's `(` (columns 36 and 41, wide characters counted as two).
