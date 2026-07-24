//! OpenType MATH metrics and the math-italic mapping (Tier 3, part one).
//!
//! Framing: `docs/inline-math-slice-framing.md` (rev 3), Q#MS6 / Q#MS7.
//!
//! Two consumers read the same bundled font bytes: cosmic-text draws with it,
//! and this module measures with it. cosmic-text does not expose the MATH
//! table, which is why `ttf-parser` is a direct dependency (Q#MS7) — already
//! in the build graph via `fontdb`, declared with a feature subset that
//! widens nothing.

use ttf_parser::Face;

/// Bundled math font (GUST Font License — see `fonts/GUST-FONT-LICENSE.txt`).
///
/// Distinct from `fonts/OFL.txt`, which covers JetBrains Mono only: Latin
/// Modern Math is GFL, an LPPL-derived licence, not the SIL OFL (framing F6).
pub const LATIN_MODERN_MATH: &[u8] = include_bytes!("../fonts/latinmodern-math.otf");

/// The MATH constants this slice's subset needs, in font units.
///
/// Deliberately narrow: Q#MS2 covers scripts and fractions, so these are the
/// constants those two require. Reading more would be speculative — the
/// values for deferred constructs are only meaningful once they have a
/// consumer (the Q#LX5 discipline, applied to metrics).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathConstants {
    /// Units per em, for scaling everything below into pixels.
    pub units_per_em: u16,
    /// Vertical position of the fraction bar / math axis.
    pub axis_height: i16,
    /// Percentage (0–100) to scale one script level down.
    pub script_percent_scale_down: i16,
    /// Baseline shift for a superscript.
    pub superscript_shift_up: i16,
    /// Baseline shift for a subscript.
    pub subscript_shift_down: i16,
    /// Thickness of the fraction rule.
    pub fraction_rule_thickness: i16,
}

/// Why the bundled font could not supply math metrics.
///
/// Q#MS7: this is a failure of the *math path only* — spans fall back to
/// source and the editor keeps running. It is surfaced rather than swallowed
/// so a bundled-font regression cannot be silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathFontError {
    /// The bytes are not a parseable font.
    Unparseable,
    /// Parsed, but carries no MATH table (e.g. a text-only font).
    NoMathTable,
    /// MATH table present but missing a constant the subset needs.
    MissingConstant(&'static str),
}

impl MathConstants {
    /// Read the subset's constants from font bytes.
    ///
    /// # Errors
    /// [`MathFontError`] when the face, the MATH table, or a needed constant
    /// is absent.
    pub fn from_font_bytes(bytes: &[u8]) -> Result<Self, MathFontError> {
        let face = Face::parse(bytes, 0).map_err(|_| MathFontError::Unparseable)?;
        let math = face.tables().math.ok_or(MathFontError::NoMathTable)?;
        let constants = math
            .constants
            .ok_or(MathFontError::MissingConstant("constants"))?;
        Ok(Self {
            units_per_em: face.units_per_em(),
            axis_height: constants.axis_height().value,
            script_percent_scale_down: constants.script_percent_scale_down(),
            superscript_shift_up: constants.superscript_shift_up().value,
            subscript_shift_down: constants.subscript_shift_down().value,
            fraction_rule_thickness: constants.fraction_rule_thickness().value,
        })
    }

    /// Convert a font-unit value to pixels at `font_size_px`.
    #[must_use]
    pub fn to_px(self, value: i16, font_size_px: f32) -> f32 {
        if self.units_per_em == 0 {
            return 0.0;
        }
        f32::from(value) * font_size_px / f32::from(self.units_per_em)
    }

    /// The per-level script scale, as a fraction (e.g. 0.7).
    #[must_use]
    pub fn script_scale(self) -> f32 {
        let pct = f32::from(self.script_percent_scale_down);
        if pct <= 0.0 { 0.7 } else { pct / 100.0 }
    }
}

