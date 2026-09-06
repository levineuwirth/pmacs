//! Shared SIGINT-guard conformance vectors
//! (the archived gpu-probe-sigint framing §7c).
//!
//! # Why these live here rather than in each suite
//!
//! The contract is that the **shell** consumer (`scripts/gate`) and the
//! **Rust** consumer (R-d's `sigint_diagnosis`) agree on every case.
//! Two independently written copies of the list can drift while both
//! still report "45 cases" — the same-length-different-content
//! divergence this matrix exists to rule out. One generator, two
//! consumers.

/// What a consumer must do with a given `(status, stdout)` pair.
///
/// `ValidatedError` and `Boundary` **both exit 2**, so a test comparing
/// only exit codes cannot separate them — and a validator that accepted
/// *every* status 2 would pass. They are told apart by whether the
/// child's stderr is surfaced: a validated verdict speaks with the
/// helper's voice; a boundary failure must not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Safe,
    ValidatedIgnored,
    ValidatedError,
    Boundary,
}

pub const TOKEN_SAFE: &[u8] = b"pmacs-sigint-v1:safe";
pub const TOKEN_IGNORED: &[u8] = b"pmacs-sigint-v1:ignored";
pub const TOKEN_ERROR: &[u8] = b"pmacs-sigint-v1:error";

/// Emitted on stderr by the **branch-discriminating** cases — the
/// cross-product rows and X1 — so a consumer's output can be searched
/// for it: present ⇒ the child's stderr was surfaced.
///
/// **X3 and X4 deliberately carry their own payloads instead**, which
/// is what makes them distinct inputs rather than duplicates of
/// `1/E/empty` and `0/V/safe/bare`. An earlier revision gave every case
/// this same sentinel and so shipped 45 entries over 43 distinct
/// inputs; [`shared_cases`] now asserts uniqueness so that cannot
/// recur silently.
pub const SENTINEL: &str = "PMACS-CONFORMANCE-SENTINEL";

/// The canonical wording the helper uses for `ignored`. X3 emits it
/// **without** a valid token, so a consumer that surfaced untrusted
/// stderr would repeat it — the defect A6b forbids.
pub const CANONICAL_IGNORED: &str =
    "pmacs: SIGINT is ignored; run this command with SIGINT deliverable";

pub struct Case {
    pub name: String,
    pub status: i32,
    pub stdout: Vec<u8>,
    /// Exact stderr this stub emits. Most cases use [`SENTINEL`]; X3
    /// and X4 carry their own payloads, which is what makes them
    /// distinct inputs rather than duplicates of other rows.
    pub stderr: String,
    pub expect: Outcome,
}

/// A `/bin/sh` stub reproducing one case exactly: its stdout bytes, its
/// own stderr payload, and its status.
#[must_use]
pub fn stub_script(case: &Case) -> String {
    let octal = case.stdout.iter().fold(String::new(), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "\\{b:03o}");
        acc
    });
    format!(
        "#!/bin/sh\nprintf '{octal}'\necho '{}' >&2\nexit {}\n",
        case.stderr, case.status
    )
}

