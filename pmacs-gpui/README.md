# pmacs-gpui

An experimental native frontend for a running pmacs daemon. It displays and
edits a source buffer, follows remote edits and shows the daemon's command
palette and bottom output panel. The daemon handles keybindings, buffer contents, command selection
and undo. Use a release build when comparing scrolling or input response.

From the repository root, build the independent trial with the pinned Rust toolchain.

```sh
cargo build --release --locked --manifest-path pmacs-gpui/Cargo.toml
```

Start a stock CRDT-enabled daemon in one terminal and keep its socket private.

```sh
socket_dir=$(mktemp -d)
printf '%s\n' "$socket_dir/editor.sock"
pmacs --daemon --socket "$socket_dir/editor.sock"
```

In a second terminal, use the printed socket path and a file of your choice.

```sh
cargo run --release --locked --manifest-path pmacs-gpui/Cargo.toml -- --attach /path/to/editor.sock src/main.rs
```

Type a character and press `Alt-x` (`M-x`). Type part of a command name
to filter the palette. Use the arrow keys to move the highlight, `Enter` to
run the command or `Ctrl-g` to return to the document. Wide windows place
descriptions beside names. Narrow windows place them below.

To try the two-client check, visit the same file in a TUI started with
`pmacs --attach --socket /path/to/editor.sock`. Type in GPUI, open and cancel
the palette, then type again. `Ctrl-/` undoes the second edit in both windows.
Closing GPUI leaves the daemon and TUI running.

In a Rust project, run `Alt-x compile.run Enter` and replace the proposed
command with `cargo test`. The output appears below the source. Click an output
line to focus the panel, or press `Ctrl-x` followed by `o` to switch focus.
Use the arrow keys or wheel to read the output. From the panel, the command
`cursor.buffer-end` jumps to the final test summary. Press `Ctrl-x o` again
to edit the source. Save with `Ctrl-x Ctrl-s` and run `compile.recompile`.

The panel paints the daemon’s cell frame, including its line numbers and mode
line. On the baseline daemon, a click in a numbered output row can place the
caret to the right by the gutter width. For precise positioning there, toggle
the focused panel’s line numbers with `Ctrl-x l`.

The Linux native-window regression uses Sway, grim, wtype, tmux and Python
with Pillow. It starts a private compositor, daemon and TUI. Supply built
executables and a new directory for its screenshots and logs.

```sh
python3 pmacs-gpui/tests/wayland_editing.py --pmacs /path/to/pmacs \
  --gpui /path/to/pmacs-gpui --output /tmp/pmacs-edit-captures
```

To exercise a real failing build, add `--build-baseline /path/to/pmacs-baseline`.
The check copies the baseline’s `proj/tally` to its private session and uses
`rig/vpointer.py` for native clicks. It also needs Rust and Tesseract. Select
an installed OCR language with `--ocr-language` (the default is `eng`).
It checks the failure, resizes and clicks the panel, corrects the assertion
in the source, reruns the tests, and undoes the correction.

Click in the source to place the caret. Drag to select, or Shift-click to
extend the selection. Clicks in an expanded tab land at the nearer edge of
the original tab. The caret and the highlight follow the shaped text,
including emoji. The daemon supplies the selection shown after either a
pointer gesture or a keyboard command.

For the source comparison, pass `--selection-baseline /path/to/pmacs-baseline`
to the native test. Repeat with `--stock` and a different output directory
to measure the shipped GUI. Both runs copy the same Rust ledger, use a
separate Unicode fixture, and compare saved bytes at 100% and 150% scale.

The trial pins the published GPUI 0.2.2 crate. Revisit the dependency when the
[standalone upstream](https://github.com/zed-industries/gpui) publishes a
buildable release. Run the native editing, panel, selection and scale checks
against the candidate before changing the pin.

In-buffer completion appears at the start of the typed prefix. Keep typing
to narrow the list, use Ctrl-n or Ctrl-p to change the selection, Tab or
Enter to accept, and Escape to dismiss. At the end of the trial ledger,
type `format_ce` and press Tab for `format_cents`. One Ctrl-/ undoes the
acceptance. Use `M-x completion.at-point` to request candidates explicitly.

The palette and completion popup use the same GPUI Component 0.5.1 list
row on the pinned GPUI 0.2.2. To run the native two-scale placement and
editing regression, pass `--completion-ledger /path/to/tally/src/ledger.rs`
to `wayland_editing.py`. The test copies the file into a private session.