/// Map a resolved codepoint to its math-mode presentation form (Q#MS2).
///
/// TeX's convention, which is why uppercase Greek is deliberately upright:
///
/// | Class | Treatment |
/// |---|---|
/// | ASCII letters | math italic, with the U+210E hole for `h` |
/// | Lowercase Greek | math italic |
/// | Uppercase Greek | upright |
/// | Digits, operators | upright |
///
/// Without this, `$x^2$` draws a roman `x` and `$\alpha x$` draws an upright
/// α beside an italic 𝑥 — mixed styles inside one expression (framing F7,
/// R2-2).
#[must_use]
pub fn math_italic(ch: char) -> char {
    // U+210E PLANCK CONSTANT is the italic `h`; the 1D4xx run has a hole
    // there, so mapping arithmetically would produce a reserved codepoint.
    if ch == 'h' {
        return '\u{210E}';
    }
    let mapped = match ch {
        'A'..='Z' => 0x1D434 + (ch as u32 - 'A' as u32),
        'a'..='z' => 0x1D44E + (ch as u32 - 'a' as u32),
        // Lowercase Greek α..ω → MATHEMATICAL ITALIC SMALL ALPHA..OMEGA.
        '\u{3B1}'..='\u{3C9}' => 0x1D6FC + (ch as u32 - 0x3B1),
        // Uppercase Greek, digits, operators: upright, per TeX.
        _ => return ch,
    };
    char::from_u32(mapped).unwrap_or(ch)
}

/// A laid-out expression. Baseline at `y = 0`, positive `y` upward.
///
/// Q#MS6: items carry CHARACTERS, not glyph IDs. Layout still resolves glyph
/// ids internally for advances and bounds — the boundary is on the emitted
/// items, so each is drawable by the existing text machinery. Glyph-id items
/// arrive with stretchy fences and big operators, both deferred.
#[derive(Clone, Debug, PartialEq)]
pub struct MathBox {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub items: Vec<MathItem>,
}

/// One drawable piece of a [`MathBox`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MathItem {
    /// A character at its own size, `baseline` relative to the box baseline.
    Glyph {
        ch: char,
        x: f32,
        baseline: f32,
        size_px: f32,
    },
    /// The fraction bar. Not a glyph — drawn on the existing quad pipeline.
    Rule {
        x: f32,
        y: f32,
        width: f32,
        thickness: f32,
    },
}

impl MathItem {
    fn shifted(self, dx: f32, dy: f32) -> Self {
        match self {
            Self::Glyph {
                ch,
                x,
                baseline,
                size_px,
            } => Self::Glyph {
                ch,
                x: x + dx,
                baseline: baseline + dy,
                size_px,
            },
            Self::Rule {
                x,
                y,
                width,
                thickness,
            } => Self::Rule {
                x: x + dx,
                y: y + dy,
                width,
                thickness,
            },
        }
    }
}

impl MathBox {
    fn empty() -> Self {
        Self {
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
            items: Vec::new(),
        }
    }

    /// Absorb `other` at offset `(dx, dy)`, growing this box's extents.
    fn absorb(&mut self, other: &Self, dx: f32, dy: f32) {
        self.items
            .extend(other.items.iter().map(|item| item.shifted(dx, dy)));
        self.ascent = self.ascent.max(other.ascent + dy);
        self.descent = self.descent.max(other.descent - dy);
    }

    /// Uniformly scale every extent and item (Q#MS10 fit-to-line).
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            width: self.width * factor,
            ascent: self.ascent * factor,
            descent: self.descent * factor,
            items: self
                .items
                .iter()
                .map(|item| match *item {
                    MathItem::Glyph {
                        ch,
                        x,
                        baseline,
                        size_px,
                    } => MathItem::Glyph {
                        ch,
                        x: x * factor,
                        baseline: baseline * factor,
                        size_px: size_px * factor,
                    },
                    MathItem::Rule {
                        x,
                        y,
                        width,
                        thickness,
                    } => MathItem::Rule {
                        x: x * factor,
                        y: y * factor,
                        width: width * factor,
                        thickness: thickness * factor,
                    },
                })
                .collect(),
        }
    }
}

/// The smallest uniform scale the slice will apply before giving up (Q#MS10).
pub const MIN_FIT_SCALE: f32 = 0.6;

