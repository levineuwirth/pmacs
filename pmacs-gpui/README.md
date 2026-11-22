# pmacs-gpui

An experimental native file view for a running pmacs daemon. It displays the
requested buffer and follows remote edits. The document scrolls vertically.
Keyboard editing, the command palette and the build panel are later milestones.

Build from the repository root with the pinned Rust toolchain. GPUI 0.2.2
is locked separately so building the release pair does not build the trial.

```sh
cargo build --locked --manifest-path pmacs-gpui/Cargo.toml
```

Start a stock CRDT-enabled daemon in one terminal and keep its socket private.

```sh
socket_dir=$(mktemp -d)
printf '%s\n' "$socket_dir/editor.sock"
pmacs --daemon --socket "$socket_dir/editor.sock"
```

In a second terminal, use the printed socket path and a file of your choice.

```sh
cargo run --locked --manifest-path pmacs-gpui/Cargo.toml -- --attach /path/to/editor.sock src/main.rs
```

To check the live view, attach a TUI with `pmacs --attach --socket /path/to/editor.sock`,
visit the same file with `C-x C-f`, and type without saving. The GPUI window should
show the edit immediately. Closing it leaves the daemon and TUI running.
