"""Check source gestures against saved UTF-8, using keyboard caret positions as landmarks."""
import hashlib
import json
import sys
import time

from PIL import Image, ImageChops


def source_selection(args, runtime, run, launch, wait_for):
    baseline = args.selection_baseline.resolve()
    pointer = baseline / "rig/vpointer.py"
    ledger = runtime / "ledger.rs"
    ledger.write_bytes((baseline / "proj/tally/src/ledger.rs").read_bytes())
    path = runtime / "source.txt"
    first = "ab\t😀z"
    original = f"{first}\nsecond line\n\nL👩‍💻R\n".encode()
    path.write_bytes(original)
    long_path = runtime / "long.txt"
    long_line = (b"0123456789" * 32) + b"\n"
    long_path.write_bytes(long_line)
    socket = runtime / "editor.sock"
    launch("daemon", args.pmacs, "--daemon", "--socket", socket)
    wait_for(socket.is_socket, "daemon")
    result = {"ledger_sha256": hashlib.sha256(ledger.read_bytes()).hexdigest(),
              "fixture_sha256": hashlib.sha256(original).hexdigest(), "scales": {}}

    def key(name, mod=None):
        cmd = ["wtype"]
        if mod:
            cmd.extend(["-M", mod])
        cmd.extend(["-k", name])
        if mod:
            cmd.extend(["-m", mod])
        run(*cmd)

    def save():
        key("x", "ctrl")
        key("s", "ctrl")

    def capture(label):
        target = args.output / f"{label}.png"
        run("grim", target)
        with Image.open(target) as im:
            return im.convert("RGB")

    def open_file(file, label):
        cmd = [args.pmacs, "--gpu", "--socket"] if args.stock else [args.gpui, "--attach"]
        proc = launch(label, *cmd, socket, file)
        def focused():
            def visit(node):
                if node.get("focused") and node.get("pid"):
                    return node["id"]
                return next((found for child in node["nodes"] + node["floating_nodes"]
                             if (found := visit(child))), None)
            return visit(json.loads(run("swaymsg", "-t", "get_tree", "-r")))
        node = wait_for(focused, "focused editor window")
        wait_for(lambda: capture(label).crop((80, 40, 320, 110)).convert("L").getextrema()[1] > 120,
                 "source text")
        return proc, node

    def close(proc, node):
        run("swaymsg", f"[con_id={node}]", "kill")
        assert proc.wait(timeout=10) == 0

    def check_edit(file, before, after, label):
        save()
        try:
            wait_for(lambda: file.read_bytes() != before, f"{label} saved edit")
        except RuntimeError:
            if not args.stock:
                raise
            return "cut left the file unchanged"
        actual = file.read_bytes()
        key("slash", "ctrl")
        save()
        wait_for(lambda: file.read_bytes() == before, f"{label} undo")
        if actual != after:
            index = next((i for i, pair in enumerate(zip(after, actual)) if pair[0] != pair[1]),
                         min(len(actual), len(after)))
            detail = f"first difference at byte {index}: expected {after[index:index+20]!r}, got {actual[index:index+20]!r}"
            if not args.stock:
                raise AssertionError(f"{label}: {detail}")
            return detail
        return "pass"

    def visit(file):
        key("x", "ctrl")
        key("f", "ctrl")
        key("End")
        run("wtype", "--", file.name)
        key("Return")
        key("x", "alt")
        run("wtype", "--", "cursor.buffer-start")
        key("Return")

    proc, node = open_file(ledger, "ledger")
    for scale, width, height in [(1, 1280, 800), (1.5, 1920, 1200)]:
        label = f"s{scale}"
        run("swaymsg", "output", "HEADLESS-1", "resolution", f"{width}x{height}", "scale", str(scale))
        output = json.loads(run("swaymsg", "-t", "get_outputs", "-r"))[0]
        assert (output["scale"], output["rect"]["width"], output["rect"]["height"]) == (scale,1280,800)
        checks = result["scales"][str(scale)] = {}
        if scale != 1:
            visit(ledger)
        capture(f"{label}-ledger")
        before = ledger.read_bytes()
        key("Home")
        key("x", "alt")
        run("wtype", "-d", "12", "--", "buffer.")
        capture(f"{label}-palette")
        key("g", "ctrl")
        run("wtype", "--", "Q")
        checks["ledger: cancel palette, type once, undo"] = check_edit(ledger, before, b"Q" + before, "palette cancel and type")
        visit(path)
        capture(f"{label}-source")
        key("Home")
        # Diff the same row against its end caret. Its leftmost changed column
        # is the independently painted keyboard caret, in screenshot pixels.
        key("End")
        time.sleep(.15)
        ref = capture(f"{label}-end")
        assert ref.size == (width, height), ref.size
        crop = (int(18 * scale), 0, int(360 * scale), int(110 * scale))
        def caret(label_, previous=0, before=width):
            def observe():
                box = ImageChops.difference(capture(label_).crop(crop), ref.crop(crop)).getbbox()
                if box and box[3] - box[1] >= 5 * scale and previous < crop[0] + box[0] < before:
                    return (crop[0] + box[0], crop[1] + (box[1] + box[3]) // 2)
            return wait_for(observe, label_)
        key("Home")
        key("Right")
        key("Right")
        time.sleep(.15)
        tab, y = caret(f"{label}-before-tab")
        key("Right")
        emoji, _ = caret(f"{label}-before-emoji", tab)
        key("Right")
        after, _ = caret(f"{label}-after-emoji", emoji)
        assert tab < emoji < after, (tab,emoji,after)
        def gesture(kind, *positions):
            run(sys.executable, pointer, "--size", f"{width}x{height}", kind, *map(str, positions))
        def inserted(offset, x, test):
            gesture("click", x, y)
            run("wtype", "--", "Q")
            checks[test] = check_edit(path, original, original[:offset] + b"Q" + original[offset:], test)
        inserted(3, emoji + (after - emoji) // 5, "before emoji")
        inserted(7, after - (after - emoji) // 5, "after emoji")
        left = tab + (emoji - tab) // 4
        right = emoji - (emoji - tab) // 4
        for start, end, direction in [(left,right,"forward"), (right,left,"reverse")]:
            gesture("drag", start, y, end, y)
            im = capture(f"{label}-tab-{direction}")
            if not args.stock:
                assert im.getpixel(((left + right) // 2, y)) == (53,73,104), "tab highlight"
            key("w", "ctrl")
            test = f"{direction} tab drag"
            checks[test] = check_edit(path, original, original.replace(b"\t", b""), test)
        # Use the last line's two keyboard positions bracketing the ZWJ emoji.
        key("x", "alt")
        run("wtype", "--", "cursor.goto-line")
        key("Return")
        run("wtype", "--", "4")
        key("Return")
        key("End")
        time.sleep(.15)
        ref = capture(f"{label}-last-end")
        crop = (int(18 * scale), 0, int(360 * scale), int(145 * scale))
        key("Home")
        key("Right")
        time.sleep(.15)
        family, last_y = caret(f"{label}-before-joined-emoji")
        key("End")
        key("Left")
        end_family, _ = caret(f"{label}-after-joined-emoji", family)
        gesture("drag", family + 1, last_y, end_family, last_y)
        capture(f"{label}-joined-emoji")
        key("w", "ctrl")
        checks["joined emoji"] = check_edit(path, original, original.replace("👩‍💻".encode(), b""), "joined emoji")
        gesture("drag", family, last_y, emoji, y)
        capture(f"{label}-multiline")
        key("w", "ctrl")
        checks["reverse multiline over blank line"] = check_edit(path, original,
            original[:3] + "👩‍💻R\n".encode(), "multiline")
        if not args.stock:
            visit(long_path)
            key("End")
            time.sleep(.15)
            ref = capture(f"{label}-long-end")
            crop = (width // 2, 0, width, int(80 * scale))
            x = width
            for step in range(4):
                key("Left")
                x, row_y = caret(f"{label}-long-left-{step + 1}", before=x)
            key("End")
            time.sleep(.15)
            gesture("click", x, row_y)
            run("wtype", "--", "Q")
            offset = len(long_line) - 5
            checks["click in shifted long line"] = check_edit(long_path, long_line,
                long_line[:offset] + b"Q" + long_line[offset:], "shifted source")
        (args.output / "selection.json").write_text(json.dumps(result, indent=2) + "\n")
    close(proc, node)
    print(json.dumps(result, indent=2))
