//! LaTeX math-mode parser for the first inline-math slice.
//!
//! Framing: `docs/archive/framings/inline-math-slice-framing.md` (rev 3), Q#MS2. This parses
//! the deliberately small subset the slice renders — characters, groups,
//! sub/superscripts and fractions — and nothing else. Every other LaTeX
//! construct is an error, which Q#MS8 turns into "show the raw source".
//!
//! The AST is *semantic*, not presentational: `\alpha` resolves to `'α'`
//! here, but the math-italic mapping (Q#MS2's table) belongs to layout, which
//! is where a codepoint becomes a glyph. Keeping the split here means the AST
//! matches what the user wrote, and a future non-italic style context does
//! not have to unpick a decision the parser baked in.

/// One node of the slice's math subset (Q#MS2).
///
/// Rev 2 of the framing folded `Symbol` into `Char`: both carried a `char`,
/// and after symbol resolution layout cannot act on the difference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MathNode {
    /// A resolved codepoint: `x`, `2`, `+`, `α`.
    Char(char),
    /// A braced group, or the top-level expression.
    Group(Vec<MathNode>),
    /// A base with optional sub- and superscript.
    Script {
        base: Box<MathNode>,
        sub: Option<Box<MathNode>>,
        sup: Option<Box<MathNode>>,
    },
    /// `\frac{num}{den}`.
    Fraction {
        num: Box<MathNode>,
        den: Box<MathNode>,
    },
}

