//! LaTeX math-mode parser for the first inline-math slice.
//!
//! Framing: `docs/inline-math-slice-framing.md` (rev 3), Q#MS2. This parses
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
    ("epsilon", 'ε'),
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
    ("phi", 'φ'),
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
        while let Some(marker @ ('^' | '_')) = self.peek() {
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
            None => Err(MathParseError::MalformedScript("script with no operand")),
            Some('^' | '_') => Err(MathParseError::MalformedScript("script with no operand")),
            Some('}') => Err(MathParseError::MalformedScript("script with no operand")),
            Some(_) => self.parse_atom(),
        }
    }
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
