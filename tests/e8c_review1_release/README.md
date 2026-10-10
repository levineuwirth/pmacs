# E8c review round 1 — release-build and real-server probes

Behavioral probes against PR #357's head `a35b76b`, run on the release
pair the handoff built at that head (`~/build/cargo-target/release/`,
copied to `~/build/e8c-review1/release/` before any run: `pmacs`
sha256 `2169b57a…`, `pmacs-gpu` `d9c1f2e0…`, `pmacs-parse-unit`
`5477bf27…`, `pmacs_fake_lsp` `d24b882e…`; `pmacs` carries E8c's Lua,
checked by its strings), and against `/usr/bin/rust-analyzer`
(`rust-analyzer 1 (9074e9b4c6 2026-09-06)`). Not a cargo target: the
drivers are scripts, and their paths are the laptop's. The cargo probes
beside this directory are `tests/e8c_review1_probes.rs`.

## Drivers

- `quit_run.py`: one quit in a private tmux server (`-S`, `-f
  /dev/null`), 120×40. The pane runs `sh`, which runs the editor as an
  ordinary child, so its exit hangs nothing up; `HOME`, every XDG root,
  `PMACS_STATE_HOME`, `TMPDIR` and `XDG_RUNTIME_DIR` are private and set
  inside the pane's command. `C-x C-c` by `send-keys` after `--delay`.
  It records the load at the keys, when the alternate screen went off,
  when the editor exited, its whole descendant tree before the keys and
  which of it is alive a second after. `--init` substitutes `@OUT@`,
  `@FAKE@` and `@MODE@`; `--tee` puts `ra_tee.py` on `PATH` as
  `rust-analyzer`; `--strace` runs the editor under `strace -f -tt -T`
  for the sync and rename calls; `--diagnose-after S` samples every
  thread's `/proc` state three times and `perf`s the main thread when
  the editor has not exited S seconds after the keys.
- `daemon_run.py`: a release daemon with private roots opens a file from
  its `init.lua`; `pmacs --attach` in a tmux pane sends `C-x C-c`; it
  records the frontend's exit and the daemon's.
- `ra_probe.py FILE MODE OUT`: rust-analyzer driven directly, its stderr
  kept: `initialize` alone, `didOpen` alone, or `didOpen` with each of
  the three requests pmacs sends after it.
- `heir_init.lua`, `heir2_init.lua`: the fake under a shell that starts
  a `sleep 600` holding its output (one server; rust and C).
  `status_init.lua`: the default servers and the status hook only.
- `load_batch.sh`, `diag_batch.sh`, `hog_batch.sh`, `io_batch.sh`: the
  batches below. Load came from `scripts/gate` sweeps at `a35b76b` in
  `~/Repos/personal/pmacs-quit-exit`, from 32 CPU-bound busy loops, or
  from a loop writing 2 GB files with `fdatasync`.

## What they showed (2026-10-10)

rust-analyzer on `braces20000.rs`, no `Cargo.toml` (`ra_probe.py`):
`didOpen` alone, 4 of 4 alive 10 s later and ended by `shutdown` and
`exit` with rc 0; `didOpen` with `inlayHint`, `semanticTokens/range`,
`semanticTokens/full` or all three, 7 of 7 aborted 0.514–0.564 s after
the `didOpen`, its stderr `thread 'WorkerN' (…) has overflowed its
stack` / `fatal runtime error: stack overflow, aborting` (`Worker1` or
`Worker3`).

Quit on `braces20000.rs` with the real server (`quit_run.py`):

| batch | condition | quit at | exit after the keys (ms) |
|---|---|---|---|
| smoke | idle, load 1.5 | 5 s | 77 |
| load1 | gate 2's sweep, load 75–135 | 1 s | 13,420; 5,915; 18,504; >20,000; >20,000; 10,239 |
| load1 | gate 2's sweep, load 77–144 | 5 s | 14,885; 77; 81; 78; 81; 77 |
| load1 `--tee` | load 65–71 | 5 s | 78; 69 (no `shutdown` or `exit` on the wire) |
| diag1 | 32 busy loops, load 20–22 | 1 s | 364; 428; 336; 410 |
| diag2 | gate 3's sweep, load 39–140 | 1 s | 15,626; 12,874; 20,591; 2,093; 16,098; 10,383 |
| strace1 | gate 3's sweep, load 82–123 | 1 s / 5 s | 267; 37; 37 / 68; 78; 77 |
| io1 `--strace` | 2 GB `fdatasync` writer, load 31–38 | 3 s | 2,750; 3,491; 7,003; 1,834 |

The two `>20,000` were killed at the driver's 20 s; with a 90 s wait
(diag2) every run exited. In every diagnosed stall the main thread was
in `D` in btrfs's fsync path (`wait_log_commit`,
`btrfs_commit_transaction`, `btrfs_btree_wait_writeback_range`,
`wait_for_commit`, `btrfs_tree_lock_nested`, `read_extent_buffer_pages`,
`folio_wait_bit_common`), with under 0.2 s of CPU. Under `strace` the
process's only sync calls are saveplace's write of `places` at quit
(`fsync` of the temporary, `rename`, `fsync` of the directory), two calls a run: 7–17 ms
each in strace1, and in io1 2.70 s, 3.38 s, 3.11 + 3.83 s and 1.70 s, matching
each run's exit.

A server whose child holds its output (`heir_init.lua`, release build):
quit 4,074–4,077 ms (3 runs, load 49–61), the alternate screen off at
26–28 ms and the pane blank until the exit; the `sleep` left under
init. Mid-session (`--mode abortonopen`): one 4,017 ms tick when the
first generation died (2 runs), then `*errors*` holds only
`LSP: default-rust crashed: signal SIGABRT`, and quit 4,062–4,074 ms.
Two such servers (`heir2_init.lua`, load 8): quit 8,079 ms.

Daemon path (`daemon_run.py`, idle): after rust-analyzer's crash was
reported, the frontend exited 61 ms after `C-x C-c` and the daemon at
101 ms, rc 0, no child left; with a heir-holding server, the frontend
at 61 ms and the daemon at 4,098 ms.