/// Why a span could not be parsed. Q#MS8 renders the raw source for all of
/// these; the variants exist so tests can assert *which* rejection fired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MathParseError {
    /// The span held no math (`$$` after delimiter stripping).
    Empty,
    /// A `{` with no matching `}`, or a stray `}`.
    UnbalancedBrace,
    /// A control sequence outside the subset, e.g. `\sqrt`.
    UnknownCommand(String),
    /// `\frac` without two braced arguments.
    MalformedCommand(&'static str),
    /// `^` or `_` with nothing to attach to, or given twice for one base.
    MalformedScript(&'static str),
    /// A `$` inside the span: the delimiters are the caller's business, and
    /// a bare one here means detection handed us something it should not
    /// have (framing acceptance 15 — `$$x$$` degrades through this path).
    UnexpectedDollar,
}

/// Greek seed map (Q#MS2). Deliberately partial — growing it is mechanical.
const GREEK: &[(&str, char)] = &[
    ("alpha", 'α'),
    ("beta", 'β'),
    ("gamma", 'γ'),
    ("delta", 'δ'),
    // TeX's \epsilon is LUNATE (U+03F5); U+03B5 is \varepsilon.
    ("epsilon", '\u{3F5}'),
    ("zeta", 'ζ'),
    ("eta", 'η'),
    ("theta", 'θ'),
    ("iota", 'ι'),
    ("kappa", 'κ'),
    ("lambda", 'λ'),
    ("mu", 'μ'),
    ("nu", 'ν'),
    ("xi", 'ξ'),
    ("pi", 'π'),
    ("rho", 'ρ'),
    ("sigma", 'σ'),
    ("tau", 'τ'),
    ("upsilon", 'υ'),
    // TeX's \phi is U+03D5; U+03C6 is \varphi.
    ("phi", '\u{3D5}'),
    ("chi", 'χ'),
    ("psi", 'ψ'),
    ("omega", 'ω'),
    ("Gamma", 'Γ'),
    ("Delta", 'Δ'),
    ("Theta", 'Θ'),
    ("Lambda", 'Λ'),
    ("Xi", 'Ξ'),
    ("Pi", 'Π'),
    ("Sigma", 'Σ'),
    ("Upsilon", 'Υ'),
    ("Phi", 'Φ'),
    ("Psi", 'Ψ'),
    ("Omega", 'Ω'),
];

/// Parse the *interior* of a math span — delimiters already stripped.
///
/// # Errors
/// Returns [`MathParseError`] for anything outside the Q#MS2 subset.
pub fn parse(source: &str) -> Result<MathNode, MathParseError> {
    let mut parser = Parser {
        chars: source.chars().collect(),
        pos: 0,
    };
    let nodes = parser.parse_sequence(None)?;
    if parser.pos < parser.chars.len() {
        // Only a stray `}` can stop the top-level sequence early.
        return Err(MathParseError::UnbalancedBrace);
    }
    if nodes.is_empty() {
        return Err(MathParseError::Empty);
    }
    Ok(MathNode::Group(nodes))
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    /// Parse until `close` (or end of input when `None`).
    fn parse_sequence(&mut self, close: Option<char>) -> Result<Vec<MathNode>, MathParseError> {
        let mut out: Vec<MathNode> = Vec::new();
        loop {
            // Whitespace is insignificant in math mode, and it must be
            // skipped HERE rather than inside `parse_atom`: the `^`/`_`
            // dispatch below happens before atoms are read, so leaving a
            // space in front of a marker would make `x ^ 2` parse the caret
            // as a literal character.
            while self.peek().is_some_and(char::is_whitespace) {
                self.pos += 1;
            }
            match self.peek() {
                None => {
                    if close.is_some() {
                        return Err(MathParseError::UnbalancedBrace);
                    }
                    return Ok(out);
                }
                Some(ch) if Some(ch) == close => {
                    self.pos += 1;
                    return Ok(out);
                }
                // A `}` we were not asked to stop at is unbalanced.
                Some('}') => return Err(MathParseError::UnbalancedBrace),
                Some('$') => return Err(MathParseError::UnexpectedDollar),
                Some('^' | '_') => {
                    let base = out.pop().ok_or(MathParseError::MalformedScript(
                        "sub/superscript with no base",
                    ))?;
                    out.push(self.parse_scripts(base)?);
                }
                Some(_) => {
                    let atom = self.parse_atom()?;
                    out.push(atom);
                }
            }
        }
    }

    /// One atom: a group, a command, or a single character.
    fn parse_atom(&mut self) -> Result<MathNode, MathParseError> {
        match self.bump() {
            Some('{') => Ok(MathNode::Group(self.parse_sequence(Some('}'))?)),
            Some('\\') => self.parse_command(),
            Some(ch) => Ok(MathNode::Char(ch)),
            None => Err(MathParseError::MalformedCommand("unexpected end of input")),
        }
    }

    fn parse_command(&mut self) -> Result<MathNode, MathParseError> {
        let mut name = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphabetic() {
                name.push(ch);
                self.pos += 1;
            } else {
                break;
            }
        }
        if name.is_empty() {
            // `\$`, `\{` … — an escaped literal.
            return match self.bump() {
                Some(ch) => Ok(MathNode::Char(ch)),
                None => Err(MathParseError::MalformedCommand("trailing backslash")),
            };
        }
        if name == "frac" {
            let num = self.parse_required_group("\\frac numerator")?;
            let den = self.parse_required_group("\\frac denominator")?;
            return Ok(MathNode::Fraction {
                num: Box::new(num),
                den: Box::new(den),
            });
        }
        if let Some((_, ch)) = GREEK.iter().find(|(n, _)| *n == name) {
            return Ok(MathNode::Char(*ch));
        }
        Err(MathParseError::UnknownCommand(name))
    }

    /// A `{…}` argument, skipping leading whitespace.
    fn parse_required_group(&mut self, what: &'static str) -> Result<MathNode, MathParseError> {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
        match self.peek() {
            Some('{') => {
                self.pos += 1;
                Ok(MathNode::Group(self.parse_sequence(Some('}'))?))
            }
            _ => Err(MathParseError::MalformedCommand(what)),
        }
    }

    /// Attach `^`/`_` to `base`, in either order, at most one each.
    fn parse_scripts(&mut self, base: MathNode) -> Result<MathNode, MathParseError> {
        let mut sub: Option<Box<MathNode>> = None;
        let mut sup: Option<Box<MathNode>> = None;
        loop {
            // Whitespace is insignificant, here too: without this skip
            // `x^2 _i` builds a nested Script instead of one merged double
            // script (drawing the subscript displaced right by the
            // superscript's width), and `x^2 ^3` parses where TeX errors.
            let resume = self.pos;
            while self.peek().is_some_and(char::is_whitespace) {
                self.pos += 1;
            }
            let Some(marker @ ('^' | '_')) = self.peek() else {
                self.pos = resume;
                break;
            };
            self.pos += 1;
            let slot = self.parse_script_operand()?;
            match marker {
                '^' if sup.is_some() => {
                    return Err(MathParseError::MalformedScript("double superscript"));
                }
                '_' if sub.is_some() => {
                    return Err(MathParseError::MalformedScript("double subscript"));
                }
                '^' => sup = Some(Box::new(slot)),
                _ => sub = Some(Box::new(slot)),
            }
        }
        Ok(MathNode::Script {
            base: Box::new(base),
            sub,
            sup,
        })
    }

    /// The operand of `^`/`_`: a braced group, or exactly one atom.
    fn parse_script_operand(&mut self) -> Result<MathNode, MathParseError> {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
        match self.peek() {
            None | Some('^' | '_' | '}') => {
                Err(MathParseError::MalformedScript("script with no operand"))
            }
            Some(_) => self.parse_atom(),
        }
    }
}