/// Scale `boxed` to fit `(ascent_budget, descent_budget)`, or `None` when
/// that would fall below [`MIN_FIT_SCALE`] — in which case Q#MS8 shows the
/// raw source rather than overdrawing into the neighbouring line.
#[must_use]
pub fn fit_to_line(boxed: &MathBox, ascent_budget: f32, descent_budget: f32) -> Option<MathBox> {
    let need_up = boxed.ascent;
    let need_down = boxed.descent;
    let up = if need_up <= 0.0 {
        1.0
    } else {
        ascent_budget / need_up
    };
    let down = if need_down <= 0.0 {
        1.0
    } else {
        descent_budget / need_down
    };
    let scale = up.min(down).min(1.0);
    if scale < MIN_FIT_SCALE {
        return None;
    }
    if scale >= 1.0 {
        return Some(boxed.clone());
    }
    Some(boxed.scaled(scale))
}

/// Lays a [`MathNode`] tree out against the bundled MATH font.
pub struct MathLayout<'a> {
    face: Face<'a>,
    constants: MathConstants,
}

impl<'a> MathLayout<'a> {
    /// Build a layout engine over font bytes.
    ///
    /// # Errors
    /// [`MathFontError`] when the face or its MATH table is unusable.
    pub fn new(bytes: &'a [u8]) -> Result<Self, MathFontError> {
        let face = Face::parse(bytes, 0).map_err(|_| MathFontError::Unparseable)?;
        let constants = MathConstants::from_font_bytes(bytes)?;
        Ok(Self { face, constants })
    }

    #[must_use]
    pub fn constants(&self) -> MathConstants {
        self.constants
    }

    /// Lay `node` out at `size_px`.
    #[must_use]
    pub fn layout(&self, node: &crate::math_parse::MathNode, size_px: f32) -> MathBox {
        use crate::math_parse::MathNode;
        match node {
            MathNode::Char(ch) => self.layout_char(*ch, size_px),
            MathNode::Group(children) => {
                let mut out = MathBox::empty();
                let mut pen = 0.0;
                for child in children {
                    let child_box = self.layout(child, size_px);
                    out.absorb(&child_box, pen, 0.0);
                    pen += child_box.width;
                }
                out.width = pen;
                out
            }
            MathNode::Script { base, sub, sup } => self.layout_script(base, sub, sup, size_px),
            MathNode::Fraction { num, den } => self.layout_fraction(num, den, size_px),
        }
    }

    fn layout_char(&self, ch: char, size_px: f32) -> MathBox {
        let presented = math_italic(ch);
        let upem = f32::from(self.constants.units_per_em.max(1));
        let (advance, ascent, descent) = self
            .face
            .glyph_index(presented)
            .map(|gid| {
                let adv = self
                    .face
                    .glyph_hor_advance(gid)
                    .map_or(0.0, |a| f32::from(a) * size_px / upem);
                // Per-glyph bounds keep boxes tight, which is what makes a
                // fraction's extents honest; fall back to face metrics when
                // a glyph has no bounding box (e.g. a space).
                let (asc, desc) = self.face.glyph_bounding_box(gid).map_or_else(
                    || {
                        (
                            f32::from(self.face.ascender()) * size_px / upem,
                            -f32::from(self.face.descender()) * size_px / upem,
                        )
                    },
                    |bb| {
                        (
                            f32::from(bb.y_max) * size_px / upem,
                            -f32::from(bb.y_min) * size_px / upem,
                        )
                    },
                );
                (adv, asc.max(0.0), desc.max(0.0))
            })
            .unwrap_or((0.0, 0.0, 0.0));
        MathBox {
            width: advance,
            ascent,
            descent,
            items: vec![MathItem::Glyph {
                ch: presented,
                x: 0.0,
                baseline: 0.0,
                size_px,
            }],
        }
    }

