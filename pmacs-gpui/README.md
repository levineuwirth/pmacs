# pmacs-gpui

An experimental native frontend for a running pmacs daemon. It displays and
edits a source buffer, follows remote edits and shows the daemon's command
palette. The daemon handles keybindings, buffer contents, command selection
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
Closing GPUI leaves the daemon and TUI running. The trial currently targets
keyboard editing in one document and the minibuffer.

The Linux native-window regression uses Sway, grim, wtype, tmux and Python
with Pillow. It starts a private compositor, daemon and TUI. Supply built
executables and a new directory for its screenshots and logs.

```sh
python3 pmacs-gpui/tests/wayland_editing.py --pmacs /path/to/pmacs \
  --gpui /path/to/pmacs-gpui --output /tmp/pmacs-edit-captures
```
