#!/usr/bin/env python3
"""Exercise native keyboard editing against a stock daemon and an independent TUI."""
import argparse
from contextlib import ExitStack
import os
from pathlib import Path
import subprocess
import tempfile
import time

from PIL import Image


def wait_for(observe, description):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = observe()
        if result:
            return result
        time.sleep(0.1)
    raise RuntimeError(f"Timed out waiting for {description}")


def exercise(args, runtime, processes):
    out = args.output
    env = {
        "PATH": f"{args.pmacs.parent}:{os.environ['PATH']}",
        "LANG": "C.UTF-8", "SHELL": "/bin/sh", "TERM": "xterm-256color",
        "XDG_RUNTIME_DIR": str(runtime), "WLR_BACKENDS": "headless",
        "WLR_LIBINPUT_NO_DEVICES": "1",
    }
    for name in ("HOME", "TMPDIR", "XDG_CONFIG_HOME", "XDG_CACHE_HOME",
                 "XDG_DATA_HOME", "XDG_STATE_HOME", "PMACS_STATE_HOME"):
        directory = runtime / name.lower()
        directory.mkdir(mode=0o700)
        env[name] = str(directory)

    def run(*cmd):
        return subprocess.run(cmd, cwd=runtime, env=env, check=True,
                              capture_output=True, text=True, timeout=12).stdout

    def launch(name, *cmd):
        log = processes.enter_context((out / f"{name}.log").open("w"))
        proc = subprocess.Popen(cmd, cwd=runtime, env=env, stdout=log, stderr=log)

        def stop():
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
        processes.callback(stop)
        return proc

    def first_socket(pattern):
        return next((path for path in runtime.glob(pattern) if path.is_socket()), None)

    config = runtime / "sway.conf"
    config.write_text("default_border none\nfocus_follows_mouse no\n"
                      "output HEADLESS-1 resolution 1280x800 scale 1\n")
    launch("sway", "sway", "-c", config)
    env["SWAYSOCK"] = str(wait_for(lambda: first_socket("sway-ipc*.sock"), "Sway IPC"))
    env["WAYLAND_DISPLAY"] = wait_for(lambda: first_socket("wayland-*"), "Wayland").name
    # A headless seat has no physical keyboard. Keep one present while clients
    # attach and while each subsequent wtype invocation sends its keys.
    launch("keyboard", "wtype", "-s", "120000")

    path = runtime / "ledger.rs"
    original = args.fixture.read_bytes() if args.fixture else b"// Ledger entries\nfn main() {}\n"
    path.write_bytes(original)
    first_line = original.decode().splitlines()[0]
    socket = runtime / "editor.sock"
    launch("daemon", args.pmacs, "--daemon", "--socket", socket)
    wait_for(socket.is_socket, "daemon socket")
    window = launch("gpui", args.gpui, "--attach", socket, path)
    wait_for(lambda: '"app_id": "pmacs-gpui"' in run("swaymsg", "-t", "get_tree", "-r"),
             "GPUI window")

    tm = ("tmux", "-S", str(runtime / "tmux.sock"), "-f", "/dev/null")
    run(*tm, "new-session", "-d", "-s", "watch", "-x", "120", "-y", "36",
        args.pmacs, "--attach", "--socket", socket)
    processes.callback(subprocess.run, (*tm, "kill-server"), env=env, capture_output=True)
    def pane():
        return run(*tm, "capture-pane", "-p", "-t", "watch")

    wait_for(lambda: "scratch" in pane().lower() or path.name in pane(), "attached TUI")
    run(*tm, "send-keys", "-t", "watch", "C-x", "C-f")
    wait_for(lambda: "file" in pane().lower(), "find-file prompt")
    run(*tm, "send-keys", "-t", "watch", "End")
    run(*tm, "send-keys", "-t", "watch", "-l", path.name)
    run(*tm, "send-keys", "-t", "watch", "Enter")

    def source(name, prefix):
        def observe():
            text = pane()
            (out / f"{name}-tui.txt").write_text(text)
            return text.splitlines()[0].lstrip().startswith(f"1 {prefix}{first_line[:64]}")
        wait_for(observe, f"TUI line {prefix + first_line!r}")

    def shot(name):
        target = out / f"{name}.png"
        run("grim", target)
        with Image.open(target) as image:
            return image.convert("RGB")

    def capture(name, predicate):
        def observe():
            image = shot(name)
            return image if predicate(image) else None
        return wait_for(observe, name)

    def body(image):
        # Exclude the first two columns, where the caret can move on undo.
        return image.crop((90, 40, 400, 62)).tobytes()

    def has_text(image):
        return image.crop((90, 43, 220, 62)).getextrema()[0][1] > 160

    def row_colors(image):
        # Compare the right padding of the first two rows. Down should swap
        # their selected/unselected colors, independent of the color scheme.
        width = image.width
        x = width // 2 + min(760, width - 24) // 2 - 8
        return (image.getpixel((x, 120)), image.getpixel((x, 155)))

    def key(keysym, modifier=None):
        if modifier:
            run("wtype", "-M", modifier, "-k", keysym, "-m", modifier)
        else:
            run("wtype", "-k", keysym)

    run("swaymsg", '[app_id="pmacs-gpui"]', "focus")
    source("00-before", "")
    initial = capture("00-before", has_text)
    run("wtype", "--", "A")
    source("01-first", "A")
    capture("01-first", lambda im: has_text(im) and body(im) != body(initial))
    key("x", "alt")
    wide = capture("02-wide", lambda im: row_colors(im)[0] != row_colors(im)[1])
    run("wtype", "-d", "12", "--", "buffer.")
    region = (275, 58, 930, 140)
    filtered = capture("03-filtered", lambda im: im.crop(region).tobytes() != wide.crop(region).tobytes()
                       and row_colors(im) == row_colors(wide))
    key("Down")
    capture("04-selected", lambda im: row_colors(im) == row_colors(filtered)[::-1])
    run("swaymsg", "output", "HEADLESS-1", "resolution", "480x720", "scale", "1")
    capture("05-narrow", lambda im: im.width == 480 and row_colors(im)[0] != row_colors(im)[1])
    key("g", "ctrl")
    capture("06-cancel", lambda im: row_colors(im)[0] == row_colors(im)[1])
    source("06-cancel", "A")
    run("wtype", "--", "B")
    source("07-second", "AB")
    key("slash", "ctrl")
    source("08-single-undo", "A")
    run("swaymsg", "output", "HEADLESS-1", "resolution", "1280x800", "scale", "1")
    key("x", "alt")
    run("wtype", "-d", "12", "--", "buffer.undo")
    capture("09-execute", lambda im: im.width == 1280 and row_colors(im)[0] != row_colors(im)[1])
    key("Return")
    source("10-command-undo", "")
    capture("10-command-undo", lambda im: body(im) == body(initial))
    assert path.read_bytes() == original, "editing saved the disk fixture"
    run("swaymsg", '[app_id="pmacs-gpui"]', "kill")
    assert window.wait(timeout=8) == 0
    print(f"PASS: two clients, two edits, keyboard selection and cancel, two undos. Captures in {out}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pmacs", type=Path, required=True)
    parser.add_argument("--gpui", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, help="optional Rust source to copy into the private session")
    parser.add_argument("--output", type=Path, required=True, help="new directory for captures and logs")
    args = parser.parse_args()
    args.pmacs, args.gpui, args.output = (p.resolve() for p in (args.pmacs, args.gpui, args.output))
    args.output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="pmgui-", dir="/tmp") as temp, ExitStack() as processes:
        exercise(args, Path(temp), processes)
