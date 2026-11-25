"""Drive the daemon's real completion session at both edges of the source window."""
import json
import re

from PIL import Image, ImageChops

PROVIDER = """
pmacs.completion.register({
    name = "ledger-fixture", priority = 200,
    fn = function()
        return {
            { label = "zxtally", kind = "function", detail = "Add the ledger entries" },
            { label = "zxtotal", kind = "variable", detail = "Current balance" },
            { label = "zxwrite", kind = "method", detail = "Save the ledger" },
        }
    end
})
"""


def color_box(image, color):
    r, g, b = ImageChops.difference(image, Image.new("RGB", image.size, color)).split()
    mask = ImageChops.lighter(ImageChops.lighter(r, g), b).point(
        lambda value: 0 if value else 255)
    if color == (246, 219, 162):
        return mask.getbbox()
    # Only a filled candidate surface or its border has long horizontal runs
    # of one color. Antialiased header and gutter glyphs can share the color.
    raw = mask.tobytes()
    width = image.width
    bands = [(m.start() % width, m.start() // width, (m.end() - 1) % width + 1)
             for m in re.finditer(b"\xff{100,}", raw)]
    if bands:
        return min(x for x, _, _ in bands), bands[0][1], max(x for _, _, x in bands), bands[-1][1] + 1



def exercise_completion(path, run, key, shot, wait_for):
    original = path.read_bytes()

    def command(name):
        key("x", "alt")
        run("wtype", "--", name)
        key("Return")

    def saved(expected):
        key("x", "ctrl")
        key("s", "ctrl")
        wait_for(lambda: path.read_bytes() == expected, f"saved bytes ending in {expected[-32:]!r}")

    def type_text(text):
        run("wtype", "-d", "30", "--", text)

    for scale, width, height in [(1, 1280, 800), (1.5, 1920, 1200)]:
        run("swaymsg", "output", "HEADLESS-1", "resolution", f"{width}x{height}", "scale", str(scale))
        output = json.loads(run("swaymsg", "-t", "get_outputs", "-r"))[0]
        assert (output["scale"], output["rect"]["width"], output["rect"]["height"]) == (scale,1280,800)
        for edge in ("top", "bottom"):
            prefix = f"completion-{scale}-{edge}"
            command("cursor.buffer-start" if edge == "top" else "cursor.buffer-end")
            def at_edge():
                image = shot(prefix + "-anchor")
                box = color_box(image, (246, 219, 162))
                if box and abs(box[0] - 64 * scale) <= 2 * scale and image.size == (width, height) and (
                        box[1] < 80 * scale if edge == "top" else box[1] > 700 * scale):
                    return box
            anchor = wait_for(at_edge, f"caret at {edge}")
            indent = b"    " if edge == "bottom" else b""
            if indent:
                type_text(indent.decode())
                def after_indent():
                    caret = color_box(shot(prefix + "-indent"), (246, 219, 162))
                    if caret and caret[0] > anchor[0] + 34 * scale:
                        return caret
                anchor = wait_for(after_indent, "painted prefix origin inside the source line")
            type_text("zx")

            def popup(name):
                def geometry():
                    image = shot(prefix + name)
                    box = color_box(image, (88, 85, 79))
                    selected = color_box(image, (60, 59, 56))
                    if box and selected:
                        return box, selected
                return wait_for(geometry, f"{prefix}{name} candidate rows")

            box, _ = popup("-open")
            assert abs(box[0] - anchor[0]) <= 8 * scale, (box, anchor)
            clear_of_caret = box[1] >= anchor[3] if edge == "top" else box[3] <= anchor[1]
            assert clear_of_caret, (box, anchor)
            type_text("t")
            def narrowed():
                image = shot(prefix + "-narrowed")
                border = color_box(image, (88, 85, 79))
                if border and border[3] - border[1] < box[3] - box[1] - 10 * scale:
                    return border
            smaller = wait_for(narrowed, "typing to narrow the daemon's candidate set")
            assert abs(smaller[0] - anchor[0]) <= 8 * scale
            if edge == "bottom":
                key("n", "ctrl")
                wait_for(lambda: popup("-selected")[1][1] > smaller[1] + 20 * scale,
                         "the daemon to highlight the second item")
                key("Tab")
                saved(original + indent + b"zxtotal")
                key("slash", "ctrl")
                saved(original + indent + b"zxt")
                for _ in indent + b"zxt":
                    key("BackSpace")
            else:
                key("Escape")
                wait_for(lambda: color_box(shot(prefix + "-closed"), (60,59,56)) is None,
                         "completion dismissal")
                type_text("!")
                saved(b"zxt!" + original)
                for _ in "zxt!":
                    key("BackSpace")
            saved(original)
        # The launcher's everyday-file example uses the stock buffer-word provider.
        # Place the prefix far enough right that the list must slide left to fit.
        padding = " " * 84
        run("wtype", "--", padding)
        def far_right():
            caret = color_box(shot(f"completion-{scale}-right-anchor"), (246, 219, 162))
            return caret if caret and caret[0] > 860 * scale else None
        wait_for(far_right, "right-side keyboard caret")
        type_text("format_ce")
        wait_for(lambda: color_box(shot(f"completion-{scale}-ledger-word"), (60, 59, 56)),
                 "the ledger's format_cents suggestion")
        right_box = color_box(shot(f"completion-{scale}-right"), (88, 85, 79))
        assert right_box and width - 8 * scale <= right_box[2] <= width, right_box
        key("Tab")
        saved(original + padding.encode() + b"format_cents")
        key("slash", "ctrl")
        saved(original + padding.encode() + b"format_ce")
        run("wtype", *(["-k", "BackSpace"] * len(padding + "format_ce")))
        saved(original)
    print("PASS: completion opens below and above its byte anchor, filters, accepts once, undoes, and cancels at both scales.")
