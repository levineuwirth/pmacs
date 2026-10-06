//! E8 review round 2 --- the label-offset reading of fix round 1
//! (`fe55d21`), through the parse every signature answer takes
//! (`SignatureHelp::from_lsp_value_in`), with no server: the offsets are
//! written here in the unit a server counts them in.
//!
//! Fix round 1 reads a parameter's `[start, end]` in the negotiated
//! encoding and then in UTF-16, and keeps the first reading under which
//! every parameter is a whole run of the label (`signature::coherent`).
//! A server that negotiated UTF-8 and counts UTF-16 (rust-analyzer) is
//! read as bytes first, and where a non-ASCII identifier's extra bytes
//! move every edge onto punctuation or a word boundary, that reading is
//! whole too and is kept. A server counting codepoints matches UTF-16
//! on a label with no character past the BMP and is read the same way;
//! with one, neither reading is usually whole and nothing is marked,
//! but sometimes the UTF-16 one is, cut short. The labels are
//! rust-analyzer's own text for the signatures in
//! `tests/e8_review2_real_server_probes.rs`, and a Python-style label
//! with an emoji default for the codepoint case.

use pmacs::lsp::PositionEncoding;
use pmacs::signature::SignatureHelp;
use serde_json::json;

/// A signature answer for `label` whose parameters are `params` (each a
/// substring found in order), their offsets counted by `count`.
fn answer(label: &str, params: &[&str], count: fn(&str) -> usize) -> serde_json::Value {
    let mut from = label.find('(').map_or(0, |i| i + 1);
    let parameters: Vec<_> = params
        .iter()
        .map(|p| {
            let at = from + label[from..].find(p).expect("parameter in label");
            from = at + p.len();
            json!({ "label": [count(&label[..at]), count(&label[..at + p.len()])] })
        })
        .collect();
    json!({
        "signatures": [{ "label": label, "parameters": parameters }],
        "activeSignature": 0,
        "activeParameter": 0,
    })
}

fn utf16(s: &str) -> usize {
    s.encode_utf16().count()
}

fn codepoints(s: &str) -> usize {
    s.chars().count()
}

/// Each parameter's marked text, `None` where no span was kept.
fn marks(label: &str, help: &SignatureHelp) -> Vec<Option<String>> {
    help.active()
        .expect("a signature")
        .parameters
        .iter()
        .map(|p| {
            p.span
                .map(|(s, e)| label[s as usize..e as usize].to_owned())
        })
        .collect()
}

/// FAILS at `57a980e`. UTF-16 offsets from a server that negotiated
/// UTF-8 (rust-analyzer, which the real-server row asks): each label's
/// byte reading is a set of whole runs, so it is kept, and in each a
/// parameter is marked on text that is not it: `名前` without its type,
/// `u8, a` across the comma, `&str,` for `a: u8`, and `长度计算`'s
/// second parameter on the first, exactly. Rust has taken non-ASCII
/// identifiers since 1.53.
#[test]
fn review2_utf16_offsets_from_a_utf8_server_are_read_as_utf16_where_both_readings_are_whole() {
    let mut wrong = Vec::new();
    for (label, params) in [
        ("fn f(名前: u8, a: u8) -> u8", &["名前: u8", "a: u8"][..]),
        (
            "fn g(名前: u8, größe: &str, a: u8) -> usize",
            &["名前: u8", "größe: &str", "a: u8"][..],
        ),
        (
            "fn 长度计算<'a, U>(a: i32, b: i32) -> i32",
            &["a: i32", "b: i32"][..],
        ),
        (
            "fn f(a: u8, 名前: u8, b: &T)",
            &["a: u8", "名前: u8", "b: &T"][..],
        ),
    ] {
        let help =
            SignatureHelp::from_lsp_value_in(&answer(label, params, utf16), PositionEncoding::Utf8);
        let got = marks(label, &help);
        let want: Vec<Option<String>> = params.iter().map(|p| Some((*p).to_owned())).collect();
        if got != want {
            wrong.push(format!(
                "{label:?}: marked {got:?}, the parameters are {want:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// PASSES at `57a980e`: the "neither" branch exists. Codepoint offsets
/// into a label with an emoji before an edge are whole under neither
/// reading, and no parameter is marked rather than a wrong one; the
/// popup does not fall back to a string search (the server sent none).
#[test]
fn review2_codepoint_offsets_no_reading_makes_whole_mark_nothing() {
    let label = "(a: str = \"😀\", b: int = 1) -> None";
    let params = ["a: str = \"😀\"", "b: int = 1"];
    let help = SignatureHelp::from_lsp_value_in(
        &answer(label, &params, codepoints),
        PositionEncoding::Utf8,
    );
    assert_eq!(marks(label, &help), [None, None]);
}

/// FAILS at `57a980e`. Codepoint offsets where the UTF-16 reading is
/// whole but short: the second parameter's emoji default is two UTF-16
/// units and one codepoint, so read as UTF-16 its span stops a
/// character early, before the closing quote, and that run is whole.
/// A parameter is marked whole or not at all.
#[test]
fn review2_codepoint_offsets_mark_a_parameter_whole_or_not_at_all() {
    let label = "(a: str, b: str = \"😀\") -> None";
    let params = ["a: str", "b: str = \"😀\""];
    let help = SignatureHelp::from_lsp_value_in(
        &answer(label, &params, codepoints),
        PositionEncoding::Utf8,
    );
    let got = marks(label, &help);
    for (mark, want) in got.iter().zip(params) {
        assert!(
            mark.as_deref().is_none_or(|m| m == want),
            "a parameter marked in part: {got:?}, the parameters are {params:?}"
        );
    }
}

fn bytes(s: &str) -> usize {
    s.len()
}

/// E8 fix round 2. The same labels from a server that counts its
/// offsets in the bytes it negotiated (clangd does): the byte reading is
/// the parameters, delimited and balanced, and the UTF-16 reading of the
/// same numbers is not, so each is marked whole. The stronger predicate
/// and the rule that two differing readings mark nothing refuse none of
/// them; over review 2's generated labels, counted in bytes, they refuse
/// none either (the fix round's pass has the sweep).
#[test]
fn fix2_byte_offsets_from_a_utf8_server_mark_each_parameter_whole() {
    let mut wrong = Vec::new();
    for (label, params) in [
        ("fn f(名前: u8, a: u8) -> u8", &["名前: u8", "a: u8"][..]),
        (
            "fn g(名前: u8, größe: &str, a: u8) -> usize",
            &["名前: u8", "größe: &str", "a: u8"][..],
        ),
        (
            "fn 长度计算<'a, U>(a: i32, b: i32) -> i32",
            &["a: i32", "b: i32"][..],
        ),
        (
            "fn move_to(現在地: (i32, i32))",
            &["現在地: (i32, i32)"][..],
        ),
        (
            "(a: str, b: str = \"😀\") -> None",
            &["a: str", "b: str = \"😀\""][..],
        ),
    ] {
        let help =
            SignatureHelp::from_lsp_value_in(&answer(label, params, bytes), PositionEncoding::Utf8);
        let got = marks(label, &help);
        let want: Vec<Option<String>> = params.iter().map(|p| Some((*p).to_owned())).collect();
        if got != want {
            wrong.push(format!(
                "{label:?}: marked {got:?}, the parameters are {want:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