    fn layout_script(
        &self,
        base: &crate::math_parse::MathNode,
        sub: &Option<Box<crate::math_parse::MathNode>>,
        sup: &Option<Box<crate::math_parse::MathNode>>,
        size_px: f32,
    ) -> MathBox {
        let base_box = self.layout(base, size_px);
        let script_px = size_px * self.constants.script_scale();
        let mut out = MathBox::empty();
        out.absorb(&base_box, 0.0, 0.0);
        let mut widest = base_box.width;
        if let Some(sup) = sup {
            let sup_box = self.layout(sup, script_px);
            let shift = self
                .constants
                .to_px(self.constants.superscript_shift_up, size_px);
            out.absorb(&sup_box, base_box.width, shift);
            widest = widest.max(base_box.width + sup_box.width);
        }
        if let Some(sub) = sub {
            let sub_box = self.layout(sub, script_px);
            let shift = self
                .constants
                .to_px(self.constants.subscript_shift_down, size_px);
            out.absorb(&sub_box, base_box.width, -shift);
            widest = widest.max(base_box.width + sub_box.width);
        }
        out.width = widest;
        out
    }

    fn layout_fraction(
        &self,
        num: &crate::math_parse::MathNode,
        den: &crate::math_parse::MathNode,
        size_px: f32,
    ) -> MathBox {
        // TeX sets an inline \frac's operands one style down, which is also
        // what the parent framing's Tier 3 specifies (70%). It is load-bearing
        // for Q#MS10: full-size operands would not fit the line at all.
        let operand_px = size_px * self.constants.script_scale();
        let num_box = self.layout(num, operand_px);
        let den_box = self.layout(den, operand_px);
        let axis = self.constants.to_px(self.constants.axis_height, size_px);
        let thickness = self
            .constants
            .to_px(self.constants.fraction_rule_thickness, size_px)
            .max(1.0);
        let gap = thickness * 2.0;

        let width = num_box.width.max(den_box.width);
        let mut out = MathBox::empty();
        // Numerator sits above the bar, denominator below it.
        let num_baseline = axis + thickness / 2.0 + gap + num_box.descent;
        let den_baseline = axis - thickness / 2.0 - gap - den_box.ascent;
        out.absorb(&num_box, (width - num_box.width) / 2.0, num_baseline);
        out.absorb(&den_box, (width - den_box.width) / 2.0, den_baseline);
        out.items.push(MathItem::Rule {
            x: 0.0,
            y: axis,
            width,
            thickness,
        });
        out.ascent = out.ascent.max(axis + thickness / 2.0);
        out.descent = out.descent.max(-(axis - thickness / 2.0));
        out.width = width;
        out
    }
}