/// One detected inline span, as byte offsets into the scanned line.
///
/// `start`/`end` bracket the WHOLE span including both `$` delimiters, which
/// is what Q#MS4 suppresses; [`Self::interior`] is what the parser sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathSpan {
    pub start: usize,
    pub end: usize,
}

impl MathSpan {
    /// Byte range of the math source between the delimiters.
    #[must_use]
    pub fn interior(self) -> std::ops::Range<usize> {
        self.start + 1..self.end - 1
    }
}

/// Find inline `$…$` spans in ONE line (Q#MS3).
///
/// Spans never cross a newline: chunking is per line and the visible slice is
/// line-ranged, so single-line spans are what keep visible-slice-scoped
/// scanning stable under scroll. Callers pass one line at a time.
///
/// Currency guards are mandatory, not a refinement (framing F5). Without
/// them `prices are $5 and $6 today` pairs the two `$` and renders `5 and `
/// as math — in exactly the grammar-less prose buffers this scanner targets.
/// Pandoc's rule:
///
/// - an opening `$` must be followed by a non-space;
/// - a closing `$` must be preceded by a non-space and not followed by a digit;
/// - `\$` is an escape and neither opens nor closes.
#[must_use]
pub fn detect_math_spans(line: &str) -> Vec<MathSpan> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut open: Option<usize> = None;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Skip the escaped byte: `\$` is literal, so it can neither open
            // nor close. Stepping two also stops `\\$` from being read as an
            // escape of the dollar.
            i += 2;
            continue;
        }
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        if bytes.get(i + 1) == Some(&b'$') {
            // `$$` is display math, which this slice defers. It is NOT two
            // inline delimiters: reading it that way makes `$$x$$` match the
            // inner `$x$`, which parses, so the span would half-render as
            // math with a stray `$` on each side. Acceptance 15 requires it
            // to degrade to source, so `$$` is opaque — it neither opens nor
            // closes, and abandons any pending opener.
            i += 2;
            open = None;
            continue;
        }
        match open {
            None => {
                // Opener: next byte must exist and be a non-space.
                let opens = bytes
                    .get(i + 1)
                    .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'$');
                if opens {
                    open = Some(i);
                }
            }
            Some(start) => {
                let prev_ok = i > start + 1 && !bytes[i - 1].is_ascii_whitespace();
                let next_ok = bytes.get(i + 1).is_none_or(|b| !b.is_ascii_digit());
                if prev_ok && next_ok {
                    spans.push(MathSpan { start, end: i + 1 });
                    open = None;
                } else if !prev_ok {
                    // `$foo $` — the closer is disqualified by the space
                    // before it. Treat this `$` as a fresh opener candidate
                    // rather than letting the span run to the next one.
                    open = bytes
                        .get(i + 1)
                        .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'$')
                        .then_some(i);
                }
            }
        }
        i += 1;
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char) -> MathNode {
        MathNode::Char(c)
    }

    fn group(nodes: Vec<MathNode>) -> MathNode {
        MathNode::Group(nodes)
    }

    #[test]
    fn detection_finds_inline_spans() {
        assert_eq!(
            detect_math_spans("$x^2$"),
            vec![MathSpan { start: 0, end: 5 }]
        );
        let two = detect_math_spans("$a$ and $b$");
        assert_eq!(two.len(), 2, "{two:?}");
        let line = "before $x^2$ after";
        let span = detect_math_spans(line)[0];
        assert_eq!(&line[span.start..span.end], "$x^2$");
        assert_eq!(&line[span.interior()], "x^2");
    }

    /// Framing F5 / acceptance 2 — the case rev 1's rule would have
    /// mis-rendered as math over "5 and ".
    #[test]
    fn currency_guards_reject_prose_dollars() {
        assert!(detect_math_spans("Price: $5.00").is_empty());
        assert!(
            detect_math_spans("prices are $5 and $6 today").is_empty(),
            "a digit after the closer disqualifies it"
        );
        assert!(
            detect_math_spans("$ x $").is_empty(),
            "space after the opener disqualifies it"
        );
        assert!(
            detect_math_spans("costs $5 or $6").is_empty(),
            "both guards together"
        );
    }

    #[test]
    fn escaped_dollars_neither_open_nor_close() {
        assert!(detect_math_spans(r"\$5 and \$6").is_empty());
        // An escaped dollar inside a span does not close it.
        let line = r"$a\$b$";
        let spans = detect_math_spans(line);
        assert_eq!(spans.len(), 1);
        assert_eq!(&line[spans[0].start..spans[0].end], r"$a\$b$");
    }

    #[test]
    fn an_unpaired_dollar_yields_nothing() {
        assert!(detect_math_spans("$x").is_empty());
        assert!(detect_math_spans("x$").is_empty());
        // Q#MS3: spans never cross a newline. Callers scan per line, so a
        // partner on the next line is simply not visible to this call.
        assert!(detect_math_spans("$x").is_empty());
        assert!(detect_math_spans("y$").is_empty());
    }

    #[test]
    fn empty_and_display_delimiters_degrade_rather_than_half_match() {
        // Acceptance 15. `$$` is opaque, so display math yields NO span and
        // falls through to source. Asserting emptiness rather than "any span
        // found must fail to parse" matters: the interior of the inner `$x$`
        // parses perfectly well, so the weaker form passed vacuously while
        // `$$x$$` half-rendered with a stray `$` on each side.
        assert!(detect_math_spans("$$").is_empty());
        assert!(
            detect_math_spans("$$x$$").is_empty(),
            "display math must not match the inner $x$"
        );
        assert!(detect_math_spans(r"$$\frac{a}{b}$$").is_empty());
        // A real inline span beside display math is still found.
        let mixed = detect_math_spans("$a$ then $$b$$");
        assert_eq!(mixed.len(), 1, "{mixed:?}");
    }

    /// Round-3 F6 — a DOCUMENTED casualty of the `$$`-opaque rule, not a
    /// guard failure: in `$a$$b$` the first span's legitimate closer is
    /// immediately followed by the second span's opener, the lookahead
    /// reads that pair as display-math `$$`, and the pending opener is
    /// abandoned. Adjacent inline spans therefore need a separating
    /// character. Pandoc finds two spans here; this scanner deliberately
    /// finds none, because distinguishing `$a$$b$` from `$$x$$` requires
    /// closer-context the framing's opaque-`$$` rule gave away.
    #[test]
    fn adjacent_inline_spans_are_eaten_by_the_display_guard() {
        assert!(detect_math_spans("$a$$b$").is_empty());
        assert!(detect_math_spans("$x^2$$y^2$").is_empty());
        // One separating character restores both spans.
        assert_eq!(detect_math_spans("$a$ $b$").len(), 2);
    }

    #[test]
    fn whitespace_before_a_script_marker_still_merges_the_scripts() {
        // F2: without skipping whitespace in `parse_scripts`, `x^2 _i` built
        // a NESTED script and drew the subscript displaced right.
        assert_eq!(parse("x^2 _i"), parse("x^2_i"));
        assert_eq!(parse("x _i ^2"), parse("x_i^2"));
        // And a doubled script is still an error with space between.
        assert!(matches!(
            parse("x^2 ^3"),
            Err(MathParseError::MalformedScript(_))
        ));
    }

    #[test]
    fn the_greek_seed_uses_tex_letter_forms() {
        // F5: TeX's \epsilon is lunate and \phi is the symbol form; the
        // U+03B5 / U+03C6 glyphs are \varepsilon / \varphi.
        assert_eq!(parse(r"\epsilon"), Ok(group(vec![ch('\u{3F5}')])));
        assert_eq!(parse(r"\phi"), Ok(group(vec![ch('\u{3D5}')])));
    }

    #[test]
    fn plain_characters_parse_in_order() {
        assert_eq!(parse("x+1"), Ok(group(vec![ch('x'), ch('+'), ch('1')])));
    }

    #[test]
    fn superscript_and_subscript_attach_to_the_preceding_atom() {
        // Framing acceptance 1.
        assert_eq!(
            parse("x^2"),
            Ok(group(vec![MathNode::Script {
                base: Box::new(ch('x')),
                sub: None,
                sup: Some(Box::new(ch('2'))),
            }]))
        );
        assert_eq!(
            parse("x_i"),
            Ok(group(vec![MathNode::Script {
                base: Box::new(ch('x')),
                sub: Some(Box::new(ch('i'))),
                sup: None,
            }]))
        );
    }

    #[test]
    fn both_scripts_parse_in_either_order() {
        let expected = MathNode::Script {
            base: Box::new(ch('x')),
            sub: Some(Box::new(ch('i'))),
            sup: Some(Box::new(ch('2'))),
        };
        assert_eq!(parse("x_i^2"), Ok(group(vec![expected.clone()])));
        assert_eq!(parse("x^2_i"), Ok(group(vec![expected])));
    }

    #[test]
    fn braced_script_operands_group() {
        assert_eq!(
            parse("x^{i+1}"),
            Ok(group(vec![MathNode::Script {
                base: Box::new(ch('x')),
                sub: None,
                sup: Some(Box::new(group(vec![ch('i'), ch('+'), ch('1')]))),
            }]))
        );
    }

    #[test]
    fn fraction_takes_two_braced_arguments() {
        assert_eq!(
            parse(r"\frac{a}{b}"),
            Ok(group(vec![MathNode::Fraction {
                num: Box::new(group(vec![ch('a')])),
                den: Box::new(group(vec![ch('b')])),
            }]))
        );
    }

    #[test]
    fn fractions_nest() {
        // Framing acceptance 1 and 12's over-tall candidate.
        let inner = MathNode::Fraction {
            num: Box::new(group(vec![ch('a')])),
            den: Box::new(group(vec![ch('b')])),
        };
        assert_eq!(
            parse(r"\frac{\frac{a}{b}}{c}"),
            Ok(group(vec![MathNode::Fraction {
                num: Box::new(group(vec![inner])),
                den: Box::new(group(vec![ch('c')])),
            }]))
        );
    }

    #[test]
    fn greek_seed_resolves_to_codepoints_not_markup() {
        assert_eq!(parse(r"\alpha"), Ok(group(vec![ch('α')])));
        assert_eq!(parse(r"\Gamma"), Ok(group(vec![ch('Γ')])));
        // The AST stays semantic: no italic mapping here (that is layout's,
        // per this module's header and Q#MS2).
        assert_eq!(parse(r"\alpha x"), Ok(group(vec![ch('α'), ch('x')])));
    }

    #[test]
    fn whitespace_is_insignificant() {
        assert_eq!(parse("x ^ 2"), parse("x^2"));
        assert_eq!(parse(r"\frac {a} {b}"), parse(r"\frac{a}{b}"));
    }

    #[test]
    fn subset_violations_are_errors_not_panics() {
        // Framing acceptance 1 and 9.
        assert_eq!(parse(""), Err(MathParseError::Empty));
        assert_eq!(parse("   "), Err(MathParseError::Empty));
        assert_eq!(parse("{a"), Err(MathParseError::UnbalancedBrace));
        assert_eq!(parse("a}"), Err(MathParseError::UnbalancedBrace));
        assert_eq!(
            parse(r"\sqrt{2}"),
            Err(MathParseError::UnknownCommand("sqrt".to_owned()))
        );
        assert!(matches!(
            parse(r"\frac{a}"),
            Err(MathParseError::MalformedCommand(_))
        ));
        assert!(matches!(
            parse(r"\frac a b"),
            Err(MathParseError::MalformedCommand(_))
        ));
        assert!(matches!(
            parse("^2"),
            Err(MathParseError::MalformedScript(_))
        ));
        assert!(matches!(
            parse("x^"),
            Err(MathParseError::MalformedScript(_))
        ));
        assert!(matches!(
            parse("x^2^3"),
            Err(MathParseError::MalformedScript(_))
        ));
    }

    #[test]
    fn an_interior_dollar_is_rejected_so_display_math_degrades() {
        // Framing acceptance 15: `$$x$$` reaches us as the interior `$x$`
        // (outer delimiters stripped), and must degrade to source rather
        // than half-render. The empty-span path covers `$$` alone.
        assert_eq!(parse("$x$"), Err(MathParseError::UnexpectedDollar));
        assert_eq!(parse(""), Err(MathParseError::Empty));
    }

    #[test]
    fn escaped_literals_survive_as_characters() {
        assert_eq!(parse(r"\{"), Ok(group(vec![ch('{')])));
        assert_eq!(parse(r"\$"), Ok(group(vec![ch('$')])));
    }
}
