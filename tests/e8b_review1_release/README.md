# E8b review round 1 — release-build probes

Behavioral probes against PR #325's head `0001946`, run on the release
pair built at that head and linked from `~/.local/bin`
(`~/build/cargo-target/release`: `pmacs 1.1.0`, `pmacs-gpu 1.1.0
(protocol v26)`, `pmacs-parse-unit 1.1.0 protocol 3`, dated 2026-10-07
10:09–10:10 +0200, after the head's commit at 00:43). Not a cargo
target: the drivers are scripts. Paths are the laptop's.

The cargo-run probes beside this directory are
`tests/e8b_review1_probes.rs` (in-process, through `dispatch_key`) and
`tests/e8b_review1_daemon_probes.rs` (E8 review 1's switching row
replayed through a real daemon).

## Drivers

- `session.sh <name>` lays out a session under
  `~/build/e8b-review1/witness/<name>/` and starts a release daemon:
  a project `proj/` holding `a.rs`, `b.rs`, `notes.txt`, `dired.lua`,
  `editor.rs` and `src/lsp.rs`, all visited, `*scratch*` kept, `a.rs`
  shown. The root spells `b` and `nts`. The init binds `C-c z` to
  `review.where`, which writes `WHERE <the window's buffer>` to the
  band: an oracle for the window that does not read E8b.2's message.
- `make-steps.py` writes one key sequence for both drivers:
  `steps-tui.json` and `gpu.script`.
- `tui_drive.py <name> steps-tui.json`: the release `pmacs --attach` in
  a 120x40 tmux pane, keys by `tmux send-keys`; the report holds each
  capture's mode line and band.
- `run-gpu.sh <name> gpu.script`: the release GPU's `--headless-probe`
  under `PMACS_GPU_PROBE_ACTION=popup`, each `key:` step through
  `App::apply_keyboard`; each `report:` writes the band.

## Runs and what they showed

`tui-1` and `gpu-1`, the same sequence, the same results on both
frontends:

- `C-x b b.rs RET` from `a.rs`: `switch-buffer: showing …/b.rs`, then
  `WHERE …/b.rs`; again: `switch-buffer: already showing …/b.rs`.
- D18's four: `nts` → `notes.txt`, `scr` → `*scratch*`, `zzz` → `no
  buffer: zzz` (window unchanged), and, after `C-c l`, `lsp` → `*lsp*`
  with `src/lsp.rs` open.
- `C-x d RET` opens `*dired:…/proj*` (its mode line reads `+*`).
- **`C-x b dired RET` from `editor.rs`: `switch-buffer: showing
  …/proj/dired.lua`, `WHERE …/dired.lua`. `C-x b dir RET` from
  `editor.rs`: `dired.lua` again.** `C-x b *dir RET` reaches the dired
  buffer.
- **`C-x k`, the prefill `*scratch*` deleted (`C-a`, nine `C-d`),
  `dired`, RET: the band stays empty, and `C-x b dired.lua RET` then
  says `no buffer: dired.lua`: the file buffer was killed, silently,
  and the dired buffer kept.**

GPU: `session_protocol_version=26`, `ready=true`, `probe_exit=0`, 47 s.