/// Spacer text reserving `width_px`, quantized UP to whole space advances.
///
/// Q#MS4 / B1': a `RichChunk`'s only width is its text, so a suppressed math
/// span reserves room the way `SourceTab` does — with spaces. Quantizing up
/// is deliberate: it keeps the projection grid-aligned with the surrounding
/// monospace text and keeps hit runs integral, at the cost of up to one
/// advance of slack on the right of the box.
#[must_use]
pub fn spacer_for_width(width_px: f32, space_advance_px: f32) -> String {
    if !width_px.is_finite() || width_px <= 0.0 || space_advance_px <= 0.0 {
        return String::new();
    }
    let n = (width_px / space_advance_px).ceil();
    // Guard the cast: a pathological advance must not mint a giant string.
    let n = n.clamp(0.0, 4096.0) as usize;
    " ".repeat(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::math_parse::parse;

    #[test]
    fn spacer_quantizes_up_to_whole_advances() {
        // Exactly two advances stays two; a sliver over rounds up, so the
        // box never overlaps the text that follows it.
        assert_eq!(spacer_for_width(20.0, 10.0).len(), 2);
        assert_eq!(spacer_for_width(20.1, 10.0).len(), 3);
        assert_eq!(spacer_for_width(0.1, 10.0).len(), 1);
        // Degenerate inputs reserve nothing rather than panicking or
        // minting an enormous string.
        assert!(spacer_for_width(0.0, 10.0).is_empty());
        assert!(spacer_for_width(-5.0, 10.0).is_empty());
        assert!(spacer_for_width(10.0, 0.0).is_empty());
        assert!(spacer_for_width(f32::NAN, 10.0).is_empty());
        assert!(spacer_for_width(f32::INFINITY, 10.0).is_empty());
        assert!(spacer_for_width(1e9, 0.001).len() <= 4096);
    }

    #[test]
    fn a_real_box_reserves_at_least_its_own_width() {
        let boxed = lay(r"\frac{a}{b}", crate::BASE_CODE_FONT_SIZE);
        let advance = 9.6_f32; // a plausible monospace advance at 16 px
        let spacer = spacer_for_width(boxed.width, advance);
        let reserved = spacer.len() as f32 * advance;
        assert!(
            reserved >= boxed.width,
            "reserved {reserved} must cover box width {}",
            boxed.width
        );
        assert!(
            reserved - boxed.width < advance,
            "slack stays under one advance"
        );
    }

    fn engine() -> MathLayout<'static> {
        MathLayout::new(LATIN_MODERN_MATH).expect("bundled font")
    }

    fn lay(src: &str, size: f32) -> MathBox {
        let node = parse(src).expect("parses");
        engine().layout(&node, size)
    }

    /// Framing acceptance 3, including its bite: the MATH constant must be
    /// READ, not hardcoded.
    #[test]
    fn superscript_is_raised_and_scaled_from_the_math_table() {
        let plain = lay("x", 16.0);
        let script = lay("x^2", 16.0);
        assert!(script.width > plain.width, "the 2 adds width");
        assert!(
            script.ascent > plain.ascent,
            "superscript must raise the box: {} vs {}",
            script.ascent,
            plain.ascent
        );
        let two = script
            .items
            .iter()
            .find_map(|i| match *i {
                MathItem::Glyph {
                    ch,
                    baseline,
                    size_px,
                    ..
                } if ch == '2' => Some((baseline, size_px)),
                _ => None,
            })
            .expect("the 2 is emitted");
        assert!(two.0 > 0.0, "raised above baseline: {}", two.0);
        assert!(two.1 < 16.0, "scaled down: {}", two.1);

        // Bite: with the script scale stubbed to 100%, the box changes —
        // proving the constant is consulted rather than assumed.
        let c = engine().constants();
        assert!(
            c.script_percent_scale_down < 100,
            "font advertises a real script scale ({}%), so 100% is a \
             meaningful stub",
            c.script_percent_scale_down
        );
        let stubbed = MathConstants {
            script_percent_scale_down: 100,
            ..c
        };
        assert!(
            (stubbed.script_scale() - c.script_scale()).abs() > 0.01,
            "stubbing the constant must change the scale actually used"
        );
    }

    #[test]
    fn subscript_drops_below_the_baseline() {
        let script = lay("x_i", 16.0);
        let i = script
            .items
            .iter()
            .find_map(|item| match *item {
                MathItem::Glyph { ch, baseline, .. } if ch == math_italic('i') => Some(baseline),
                _ => None,
            })
            .expect("the i is emitted");
        assert!(i < 0.0, "subscript sits below the baseline: {i}");
        assert!(script.descent > lay("x", 16.0).descent);
    }

    /// Framing acceptance 4.
    #[test]
    fn fraction_stacks_operands_around_a_rule_at_the_axis() {
        let frac = lay(r"\frac{a}{b}", 16.0);
        let rule = frac
            .items
            .iter()
            .find_map(|item| match *item {
                MathItem::Rule {
                    y,
                    width,
                    thickness,
                    ..
                } => Some((y, width, thickness)),
                MathItem::Glyph { .. } => None,
            })
            .expect("a fraction draws a rule");
        assert!(rule.0 > 0.0, "rule sits at the math axis, above baseline");
        assert!(rule.2 > 0.0 && rule.1 > 0.0);

        let mut above = 0;
        let mut below = 0;
        for item in &frac.items {
            if let MathItem::Glyph { baseline, .. } = *item {
                if baseline > rule.0 {
                    above += 1;
                } else if baseline < rule.0 {
                    below += 1;
                }
            }
        }
        assert_eq!((above, below), (1, 1), "one operand each side of the bar");
        assert!(frac.ascent > 0.0 && frac.descent > 0.0);
    }

    /// F1 / B6 — the height budget, computed rather than guessed.
    ///
    /// The round-2 review warned that acceptance 12's fallback case must be
    /// derived by computation or it would "surprise-pass by rendering". It
    /// was right, and rev 3's guess was wrong: a doubly-nested fraction still
    /// fits. This test derives the budget the way Q#MS10 defines it — from
    /// the LINE BOX, whose baseline the CODE font places — and then searches
    /// for the depth that actually trips the floor, so the case can never
    /// drift out from under the acceptance criterion.
    #[test]
    fn fit_to_line_admits_real_fractions_and_finds_the_true_fallback_depth() {
        // Q#MS10: the budget is the line box less a one-pixel margin, split
        // at the text baseline. The baseline is where the CODE font puts it
        // (JetBrains Mono at BASE_CODE_FONT_SIZE inside BASE_CODE_LINE_HEIGHT),
        // NOT where the math font's own metrics would.
        let code = Face::parse(crate::JETBRAINS_MONO, 0).expect("code face");
        let code_upem = f32::from(code.units_per_em());
        let baseline_from_top = f32::from(code.ascender()) * crate::BASE_CODE_FONT_SIZE / code_upem;
        let margin = 1.0;
        let asc_budget = baseline_from_top - margin;
        let desc_budget = crate::BASE_CODE_LINE_HEIGHT - baseline_from_top - margin;
        assert!(
            asc_budget > 0.0 && desc_budget > 0.0,
            "budget must be positive: {asc_budget} / {desc_budget}"
        );

        let scale_of = |src: &str| {
            let boxed = lay(src, crate::BASE_CODE_FONT_SIZE);
            let up = asc_budget / boxed.ascent.max(f32::EPSILON);
            let down = desc_budget / boxed.descent.max(f32::EPSILON);
            (up.min(down).min(1.0), boxed)
        };

        // The flagship cases must RENDER, not fall back (B6).
        for src in [r"\frac{a}{b}", r"\frac{x^2}{y}", "x^2", r"\alpha x"] {
            let (scale, boxed) = scale_of(src);
            eprintln!(
                "{src}: asc={:.2} desc={:.2} scale={scale:.3}",
                boxed.ascent, boxed.descent
            );
            assert!(
                scale >= MIN_FIT_SCALE,
                "{src} must render, not fall back: scale {scale:.3} < {MIN_FIT_SCALE}"
            );
            assert!(fit_to_line(&boxed, asc_budget, desc_budget).is_some());
        }

        // Now FIND the depth that trips the floor rather than assuming one.
        // Nest fractions until the scale drops below it.
        let mut src = String::from(r"\frac{a}{b}");
        let mut depth = 1;
        let tripped = loop {
            let (scale, _) = scale_of(&src);
            eprintln!("depth {depth}: scale={scale:.3}");
            if scale < MIN_FIT_SCALE {
                break Some((depth, src.clone()));
            }
            if depth >= 6 {
                break None;
            }
            src = format!(r"\frac{{{src}}}{{c}}");
            depth += 1;
        };
        let (depth, deep_src) = tripped.expect(
            "some nesting depth must exceed the floor, or Q#MS10's fallback \
             arm is unreachable and the floor is dead code",
        );
        assert!(
            depth > 2,
            "rev 3 guessed a doubly-nested fraction would fall back; the real \
             depth is {depth}, so acceptance 12 must use that case"
        );
        assert!(
            fit_to_line(
                &lay(&deep_src, crate::BASE_CODE_FONT_SIZE),
                asc_budget,
                desc_budget
            )
            .is_none()
        );
    }

    #[test]
    fn fitting_scales_extents_and_items_together() {
        let boxed = lay(r"\frac{a}{b}", 16.0);
        let half = boxed.scaled(0.5);
        assert!((half.ascent - boxed.ascent * 0.5).abs() < 0.001);
        assert!((half.width - boxed.width * 0.5).abs() < 0.001);
        for (before, after) in boxed.items.iter().zip(half.items.iter()) {
            if let (MathItem::Glyph { size_px: b, .. }, MathItem::Glyph { size_px: a, .. }) =
                (before, after)
            {
                assert!((a - b * 0.5).abs() < 0.001, "glyph size scales too");
            }
        }
    }

    #[test]
    fn a_group_advances_the_pen_left_to_right() {
        let boxed = lay("abc", 16.0);
        let xs: Vec<f32> = boxed
            .items
            .iter()
            .filter_map(|item| match *item {
                MathItem::Glyph { x, .. } => Some(x),
                MathItem::Rule { .. } => None,
            })
            .collect();
        assert_eq!(xs.len(), 3);
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "left to right: {xs:?}");
        assert!(boxed.width > xs[2], "width covers the last advance");
    }

    /// B5 — `ttf-parser` supplies every constant the subset needs, from the
    /// bundled font. This is the bet that would sink Tier 3 if false, so it
    /// runs against the real embedded bytes rather than a fixture.
    #[test]
    fn bundled_font_yields_every_math_constant_the_subset_needs() {
        let c = MathConstants::from_font_bytes(LATIN_MODERN_MATH)
            .expect("bundled Latin Modern Math must expose MATH constants");
        assert_eq!(c.units_per_em, 1000, "LM Math is a 1000 upem font");
        assert!(c.axis_height > 0, "axis height: {}", c.axis_height);
        assert!(
            (50..=100).contains(&c.script_percent_scale_down),
            "script scale percent out of range: {}",
            c.script_percent_scale_down
        );
        assert!(c.superscript_shift_up > 0);
        assert!(c.subscript_shift_down > 0);
        assert!(c.fraction_rule_thickness > 0);
    }

    #[test]
    fn a_text_font_without_a_math_table_is_rejected_not_defaulted() {
        // Q#MS7: a font with no MATH table must surface, not silently
        // produce plausible-looking zeros.
        let err = MathConstants::from_font_bytes(crate::JETBRAINS_MONO)
            .expect_err("JetBrains Mono has no MATH table");
        assert_eq!(err, MathFontError::NoMathTable);
        assert_eq!(
            MathConstants::from_font_bytes(b"not a font"),
            Err(MathFontError::Unparseable)
        );
    }

    #[test]
    fn font_units_convert_to_pixels_against_upem() {
        let c = MathConstants::from_font_bytes(LATIN_MODERN_MATH).expect("constants");
        // Half an em at 16 px is 8 px.
        let half_em = i16::try_from(c.units_per_em / 2).expect("fits");
        assert!((c.to_px(half_em, 16.0) - 8.0).abs() < 0.01);
        let scale = c.script_scale();
        assert!((0.5..=1.0).contains(&scale), "script scale: {scale}");
    }

    #[test]
    fn math_italic_follows_tex_convention_including_the_planck_hole() {
        // Framing acceptance 13.
        assert_eq!(math_italic('x'), '\u{1D465}');
        assert_eq!(math_italic('A'), '\u{1D434}');
        // The 1D4xx run has a hole at italic `h`; arithmetic would land on a
        // reserved codepoint, so `h` maps to U+210E instead.
        assert_eq!(math_italic('h'), '\u{210E}');
        // Lowercase Greek is italic...
        assert_eq!(math_italic('α'), '\u{1D6FC}');
        assert_eq!(math_italic('ω'), '\u{1D714}');
        // ...uppercase Greek is NOT (TeX convention, deliberate).
        assert_eq!(math_italic('Γ'), 'Γ');
        assert_eq!(math_italic('Ω'), 'Ω');
        // Digits and operators stay upright.
        assert_eq!(math_italic('2'), '2');
        assert_eq!(math_italic('+'), '+');
    }

    #[test]
    fn every_italic_mapping_lands_on_a_real_glyph_in_the_bundled_font() {
        // A mapping that produces codepoints the bundled font cannot draw
        // would render tofu — worse than the roman fallback it replaced.
        let face = Face::parse(LATIN_MODERN_MATH, 0).expect("parse bundled font");
        let sample = "abhxyzABXYZαβωΓΩ0129+=";
        for ch in sample.chars() {
            let mapped = math_italic(ch);
            assert!(
                face.glyph_index(mapped).is_some(),
                "no glyph for {ch:?} -> {mapped:?} (U+{:04X})",
                mapped as u32
            );
        }
    }
}
