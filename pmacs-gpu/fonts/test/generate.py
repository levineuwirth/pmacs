#!/usr/bin/env python3
"""Generate the four hermetic test faces for the pmacs-gpu font tests.

The gpu-set-font acceptance suite (docs/gpu-set-font-framing.md) needs
family-routing tests that cannot depend on whatever fonts the host has
installed, so these tiny fixture faces are generated and committed:

  PmacsTestMonoTwo-Regular.ttf      "Pmacs Test Mono Two"     monospaced,
                                    advance 720/1000 (JetBrains Mono is
                                    600/1000, so the measured advance
                                    ratio is exactly 1.2)
  PmacsTestProportional-Regular.ttf "Pmacs Test Proportional" varying
                                    advances, not monospaced
  PmacsTestFamily-Regular.ttf       "Pmacs Test Family"       monospaced
                                    normal face, advance 800/1000
  PmacsTestFamily-Bold.ttf          "Pmacs Test Family"       BOLD and
                                    proportional -- the same-family
                                    collision the sanitizer removes and
                                    the four-style monospace gate must
                                    reject

Every glyph is a plain rectangle (ink for frame-diff tests); coverage
is space, the digits (the ADVANCE_PROBE string), and a-z. fontdb's
`monospaced` flag reads the post table's isFixedPitch, so that is the
one bit that decides mono vs proportional here.

Run from this directory:  python3 generate.py
Requires fontTools (any recent version).
"""

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

UPM = 1000
CHARS = " 0123456789abcdefghijklmnopqrstuvwxyz"
ASCENT = 800
DESCENT = -200


def glyph_name(char):
    return "uni%04X" % ord(char)


def rect_glyph(advance):
    """A filled rectangle spanning most of the advance width."""
    pen = TTGlyphPen(None)
    left = 60
    right = max(left + 40, advance - 60)
    pen.moveTo((left, 0))
    pen.lineTo((right, 0))
    pen.lineTo((right, 700))
    pen.lineTo((left, 700))
    pen.closePath()
    return pen.glyph()


def empty_glyph():
    return TTGlyphPen(None).glyph()


def build(path, family, style, weight, bold, fixed_pitch, advance_for):
    order = [".notdef"] + [glyph_name(c) for c in CHARS]
    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({ord(c): glyph_name(c) for c in CHARS})
    glyphs = {".notdef": rect_glyph(600)}
    metrics = {".notdef": (600, 60)}
    for c in CHARS:
        adv = advance_for(c)
        name = glyph_name(c)
        glyphs[name] = empty_glyph() if c == " " else rect_glyph(adv)
        metrics[name] = (adv, 0 if c == " " else 60)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=ASCENT, descent=DESCENT)
    # fontdb refuses faces without a PostScript name (nameID 6).
    ps_name = (family + "-" + style).replace(" ", "")
    fb.setupNameTable({"familyName": family, "styleName": style, "psName": ps_name})
    fb.setupOS2(
        sTypoAscender=ASCENT,
        sTypoDescender=DESCENT,
        usWinAscent=ASCENT,
        usWinDescent=-DESCENT,
        usWeightClass=weight,
        fsSelection=0x20 if bold else 0x40,  # BOLD else REGULAR
    )
    fb.setupPost(isFixedPitch=1 if fixed_pitch else 0)
    if bold:
        fb.font["head"].macStyle = 0x01
    fb.save(path)
    print("wrote", path)


def proportional_advance(c):
    if c == " ":
        return 250
    if c.isdigit():
        return 500
    # A spread of widths so no two adjacent letters share one.
    return 300 + (ord(c) - ord("a")) * 15


build(
    "PmacsTestMonoTwo-Regular.ttf",
    "Pmacs Test Mono Two",
    "Regular",
    400,
    bold=False,
    fixed_pitch=True,
    advance_for=lambda c: 720,
)
build(
    "PmacsTestProportional-Regular.ttf",
    "Pmacs Test Proportional",
    "Regular",
    400,
    bold=False,
    fixed_pitch=False,
    advance_for=proportional_advance,
)
build(
    "PmacsTestFamily-Regular.ttf",
    "Pmacs Test Family",
    "Regular",
    400,
    bold=False,
    fixed_pitch=True,
    advance_for=lambda c: 800,
)
build(
    "PmacsTestFamily-Bold.ttf",
    "Pmacs Test Family",
    "Bold",
    700,
    bold=True,
    fixed_pitch=False,
    advance_for=proportional_advance,
)
