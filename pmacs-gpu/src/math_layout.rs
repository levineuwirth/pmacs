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

#[cfg(test)]
mod tests {
    use super::*;

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