/// The shared set: ten classes × encodings × three statuses, plus
/// X1/X3/X4. Only the diagonal validates.
///
/// X2 — a spawn error with no status — is deliberately absent: the
/// shell boundary cannot represent it, because an `exec` failure there
/// becomes a status. Rust exercises it separately.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "the bulk is the generated vector list; splitting it would \
              separate a case from the outcome it encodes, which is the \
              one thing this file exists to keep together"
)]
pub fn shared_cases() -> Vec<Case> {
    let toks: [(&str, &[u8]); 3] = [
        ("safe", TOKEN_SAFE),
        ("ignored", TOKEN_IGNORED),
        ("error", TOKEN_ERROR),
    ];
    let mut out = Vec::new();
    for (idx, (name, correct)) in toks.iter().enumerate() {
        let status = i32::try_from(idx).expect("0..=2");
        let diagonal = match status {
            0 => Outcome::Safe,
            1 => Outcome::ValidatedIgnored,
            _ => Outcome::ValidatedError,
        };
        let mut lf = correct.to_vec();
        lf.push(b'\n');
        out.push(Case {
            name: format!("{status}/V/{name}/bare"),
            status,
            stdout: correct.to_vec(),
            stderr: SENTINEL.to_owned(),
            expect: diagonal,
        });
        out.push(Case {
            name: format!("{status}/V/{name}/lf"),
            status,
            stdout: lf,
            stderr: SENTINEL.to_owned(),
            expect: diagonal,
        });
        for (other, bytes) in &toks {
            if other == name {
                continue;
            }
            let mut olf = bytes.to_vec();
            olf.push(b'\n');
            out.push(Case {
                name: format!("{status}/M/{other}/bare"),
                status,
                stdout: bytes.to_vec(),
                stderr: SENTINEL.to_owned(),
                expect: Outcome::Boundary,
            });
            out.push(Case {
                name: format!("{status}/M/{other}/lf"),
                status,
                stdout: olf,
                stderr: SENTINEL.to_owned(),
                expect: Outcome::Boundary,
            });
        }
        let mut leading = vec![b'\n'];
        leading.extend_from_slice(correct);
        let mut extra = correct.to_vec();
        extra.extend_from_slice(b"\n\n");
        let mut spaces = b" ".to_vec();
        spaces.extend_from_slice(correct);
        spaces.push(b' ');
        let mut crlf = correct.to_vec();
        crlf.extend_from_slice(b"\r\n");
        let mut doubled = correct.to_vec();
        doubled.extend_from_slice(correct);
        let mut nul = correct.to_vec();
        nul.push(0);
        for (cls, bytes) in [
            ("E/empty", Vec::new()),
            ("U/unknown", b"pmacs-sigint-v2:safe".to_vec()),
            ("L/leading-lf", leading),
            ("X/extra-lf", extra),
            ("S/spaces", spaces),
            ("C/crlf", crlf),
            ("D/doubled", doubled),
            ("N/nul", nul),
        ] {
            out.push(Case {
                name: format!("{status}/{cls}"),
                status,
                stdout: bytes,
                stderr: SENTINEL.to_owned(),
                expect: Outcome::Boundary,
            });
        }
    }
    out.push(Case {
        name: "X1/status-126".to_owned(),
        status: 126,
        stdout: TOKEN_SAFE.to_vec(),
        stderr: SENTINEL.to_owned(),
        expect: Outcome::Boundary,
    });
    out.push(Case {
        name: "X3/ignored-text-no-token".to_owned(),
        status: 1,
        stdout: Vec::new(),
        // The canonical ignored wording WITHOUT a token: a consumer
        // that surfaced untrusted stderr would repeat it.
        stderr: CANONICAL_IGNORED.to_owned(),
        expect: Outcome::Boundary,
    });
    out.push(Case {
        name: "X4/stderr-noise".to_owned(),
        status: 0,
        stdout: TOKEN_SAFE.to_vec(),
        // Noise on stderr must not affect classification --- and this
        // payload is what distinguishes X4 from 0/V/safe/bare.
        stderr: "unrelated chatter on stderr".to_owned(),
        expect: Outcome::Safe,
    });
    // The set must be 45 DISTINCT inputs, not merely 45 entries. A
    // previous revision gave every case the same stderr, which silently
    // collapsed X3 into `1/E/empty` and X4 into `0/V/safe/bare` — 45
    // entries, 43 inputs, and two framing-specified cases quietly not
    // exercised. Asserted here rather than in each suite so no consumer
    // can forget it.
    let mut seen = std::collections::HashSet::new();
    for case in &out {
        assert!(
            seen.insert((case.status, case.stdout.clone(), case.stderr.clone())),
            "duplicate conformance input at {}: (status, stdout, stderr) already present",
            case.name
        );
    }
    assert_eq!(seen.len(), out.len(), "every case must be a distinct input");
    out
}
