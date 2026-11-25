"""Visit a real rustc diagnostic and prove the exact destination by editing it."""
import csv
import io
import sys


def visit_diagnostic(path, out, run, key, shot, language, pointer, wait_for):
    target = path.with_name("ledger.rs")
    before = target.read_text()
    expected = before.replace("let held =", "let _held =")
    assert expected != before
    def command(name):
        key("x", "alt")
        run("wtype", "--", name)
        key("Return")

    def compile_command(previous, cmdline):
        command("compile.run")
        key("End")
        run("wtype", *(["-k", "BackSpace"] * len(previous)))
        run("wtype", "--", cmdline)
        key("Return")

    compiler = "cargo rustc --message-format=short -- -Dunused-variables"
    compile_command("cargo build", compiler)

    def save():
        key("x", "ctrl")
        key("s", "ctrl")

    def capture_focus(name, scale, color):
        frame = shot(name)
        return any(frame.getpixel((0, y)) == color
                   for y in range(round(40 * scale), frame.height))

    def location(tag, scale, needle):
        im = shot(tag + "-output")
        if im.size != (round(1280 * scale), round(800 * scale)):
            return None
        colors = [(246, 219, 162), (86, 84, 78)]
        title = next((y for y in range(im.height) if im.getpixel((0, y)) in colors), None)
        if title is None:
            return None
        top = title + round(32 * scale)
        grid = out / (tag + "-grid.png")
        im.crop((0, top, im.width, im.height - round(24 * scale))).save(grid)
        tsv = run("tesseract", grid, "stdout", "-l", language, "--psm", "6", "tsv")
        (out / (tag + ".tsv")).write_text(tsv)
        for word in csv.DictReader(io.StringIO(tsv), delimiter="\t"):
            if word["text"].lower() == needle:
                return im, round(110 * scale), top + int(word["top"]) + int(word["height"]) // 2
        return None

    def pointer_event(mode, gesture, *coords):
        run(sys.executable, pointer, "--size", mode, gesture, *map(str, coords))

    for scale, mode in ((1, "1280x800"), (1.5, "1920x1200")):
        tag = f"visit-{scale}"
        run("swaymsg", "output", "HEADLESS-1", "resolution", mode, "scale", str(scale))
        image, x, y = wait_for(lambda: location(tag, scale, "unused"), "the compiler diagnostic in the output")
        # The echoed command is ordinary output, even though it includes
        # the same compiler flag. A drag beginning on the link is not a click.
        title = next(y for y in range(image.height) if image.getpixel((0, y)) in
                     [(246, 219, 162), (86, 84, 78)])
        pointer_event(mode, "click", x, title + round(42 * scale))
        wait_for(lambda: capture_focus(tag + "-plain", scale, (246, 219, 162)), "plain output focus")
        pointer_event(mode, "drag", x, y, x, y + round(20 * scale))
        assert capture_focus(tag + "-drag", scale, (246, 219, 162)), "drag activated a location"
        pointer_event(mode, "click", x, y)
        wait_for(lambda: capture_focus(tag + "-jump", scale, (86, 84, 78)),
                 "source focus from one click")
        run("wtype", "--", "_")
        save()
        wait_for(lambda: target.read_text() == expected, "an edit at the reported file, line and column")
        key("slash", "ctrl")
        save()
        wait_for(lambda: target.read_text() == before, "undo at the visited location")
        key("comma", "alt")
        wait_for(lambda: capture_focus(tag + "-back", scale, (246, 219, 162)),
                 "jump back to Output")
    # libtest prefixes its location with a thread name and process/thread id.
    # Visit that real output, then prove which character received the edit.
    compile_command(compiler, "RUSTFLAGS=-Awarnings cargo test --quiet --lib average_rounds_half_up")
    scale, mode = 1.5, "1920x1200"
    tag = "panic"
    def finished():
        frame = shot("panic-result")
        grid = out / "panic-result-grid.png"
        frame.crop((0, frame.height // 2, frame.width, frame.height)).save(grid)
        return "exited abnorm" in run("tesseract", grid, "stdout", "-l", language, "--psm", "6").lower()
    wait_for(finished, "libtest to exit")
    if not capture_focus("panic-focus", scale, (246, 219, 162)):
        key("x", "ctrl")
        key("o")
    command("cursor.buffer-end")
    command("compile.previous-error-line")
    _, x, y = wait_for(lambda: location(tag, scale, "panicked"), "the libtest panic row")
    pointer_event(mode, "click", x, y)
    wait_for(lambda: capture_focus("panic-jump", scale, (86, 84, 78)), "source focus on the assertion")
    original = path.read_text()
    edited = original.replace("    assert_eq!(ledger.average(), 3);", "    _assert_eq!(ledger.average(), 3);")
    assert edited != original
    run("wtype", "--", "_")
    save()
    wait_for(lambda: path.read_text() == edited, "the panic location at ledger_tests.rs:29:5")
    key("slash", "ctrl")
    save()
    wait_for(lambda: path.read_text() == original, "undo at the failing assertion")
    print("PASS: compiler location at both scales, jump back, and libtest panic location; exact edits restored.")
