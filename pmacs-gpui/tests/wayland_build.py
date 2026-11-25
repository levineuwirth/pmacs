"""Build-panel portion of the private Wayland editing test."""
import csv
import io
import sys


def build_panel(path, out, run, key, shot, pane, language, pointer, wait_for):
    def command(name):
        key("x", "alt")
        run("wtype", "-d", "8", "--", name)
        key("Return")

    def ocr(name, image):
        crop = out / f"{name}-output.png"
        image.crop((0, image.height // 2, image.width, image.height)).resize(
            (image.width * 2, (image.height - image.height // 2) * 2)).save(crop)
        text = run("tesseract", crop, "stdout", "-l", language, "--psm", "6")
        (out / f"{name}-output.txt").write_text(text)
        return text.lower()

    def visible(name, *words, dimensions=None):
        def observe():
            image = shot(name)
            if dimensions and image.size != dimensions:
                return None
            text = ocr(name, image)
            return image if all(word in text for word in words) else None
        return wait_for(observe, f"{name} with {words}")

    def click(image, x, y):
        run(sys.executable, pointer, "--size", f"{image.width}x{image.height}", "click", str(x), str(y))

    def focus_next():
        key("x", "ctrl")
        key("o")

    command("compile.run")
    key("End")
    for _ in "cargo build":
        key("BackSpace")
    run("wtype", "--", "cargo test")
    key("Return")
    # Cargo and the editor's job watcher update independently of paint.
    initial = visible("11-build", "output", "compilation")
    def border(image):
        return next((y for y in range(40, image.height)
                     if image.getpixel((0, y)) in [(246, 219, 162), (86, 84, 78)]), -1)
    visible("11-finished", "exited abnorm")
    top = border(initial)
    click(initial, 75, top + 32 + 10)
    wait_for(lambda: shot("12-click").getpixel((0, top + 2)) == (246, 219, 162),
             "the daemon to focus the clicked panel")
    command("cursor.buffer-end")
    visible("13-failed", "3 passed", "1 failed", "average", "half", "compile exited")
    def drag(frame, scale, target):
        x = frame.width // 2
        y = border(frame) + round(4 * scale)
        run(sys.executable, pointer, "--size", f"{frame.width}x{frame.height}",
            "drag", str(x), str(y), str(x), str(target + round(4 * scale)))

    gold = (246, 219, 162)
    gray = (86, 84, 78)

    def settled(name, position, color, dimensions=None):
        def observe():
            frame = shot(name)
            top = border(frame)
            if dimensions:
                # Sway can expose the new output with the previous, smaller
                # surface at its origin while GPUI is waiting for a new frame.
                if frame.size != dimensions or top < 0 or frame.getpixel(
                        (frame.width - 2, top + 10)) != (50, 48, 44):
                    return None
            return frame if top >= 0 and position(top) and frame.getpixel((0, top + 10)) == color else None
        return wait_for(observe, name)

    # Read the actual diagnostic's baseline from the painted output. The
    # stock daemon offsets the numbered column, but the row must match.
    def hit_failure(name, frame, scale):
        top = border(frame) + round(32 * scale)
        crop = out / f"{name}-grid.png"
        frame.crop((0, top, frame.width, frame.height - round(24 * scale))).save(crop)
        tsv = run("tesseract", crop, "stdout", "-l", language, "--psm", "6", "tsv")
        word = next(row for row in csv.DictReader(io.StringIO(tsv), delimiter="\t")
                    if "half" in row["text"].lower())
        y = top + int(word["top"]) + int(word["height"]) // 2
        x = round(82 * scale)
        click(frame, x, y)
        def caret_on_failure(capture):
            after = shot(capture)
            return any(after.getpixel((col, y)) == gold
                       for col in range(x - round(10 * scale), x + round(70 * scale)))
        wait_for(lambda: caret_on_failure(name), "the output caret on the clicked failure")
        key("Down")
        wait_for(lambda: not caret_on_failure(name + "-down"), "Down moving the caret off the failure")

    for scale, mode in ((1, "1280x800"), (1.5, "1920x1200")):
        run("swaymsg", "output", "HEADLESS-1", "resolution", mode, "scale", str(scale))
        tag = f"divider-{scale}"
        frame = visible(f"{tag}-before", "exited", "half",
                        dimensions=(round(1280 * scale), round(800 * scale)))
        original_top = border(frame)
        bigger = original_top - round(60 * scale)
        drag(frame, scale, bigger)
        enlarged = settled(f"{tag}-grown", lambda y: abs(y - bigger) <= 2, gold)
        hit_failure(f"{tag}-click", enlarged, scale)
        command("cursor.buffer-end")
        visible(f"{tag}-end", "compile exited")
        focus_next()
        src = settled(f"{tag}-source", lambda y: abs(y - bigger) <= 2, gray)
        drag(src, scale, 0)
        maximum = settled(f"{tag}-max", lambda y: 64 * scale < y < bigger - 20 * scale, gray)
        drag(maximum, scale, maximum.height - round(8 * scale))
        minimum = settled(f"{tag}-min", lambda y: bigger < y <= maximum.height - 96 * scale + 2, gray)
        drag(minimum, scale, original_top)
        settled(f"{tag}-restored", lambda y: abs(y - original_top) <= 2, gray)
        key("a", "ctrl")
        run("wtype", "--", "resizefocus")
        key("x", "ctrl")
        key("s", "ctrl")
        wait_for(lambda: "resizefocus" in path.read_text(), "typing in source after the drag")
        key("slash", "ctrl")
        key("x", "ctrl")
        key("s", "ctrl")
        wait_for(lambda: "resizefocus" not in path.read_text(), "undo in source")
        focus_next()
        command("cursor.buffer-end")
    # Resize with the result still open. A new cell declaration must obtain
    # a new frame before a click can address its row and column.
    run("swaymsg", "output", "HEADLESS-1", "resolution", "640x360", "scale", "1")
    compact = settled("compact-before", lambda y: y > 0, gold, dimensions=(640, 360))
    drag(compact, 1, 350)
    short = settled("compact-min", lambda y: abs(y - (360 - 96)) <= 2, gold)
    drag(short, 1, 0)
    settled("compact-max", lambda y: 64 < y < 106, gold)
    run("swaymsg", "output", "HEADLESS-1", "resolution", "900x720", "scale", "1")
    large = settled("14-back", lambda y: y > 0, gold, dimensions=(900, 720))
    drag(large, 1, 720 - 296)
    settled("14-original-height", lambda y: abs(y - (720 - 296)) <= 2, gold)
    command("cursor.buffer-start")
    command("cursor.buffer-end")
    resized = visible("14-resized", "3 passed", "1 failed", "compile exited")
    top = border(resized)
    # Click an inked cell on the result row; the daemon paints its caret
    # at the hit. The separate header and side padding are outside the grid.
    x = 82

    def long_row(image):
        return next(y for y in range(border(image) + 32, image.height - 46, 20)
                    if image.crop((200, y + 2, 300, y + 18)).getextrema()[0][1] > 160)

    def click_and_locate(image, name):
        y = long_row(image)
        click(image, x, y + 10)
        def observed():
            frame = shot(name)
            return next((col for col in range(12, frame.width) if frame.getpixel((col, y + 5)) == gold), None)
        return wait_for(observed, "the daemon caret on the clicked output row")

    caret_x = click_and_locate(resized, "15-mapped-click")
    (out / "15-gutter-click.txt").write_text(f"Click at x={x}, painted caret starts at x={caret_x}.\n")
    # The stock daemon counts the numbered gutter as text on the inverse
    # path. Remove that gutter in the panel and require an exact cell hit.
    key("x", "ctrl")
    key("l")
    def bare_grid():
        image = shot("15-no-gutter")
        y = border(image) + 32
        # The numbered gutter occupied the first four cells. Its removal
        # moves the start of a line of text all the way to the grid origin.
        return image if any(image.crop((12, row + 2, 20, row + 18)).getextrema()[0][1] > 150
                            for row in range(y, y + 11 * 20, 20)) else None
    bare = wait_for(bare_grid, "the output grid without line numbers")
    key("a", "ctrl")
    y = long_row(bare)
    wait_for(lambda: shot("15-start-of-line").getpixel((12, y + 5)) == gold,
             "caret at the start of the bare line before the click")
    landed = click_and_locate(bare, "15-exact-cell")
    with (out / "15-gutter-click.txt").open("a") as result:
        result.write(f"Without line numbers, click at x={x}, caret at x={landed}.\n")
    assert 0 <= x - landed < 9, f"clicked x={x}, caret at {landed}"
    focus_next()
    wait_for(lambda: shot("16-source-focus").getpixel((0, top + 2)) == (86, 84, 78),
             "keyboard focus back in the document")
    original = path.read_text()
    target = next(i for i, line in enumerate(original.splitlines(), 1)
                  if "assert_eq!(ledger.average(), 3);" in line)
    def save():
        key("x", "ctrl")
        key("s", "ctrl")
    command("cursor.goto-line")
    run("wtype", "--", str(target))
    key("Return")
    key("End")
    key("Left")
    key("Left")
    key("BackSpace")
    run("wtype", "--", "2")
    save()
    fixed = original.replace("assert_eq!(ledger.average(), 3);", "assert_eq!(ledger.average(), 2);")
    wait_for(lambda: path.read_text() == fixed, "the corrected expectation on disk")
    command("compile.recompile")
    # This invocation may focus the panel or retain document focus.
    visible("17-rerun", "finished")
    if shot("17-rerun").getpixel((0, top + 2)) != (246, 219, 162):
        focus_next()
    command("cursor.buffer-end")
    visible("18-passed", "4 passed", "failed")
    focus_next()
    key("slash", "ctrl")
    save()
    wait_for(lambda: path.read_text() != fixed, "the first saved undo")
    if path.read_text() != original:
        key("slash", "ctrl")
        save()
    wait_for(lambda: path.read_text() == original, "undo restoring the original source")
    shot("19-restored")
    (out / "19-restored-tui.txt").write_text(pane())
    print("PASS: real cargo test failed, mapped panel click, focus return, corrected test passed, undo restored source.")
