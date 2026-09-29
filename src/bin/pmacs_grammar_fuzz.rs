// pmacs_grammar_fuzz.rs --- fuzz every bundled tree-sitter grammar (E7g).

//! `pmacs_grammar_fuzz` --- parse real and mutated source against every
//! grammar in [`pmacs::syntax::BUILTIN_LANGUAGES`] and report what aborts,
//! hangs, or allocates without bound.
//!
//! `#![forbid(unsafe_code)]` covers the Rust; the grammars are C, and one
//! of them (tree-sitter-haskell 0.23.1, E7g) corrupted the heap on the
//! most ordinary Haskell file there is. This binary is how a grammar earns
//! its place: `scripts/fuzz-grammars` builds it the way the grammars ship
//! (release optimization, GCC, the C instrumented with `AddressSanitizer`),
//! seeds it from real files, and runs it.
//!
//! ```text
//! pmacs_grammar_fuzz list
//! pmacs_grammar_fuzz collect --out DIR [--from [LABEL=]PATH]...
//!                            [--corpus GRAMMAR=PATH]... [--max-files N]
//! pmacs_grammar_fuzz run --corpus DIR --out DIR [--seconds N] [--jobs N]
//!                        [--grammar NAME]... [--edits N] [--seed N]
//!                        [--hang-ms N] [--rss-mb N] [--min-mutations N]
//!                        [--max-seconds N] [--long NAME]... [--long-seconds N]
//! pmacs_grammar_fuzz changed --since REF
//! pmacs_grammar_fuzz repro GRAMMAR FILE [--edits N --seed N] [--trace 1]
//! ```
//!
//! Each grammar is driven through a worker process (`worker GRAMMAR`,
//! internal) so an abort, a hang or a runaway allocation costs the worker
//! and not the run. The worker does per input what the editor does per
//! file: `run_parse` cold with its injection layers, the settle step
//! (`SyntaxRegistry::resolve_layer_queries`), the highlight capture walk
//! over every layer whole and over a viewport, a walk of every node; then
//! `--edits` incremental reparses of the same text under typing, deletes,
//! pastes and batched edits, each settled the same way (two, and no
//! retyped window, for an input past the 64 KB seed cap).
//!
//! Findings are crashes (the process died: a sanitizer report, a glibc
//! heap check, an assert, a panic), hangs (a parse that has not returned
//! after twelve times the hang limit, alone), slow parses (over the limit
//! but returning), and allocations (one input growing the worker's RSS by
//! more than `--rss-mb`). Each is confirmed alone in a fresh worker and,
//! unless it is slow, minimized.
//!
//! `run` exits 1 on a crash or a hang, 2 on a usage or setup error (a
//! grammar with no seeds is one: the rule is corpus-seeded), and 0
//! otherwise. That is the owner's ruling at E7g (D36): what aborts or never
//! returns is not shipped, what is slow or large is filed. E7h.3 made the
//! line hold where review 1's planted defects showed it did not: every
//! parse runs under the hang limit as its deadline, so a parse that never
//! returns is cancelled and filed as a hang whatever it allocates; an
//! allocation is confirmed under four times the memory and twelve times the
//! time, and one that still has not returned is a hang; and a crash that
//! does not come back alone is replayed after the inputs its worker ran
//! before it, and fails the run however it came back (a kill by signal 9,
//! the host reclaiming memory, excepted). A hang that does not come back
//! alone is a loaded worker's and is reported, not failed.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pmacs::syntax::{
    BUILTIN_LANGUAGES, LanguageEntry, ParseError, ParseRequest, ParseTreeBundle, SyntaxRegistry,
    byte_to_point, compute_highlight_spans_for, default_injection_aliases, run_parse,
};

/// Seeds longer than this are cut at a line boundary: the editor parses
/// whole files, but a fuzzer's time goes further on many short inputs.
const MAX_SEED_BYTES: usize = 64 * 1024;
/// A mutated input is cut back to this.
const MAX_INPUT_BYTES: usize = 256 * 1024;
/// A worker is replaced after this many inputs, so what a grammar holds on
/// to across inputs shows as growth per worker and not as one input's.
const INPUTS_PER_WORKER: u64 = 500;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        eprintln!("usage: pmacs_grammar_fuzz list|collect|run|changed|repro|worker ...");
        return ExitCode::from(2);
    };
    let result = match cmd.as_str() {
        "list" => {
            for entry in BUILTIN_LANGUAGES {
                println!("{}\t{}", entry.name, entry.extensions.join(","));
            }
            Ok(ExitCode::SUCCESS)
        }
        "collect" => collect(rest),
        "run" => run(rest),
        "changed" => changed(rest),
        "repro" => repro(rest),
        "worker" => worker(rest),
        other => Err(format!("unknown subcommand `{other}`")),
    };
    result.unwrap_or_else(|e| {
        eprintln!("pmacs_grammar_fuzz: {e}");
        ExitCode::from(2)
    })
}

// -------------------------------------------------------------------
// Arguments
// -------------------------------------------------------------------

/// `--flag value` pairs, repeated flags kept in order.
struct Flags(Vec<(String, String)>);

impl Flags {
    fn parse(args: &[String], positional: usize) -> Result<(Vec<String>, Self), String> {
        let mut pos = Vec::new();
        let mut flags = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
                flags.push((name.to_owned(), v.clone()));
            } else {
                pos.push(a.clone());
            }
        }
        if pos.len() != positional {
            return Err(format!("expected {positional} positional argument(s)"));
        }
        Ok((pos, Self(flags)))
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.all(name).last().copied()
    }

    fn num(&self, name: &str, default: u64) -> Result<u64, String> {
        self.one(name).map_or(Ok(default), |v| {
            v.parse()
                .map_err(|_| format!("--{name}: not a number: {v}"))
        })
    }

    fn path(&self, name: &str) -> Result<PathBuf, String> {
        self.one(name)
            .map(PathBuf::from)
            .ok_or_else(|| format!("--{name} is required"))
    }
}

fn entry(name: &str) -> Result<&'static LanguageEntry, String> {
    BUILTIN_LANGUAGES
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("no bundled grammar named `{name}`"))
}

// -------------------------------------------------------------------
// Randomness: SplitMix64, so a run is reproducible from its seed.
// -------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.next() % den < num
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3)
    })
}

// -------------------------------------------------------------------
// The worker: what the editor does with a file, once per input.
// -------------------------------------------------------------------

fn worker(args: &[String]) -> Result<ExitCode, String> {
    let (pos, flags) = Flags::parse(args, 1)?;
    let lang = entry(&pos[0])?;
    let mut session = Session::new(lang);
    // E7h.3: the parent's per-parse hang limit is the parse's deadline, so
    // a grammar whose parse never returns is cancelled through the same
    // progress callback the editor uses and reported as a hang, before
    // anything it allocates while cycling can pass for a large parse.
    session.deadline = match flags.num("deadline-ms", 0)? {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    };
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut out = std::io::stdout().lock();
    writeln!(out, "READY").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    let mut header = String::new();
    loop {
        header.clear();
        if input.read_line(&mut header).map_err(|e| e.to_string())? == 0 {
            return Ok(ExitCode::SUCCESS);
        }
        let mut f = header.split_whitespace().map(str::parse::<u64>);
        let (Some(Ok(len)), Some(Ok(edits)), Some(Ok(seed))) = (f.next(), f.next(), f.next())
        else {
            return Err(format!("bad frame header `{}`", header.trim_end()));
        };
        let mut bytes = vec![0; usize::try_from(len).map_err(|e| e.to_string())?];
        input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let exercised = session.exercise(text, edits as u32, seed, &mut || {
            // A heartbeat per parse: the parent's hang limit is per parse.
            let _ = writeln!(out, "P").and_then(|()| out.flush());
        });
        match exercised {
            Ok(parses) => {
                let micros = started.elapsed().as_micros();
                writeln!(out, "E {parses} {micros} {}", peak_rss_kb("self"))
                    .map_err(|e| e.to_string())?;
            }
            // The parse never returned inside its deadline and was
            // cancelled; the parent files it as a hang. The worker is
            // replaced, as after any finding.
            Err(e) => writeln!(out, "D {e}").map_err(|e| e.to_string())?,
        }
        out.flush().map_err(|e| e.to_string())?;
    }
}

/// One grammar's editor-side machinery, reused across inputs as the
/// editor reuses it across files.
struct Session {
    lang: &'static LanguageEntry,
    language: tree_sitter::Language,
    registry: SyntaxRegistry,
    aliases: Arc<std::collections::HashMap<String, String>>,
    /// `repro --trace 1`: each parse's text, prior tree and edits to
    /// stderr before it runs, so a hang's last line is its reproduction.
    trace: bool,
    /// Every parse's deadline (E7h.3): the parent's hang limit, passed as
    /// `worker --deadline-ms`. `None` for `repro` and the unit rows.
    deadline: Option<Duration>,
    /// `PMACS_FUZZ_SELFTEST` (E7h.3): a defect planted in the worker on
    /// inputs holding [`SELFTEST_TRIGGER`], so the harness's own
    /// classification is witnessed by tests that plant each class.
    selftest: Option<SelfTest>,
}

/// The text that fires a planted [`SelfTest`] defect.
const SELFTEST_TRIGGER: &str = "@FUZZSELFTEST@";

/// The defect classes a harness self-test plants (E7h.3), one per mode of
/// `PMACS_FUZZ_SELFTEST`: each is a class the harness must classify, and
/// all but `slow` must fail the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelfTest {
    /// The worker dies (an abort, as a failed assert or a sanitizer does).
    Crash,
    /// The input never returns and allocates nothing.
    Hang,
    /// The input never returns and grows its memory about 1.3 GB a second.
    Grow,
    /// The input returns, after `PMACS_FUZZ_SELFTEST_MS` (default 2500 ms).
    Slow,
    /// The input returns after `PMACS_FUZZ_SELFTEST_MS` for each trigger it
    /// holds: a large input runs past twelve times the limit where its
    /// minimal one does not, as the cmake grammar's parse does on
    /// whitespace (E7h).
    Sized,
    /// The worker dies on the third trigger it sees: only a sequence of
    /// inputs reproduces it, never one alone.
    Sequence,
}

impl SelfTest {
    fn from_env() -> Option<Self> {
        match std::env::var("PMACS_FUZZ_SELFTEST").ok()?.as_str() {
            "crash" => Some(Self::Crash),
            "hang" => Some(Self::Hang),
            "grow" => Some(Self::Grow),
            "slow" => Some(Self::Slow),
            "sized" => Some(Self::Sized),
            "sequence" => Some(Self::Sequence),
            _ => None,
        }
    }

    /// Fire on a triggering input, before its first parse.
    fn fire(self, text: &str) {
        static SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        match self {
            Self::Crash => std::process::abort(),
            Self::Hang => loop {
                std::thread::sleep(Duration::from_secs(1));
            },
            Self::Grow => {
                let mut held: Vec<Vec<u8>> = Vec::new();
                loop {
                    held.push(vec![1; 64 << 20]);
                    std::hint::black_box(&held);
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            Self::Slow | Self::Sized => {
                let ms: u64 = std::env::var("PMACS_FUZZ_SELFTEST_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2500);
                let times = if self == Self::Sized {
                    text.matches(SELFTEST_TRIGGER).count() as u64
                } else {
                    1
                };
                std::thread::sleep(Duration::from_millis(ms * times));
            }
            Self::Sequence => {
                if SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 >= 3 {
                    std::process::abort();
                }
            }
        }
    }
}

impl Session {
    fn new(lang: &'static LanguageEntry) -> Self {
        Self {
            lang,
            language: (lang.loader)(),
            registry: SyntaxRegistry::new(),
            aliases: Arc::new(default_injection_aliases()),
            trace: false,
            deadline: None,
            selftest: SelfTest::from_env(),
        }
    }

    fn parse(
        &self,
        text: &str,
        prior: Option<tree_sitter::Tree>,
        edits: Vec<tree_sitter::InputEdit>,
    ) -> Result<ParseTreeBundle, ParseError> {
        if self.trace {
            eprintln!(
                "parse: incremental {}, edits {edits:?}\ntext {text:?}",
                prior.is_some()
            );
        }
        let req = ParseRequest {
            source: Arc::from(text.as_bytes()),
            language: self.language.clone(),
            language_name: self.lang.name.to_owned(),
            prior_tree: prior,
            edits,
            injection_aliases: self.aliases.clone(),
            deadline: self.deadline,
        };
        // A parse past its deadline is the caller's to report (a hang);
        // any other parse without a tree is itself a finding.
        match run_parse(req) {
            Err(e @ ParseError::DeadlineExceeded { .. }) => Err(e),
            Err(e) => panic!("run_parse returned no tree: {e}"),
            Ok(bundle) => Ok(bundle),
        }
    }

    /// The settle step and its consumers: layer queries resolved, the
    /// highlight walk whole and over a viewport, every node visited.
    fn settle(&self, bundle: &ParseTreeBundle, rng: &mut Rng) {
        let resolved = self.registry.resolve_layer_queries(bundle);
        let source = resolved.source.as_ref();
        for layer in &resolved.layers {
            if let Some(query) = &layer.highlight_query {
                let facts = layer.local_facts.as_deref();
                let whole = compute_highlight_spans_for(query, &layer.tree, source, facts, None);
                let a = rng.below(source.len() + 1);
                let view = a..(a + 4096).min(source.len());
                let part =
                    compute_highlight_spans_for(query, &layer.tree, source, facts, Some(view));
                std::hint::black_box((whole.len(), part.len()));
            }
            std::hint::black_box(walk(&layer.tree));
        }
    }

    /// Cold parse and settle, then `edits` incremental reparses, calling
    /// `beat` after each. Returns the number of parses run, or the first
    /// parse that ran past its deadline.
    fn exercise(
        &self,
        text: String,
        edits: u32,
        seed: u64,
        beat: &mut dyn FnMut(),
    ) -> Result<u32, ParseError> {
        if let Some(planted) = self.selftest
            && text.contains(SELFTEST_TRIGGER)
        {
            planted.fire(&text);
        }
        let mut rng = Rng(seed);
        // An input past the seed cap is a depth operator's or a repeat's:
        // its cold parse is what reaches a scanner's bound, and tree-sitter's
        // error recovery can make each of its parses take seconds (a
        // 238 KB Lua stair, 0.8 s a parse in release, E7h.4), so it gets
        // two edits and no retyped window instead of 49 parses.
        let large = text.len() > MAX_SEED_BYTES;
        let edits = if large { edits.min(2) } else { edits };
        let bundle = self.parse(&text, None, Vec::new())?;
        self.settle(&bundle, &mut rng);
        beat();
        let mut parses = 1;
        let mut tree = bundle.root_tree().clone();
        let mut cur = text;
        let mut left = edits;
        // Retype a window one character at a time, as a person does.
        if left > 0 && !cur.is_empty() && !large && rng.chance(1, 4) {
            let (a, b) = span(&cur, &mut rng, 48);
            let window = cur[a..b].to_owned();
            let mut steps = vec![(a, b, String::new())];
            let mut at = a;
            for ch in window.chars() {
                steps.push((at, at, ch.to_string()));
                at += ch.len_utf8();
            }
            for (s, e, ins) in steps {
                let (next, edit) = splice(&cur, s, e, &ins);
                cur = next;
                let b = self.parse(&cur, Some(tree), vec![edit])?;
                self.settle(&b, &mut rng);
                beat();
                tree = b.root_tree().clone();
                parses += 1;
            }
            left -= 1;
        }
        for _ in 0..left {
            let batch = if rng.chance(1, 4) {
                2 + rng.below(3)
            } else {
                1
            };
            let mut ops = Vec::with_capacity(batch);
            for _ in 0..batch {
                let (next, edit) = random_edit(&cur, &mut rng);
                cur = next;
                ops.push(edit);
            }
            let b = self.parse(&cur, Some(tree), ops)?;
            self.settle(&b, &mut rng);
            beat();
            tree = b.root_tree().clone();
            parses += 1;
        }
        Ok(parses)
    }
}

fn walk(tree: &tree_sitter::Tree) -> usize {
    let mut c = tree.walk();
    let mut n = 0;
    loop {
        let node = c.node();
        std::hint::black_box((node.kind_id(), node.byte_range(), node.is_error()));
        std::hint::black_box((
            node.is_missing(),
            node.start_position(),
            node.end_position(),
        ));
        n += 1;
        if c.goto_first_child() {
            continue;
        }
        loop {
            if c.goto_next_sibling() {
                break;
            }
            if !c.goto_parent() {
                return n;
            }
        }
    }
}

/// `VmHWM` in kB for `/proc/<who>/status`, or 0 where there is none.
fn peak_rss_kb(who: &str) -> u64 {
    proc_status_kb(who, "VmHWM:")
}

fn proc_status_kb(who: &str, key: &str) -> u64 {
    std::fs::read_to_string(format!("/proc/{who}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix(key))
                .and_then(|v| v.split_whitespace().next()?.parse().ok())
        })
        .unwrap_or(0)
}

// -------------------------------------------------------------------
// Edits and mutations, on char boundaries: the editor's text is UTF-8.
// -------------------------------------------------------------------

/// Fragments that open or close constructs across the grammars; the
/// mutator also draws tokens from each grammar's own seeds.
const FRAGMENTS: &[&str] = &[
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    "<",
    ">",
    "\"",
    "'",
    "`",
    ";",
    ":",
    ",",
    ".",
    "=",
    "+",
    "-",
    "*",
    "/",
    "\\",
    "|",
    "&",
    "!",
    "?",
    "#",
    "$",
    "%",
    "@",
    "^",
    "~",
    "\n",
    "\t",
    " ",
    "\r",
    "\r\n",
    "\0",
    "\u{e9}",
    "\u{4e2d}",
    "\u{1f600}",
    "\u{feff}",
    "\u{2028}",
    "\u{200d}",
    "\u{301}",
    "{-",
    "-}",
    "{-#",
    "#-}",
    "/*",
    "*/",
    "<!--",
    "-->",
    "```",
    "~~~",
    "$$",
    "\\begin{",
    "\\end{",
    "${",
    "#{",
    "{%",
    "%}",
    "'''",
    "\"\"\"",
    "#!",
    "<<EOF\n",
    "\nEOF\n",
    "r#\"",
    "\"#",
    "--[[",
    "]]",
    "<script>",
    "</script>",
    "<style>",
    "---\n",
    "...\n",
];

fn boundary(text: &str, rng: &mut Rng) -> usize {
    let mut i = rng.below(text.len() + 1);
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn span(text: &str, rng: &mut Rng, max: usize) -> (usize, usize) {
    let a = boundary(text, rng);
    let mut b = (a + 1 + rng.below(max)).min(text.len());
    while !text.is_char_boundary(b) {
        b += 1;
    }
    (a, b)
}

fn splice(text: &str, start: usize, old_end: usize, ins: &str) -> (String, tree_sitter::InputEdit) {
    let mut next = String::with_capacity(text.len() + ins.len());
    next.push_str(&text[..start]);
    next.push_str(ins);
    next.push_str(&text[old_end..]);
    let new_end = start + ins.len();
    let edit = tree_sitter::InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: byte_to_point(text.as_bytes(), start),
        old_end_position: byte_to_point(text.as_bytes(), old_end),
        new_end_position: byte_to_point(next.as_bytes(), new_end),
    };
    (next, edit)
}

fn one_char(text: &str, rng: &mut Rng) -> String {
    if !text.is_empty() && rng.chance(1, 2) {
        let at = boundary(text, rng);
        if let Some(c) = text[at..].chars().next() {
            return c.to_string();
        }
    }
    FRAGMENTS[rng.below(FRAGMENTS.len())].to_owned()
}

/// One editor-shaped edit: a keystroke, a backspace run, a paste, or a
/// replacement.
fn random_edit(text: &str, rng: &mut Rng) -> (String, tree_sitter::InputEdit) {
    match rng.below(5) {
        0 | 1 => {
            let p = boundary(text, rng);
            splice(text, p, p, &one_char(text, rng))
        }
        2 if !text.is_empty() => {
            let (a, b) = span(text, rng, 8);
            splice(text, a, b, "")
        }
        3 if !text.is_empty() => {
            let (a, b) = span(text, rng, 256);
            let chunk = text[a..b].to_owned();
            let p = boundary(text, rng);
            splice(text, p, p, &chunk)
        }
        _ => {
            let (a, b) = if text.is_empty() {
                (0, 0)
            } else {
                span(text, rng, 16)
            };
            splice(text, a, b, FRAGMENTS[rng.below(FRAGMENTS.len())])
        }
    }
}

/// Tokens drawn from a grammar's seeds: identifier runs and short
/// punctuation runs, as a dictionary for the mutator.
fn harvest(seeds: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut keep = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() && seen.insert(cur.clone()) {
            out.push(cur.clone());
        }
        cur.clear();
    };
    for s in seeds {
        let mut cur = String::new();
        let mut class = 0;
        for c in s.chars() {
            let k = if c.is_whitespace() {
                0
            } else if c.is_alphanumeric() || c == '_' {
                1
            } else {
                2
            };
            if k != class || (k == 2 && cur.len() >= 3) || cur.len() >= 24 {
                keep(&mut cur, &mut out);
                class = k;
            }
            if k != 0 {
                cur.push(c);
            }
        }
        keep(&mut cur, &mut out);
        if out.len() >= 8192 {
            break;
        }
    }
    if out.is_empty() {
        out.push("x".to_owned());
    }
    out
}

struct Mutator<'a> {
    seeds: &'a [String],
    dict: &'a [String],
}

impl Mutator<'_> {
    fn mutate(&self, rng: &mut Rng) -> String {
        let mut s = self.seeds[rng.below(self.seeds.len())].clone();
        for _ in 0..=rng.below(6) {
            s = self.once(&s, rng);
        }
        if s.len() > MAX_INPUT_BYTES {
            let mut cut = MAX_INPUT_BYTES;
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            s.truncate(cut);
        }
        s
    }

    fn other(&self, rng: &mut Rng) -> &str {
        &self.seeds[rng.below(self.seeds.len())]
    }

    fn once(&self, text: &str, rng: &mut Rng) -> String {
        let insert = |text: &str, rng: &mut Rng, ins: &str| {
            let at = boundary(text, rng);
            splice(text, at, at, ins).0
        };
        match rng.below(14) {
            0 => {
                let frag = FRAGMENTS[rng.below(FRAGMENTS.len())];
                insert(text, rng, frag)
            }
            1 => {
                let token = &self.dict[rng.below(self.dict.len())];
                insert(text, rng, token)
            }
            2 if !text.is_empty() => {
                let (lo, hi) = span(text, rng, 64);
                splice(text, lo, hi, "").0
            }
            3 if !text.is_empty() => {
                let (lo, hi) = span(text, rng, 256);
                insert(text, rng, &text[lo..hi])
            }
            4 => {
                let other = self.other(rng);
                let (lo, hi) = if other.is_empty() {
                    (0, 0)
                } else {
                    span(other, rng, 512)
                };
                insert(text, rng, &other[lo..hi])
            }
            5 => {
                let other = self.other(rng);
                let head = boundary(text, rng);
                let tail = boundary(other, rng);
                format!("{}{}", &text[..head], &other[tail..])
            }
            6 => line_op(text, rng),
            7 => indent_op(text, rng),
            12 => depth_prefix(text, rng),
            13 => depth_stair(text, rng),
            8 => {
                let cut = boundary(text, rng);
                text[..cut].to_owned()
            }
            9 if !text.is_empty() => {
                let (lo, hi) = span(text, rng, 8);
                let most = if rng.chance(1, 4) { 4000 } else { 60 };
                let times = 2 + rng.below(most);
                insert(text, rng, &text[lo..hi].repeat(times))
            }
            10 if !text.is_empty() => {
                let (lo, hi) = span(text, rng, 32);
                let (from, to) = span(text, rng, 32);
                splice(text, lo, hi, &text[from..to]).0
            }
            _ if !text.is_empty() => {
                let (lo, hi) = span(text, rng, 1);
                let ch = char::from(b' ' + (rng.below(95) as u8));
                splice(text, lo, hi, &ch.to_string()).0
            }
            _ => {
                let frag = FRAGMENTS[rng.below(FRAGMENTS.len())];
                insert(text, rng, frag)
            }
        }
    }
}

/// How deep a depth operator nests (E7h.4): past the depths at which a
/// scanner's state outgrows tree-sitter's 1024-byte serialization buffer
/// --- 255 open blocks at four bytes (markdown), 254 levels (YAML), 511
/// indents with a string open (Python), all found by E7g review 1 --- and
/// shallow enough that a capture walk quadratic in depth (Lua's, #292)
/// stays in milliseconds.
fn depth(rng: &mut Rng) -> usize {
    250 + rng.below(851)
}

/// Repeat the start of a line (its first one to three characters: `> `,
/// `- `, `(`, `{`, `#`, `if `...) hundreds of times at the line's start.
fn depth_prefix(s: &str, rng: &mut Rng) -> String {
    let mut lines: Vec<String> = s.split_inclusive('\n').map(str::to_owned).collect();
    if lines.is_empty() {
        lines.push("(\n".to_owned());
    }
    let i = rng.below(lines.len());
    let line = lines[i].clone();
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let body = &line[indent..];
    let take = 1 + rng.below(3);
    let unit: String = if body.trim().is_empty() {
        FRAGMENTS[rng.below(FRAGMENTS.len())].to_owned()
    } else {
        body.chars().take(take).collect()
    };
    let room = MAX_INPUT_BYTES.saturating_sub(s.len()) / unit.len().max(1);
    let k = depth(rng).min(room);
    lines[i] = format!("{}{}{}", &line[..indent], unit.repeat(k), body);
    lines.concat()
}

/// Copy a line hundreds of times, each copy indented one step deeper (a
/// step of one, two or four spaces), then another line of the input at the
/// deepest level: nested blocks, keys or list items with whatever the
/// other line holds (a string, say) at the bottom.
fn depth_stair(s: &str, rng: &mut Rng) -> String {
    let lines: Vec<&str> = s.split_inclusive('\n').collect();
    if lines.is_empty() {
        return s.to_owned();
    }
    // Half the time the stair is a line that opens a block (ends in `:`,
    // `{`, `(` or `[`) and the line at the bottom holds a string: Python
    // overruns only with a string open 511 indents deep, which a uniform
    // pick of two lines out of a whole file rarely assembles.
    let pick = |rng: &mut Rng, want: &dyn Fn(&str) -> bool| {
        let fit: Vec<usize> = (0..lines.len())
            .filter(|&j| want(lines[j].trim()))
            .collect();
        if fit.is_empty() || rng.chance(1, 2) {
            rng.below(lines.len())
        } else {
            fit[rng.below(fit.len())]
        }
    };
    let i = pick(rng, &|l| l.ends_with([':', '{', '(', '[']));
    let stair = lines[i].trim();
    let last = lines[pick(rng, &|l| l.contains(['"', '\'', '`']))].trim();
    // One space a level most often: the stair's size is quadratic in its
    // step, and only a step of one fits 511 levels under the input cap.
    let step = [1, 1, 2, 4][rng.below(4)];
    let budget = MAX_INPUT_BYTES.saturating_sub(s.len());
    let mut k = depth(rng);
    let size =
        |k: usize| k * (k - 1) / 2 * step + k * (stair.len() + 1) + k * step + last.len() + 1;
    while k > 1 && size(k) > budget {
        k -= k / 8 + 1;
    }
    let mut out = String::with_capacity(s.len() + budget.min(k * k * step));
    out.extend(lines[..i].iter().copied());
    for level in 0..k {
        out.push_str(&" ".repeat(level * step));
        out.push_str(stair);
        out.push('\n');
    }
    out.push_str(&" ".repeat(k * step));
    out.push_str(last);
    out.push('\n');
    out.extend(lines[i + 1..].iter().copied());
    out
}

fn line_op(s: &str, rng: &mut Rng) -> String {
    let mut lines: Vec<&str> = s.split_inclusive('\n').collect();
    if lines.is_empty() {
        return s.to_owned();
    }
    let i = rng.below(lines.len());
    match rng.below(3) {
        0 => {
            lines.remove(i);
        }
        1 => lines.insert(i, lines[i]),
        _ => {
            let j = rng.below(lines.len());
            lines.swap(i, j);
        }
    }
    lines.concat()
}

fn indent_op(s: &str, rng: &mut Rng) -> String {
    let mut lines: Vec<String> = s.split_inclusive('\n').map(str::to_owned).collect();
    if lines.is_empty() {
        return s.to_owned();
    }
    let i = rng.below(lines.len());
    let line = &mut lines[i];
    let body = line.trim_start_matches([' ', '\t']).to_owned();
    let pad = match rng.below(4) {
        0 => String::new(),
        1 => "\t".repeat(1 + rng.below(4)),
        _ => " ".repeat(rng.below(17)),
    };
    *line = pad + &body;
    lines.concat()
}

// -------------------------------------------------------------------
// The parent side of a worker.
// -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Crash,
    /// A parse that did not return within the long limit alone.
    Hang,
    /// A parse over the hang limit that did return alone.
    Slow,
    Alloc,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Hang => "hang",
            Self::Slow => "slow",
            Self::Alloc => "alloc",
        }
    }
}

enum Outcome {
    Done {
        parses: u64,
        micros: u64,
    },
    Failed {
        kind: Kind,
        signature: String,
        detail: String,
    },
}

struct Limits {
    hang: Duration,
    rss_kb: u64,
}

/// One input as a worker ran it, kept so a finding that needs what came
/// before it can be replayed (E7h.3).
#[derive(Clone)]
struct Input {
    text: String,
    edits: u32,
    seed: u64,
}

/// A worker's history is capped in bytes; the oldest inputs go first.
const HISTORY_BYTES: usize = 64 << 20;

struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: PathBuf,
    baseline_kb: u64,
    inputs: u64,
    /// The inputs this worker has run, in order, the current one last.
    history: std::collections::VecDeque<Input>,
    history_bytes: usize,
}

impl Worker {
    /// Spawn a worker whose every parse has `deadline` (the hang limit
    /// it runs under): a parse that never returns is cancelled at it and
    /// reported as a hang (E7h.3).
    fn spawn(grammar: &str, stderr: PathBuf, deadline: Duration) -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let err = std::fs::File::create(&stderr).map_err(|e| e.to_string())?;
        let deadline_ms = deadline.as_millis().max(1).to_string();
        let mut child = Command::new(exe)
            .args(["worker", grammar, "--deadline-ms", &deadline_ms])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(err)
            .spawn()
            .map_err(|e| format!("spawn worker: {e}"))?;
        let stdin = child.stdin.take().ok_or("no worker stdin")?;
        let stdout = child.stdout.take().ok_or("no worker stdout")?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let ready = lines.recv_timeout(Duration::from_mins(2));
        if ready.as_deref() != Ok("READY") {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("worker for {grammar} did not start: {ready:?}"));
        }
        let baseline_kb = proc_status_kb(&child.id().to_string(), "VmRSS:");
        Ok(Self {
            child,
            stdin,
            lines,
            stderr,
            baseline_kb,
            inputs: 0,
            history: std::collections::VecDeque::new(),
            history_bytes: 0,
        })
    }

    /// The inputs this worker ran before the current (last) one.
    fn history_before_last(&self) -> Vec<Input> {
        let n = self.history.len().saturating_sub(1);
        self.history.iter().take(n).cloned().collect()
    }

    fn run(&mut self, text: &str, edits: u32, seed: u64, limits: &Limits) -> Outcome {
        self.history.push_back(Input {
            text: text.to_owned(),
            edits,
            seed,
        });
        self.history_bytes += text.len();
        while self.history_bytes > HISTORY_BYTES && self.history.len() > 1 {
            if let Some(old) = self.history.pop_front() {
                self.history_bytes -= old.text.len();
            }
        }
        let header = format!("{} {edits} {seed}\n", text.len());
        let sent = self
            .stdin
            .write_all(header.as_bytes())
            .and_then(|()| self.stdin.write_all(text.as_bytes()))
            .and_then(|()| self.stdin.flush());
        let mut last = Instant::now();
        let pid = self.child.id().to_string();
        // Excessive allocation is one input's growth, measured from where
        // the worker stood when the input went in; what accumulates across
        // inputs is reported per worker instead (`growth_kb`).
        let start_kb = proc_status_kb(&pid, "VmRSS:");
        self.inputs += 1;
        loop {
            if sent.is_err() {
                return self.died();
            }
            match self.lines.recv_timeout(Duration::from_millis(50)) {
                Ok(line) if line == "P" => last = Instant::now(),
                // E7h.3: the worker's parse ran past its deadline (the hang
                // limit) and was cancelled; the same finding as a hang
                // caught by the missing heartbeat.
                Ok(line) if line.starts_with("D ") => {
                    self.kill();
                    return Outcome::Failed {
                        kind: Kind::Hang,
                        signature: format!("one parse over {} ms", limits.hang.as_millis()),
                        detail: line[2..].to_owned(),
                    };
                }
                Ok(line) => {
                    let mut f = line.split_whitespace().skip(1).map(str::parse::<u64>);
                    let parses = f.next().and_then(Result::ok).unwrap_or(0);
                    let micros = f.next().and_then(Result::ok).unwrap_or(0);
                    return Outcome::Done { parses, micros };
                }
                Err(RecvTimeoutError::Disconnected) => return self.died(),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if last.elapsed() > limits.hang {
                self.kill();
                return Outcome::Failed {
                    kind: Kind::Hang,
                    signature: format!("one parse over {} ms", limits.hang.as_millis()),
                    detail: String::new(),
                };
            }
            let rss = proc_status_kb(&pid, "VmRSS:");
            if rss > start_kb + limits.rss_kb {
                self.kill();
                return Outcome::Failed {
                    kind: Kind::Alloc,
                    signature: format!("one input grew RSS by over {} MB", limits.rss_kb / 1024),
                    detail: format!(
                        "RSS {rss} kB, {start_kb} kB when the input went in, {} kB at spawn",
                        self.baseline_kb
                    ),
                };
            }
        }
    }

    /// RSS gained since spawn, in kB.
    fn growth_kb(&self) -> u64 {
        proc_status_kb(&self.child.id().to_string(), "VmRSS:").saturating_sub(self.baseline_kb)
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn died(&mut self) -> Outcome {
        let status = self.child.wait().ok();
        let log = std::fs::read(&self.stderr).unwrap_or_default();
        let log = String::from_utf8_lossy(&log[log.len().saturating_sub(64 * 1024)..]).into_owned();
        let how = status.map_or_else(
            || "unknown exit".to_owned(),
            |s| match (s.signal(), s.code()) {
                (Some(sig), _) => format!("signal {sig}"),
                (None, Some(code)) => format!("exit {code}"),
                (None, None) => "unknown exit".to_owned(),
            },
        );
        Outcome::Failed {
            kind: Kind::Crash,
            signature: crash_signature(&log, &how),
            detail: format!("{how}\n{log}"),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
    }
}

/// What glibc's allocator prints before it aborts on a corrupted heap.
const GLIBC: [&str; 7] = [
    "corrupted ",
    "free(): ",
    "malloc(): ",
    "double free",
    "munmap_chunk(): ",
    "realloc(): ",
    "malloc_consolidate(): ",
];

/// A short, stable name for a crash: the sanitizer's error and first
/// frame, glibc's complaint, the panic site, or the signal.
fn crash_signature(log: &str, how: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    if let Some(i) = lines
        .iter()
        .position(|l| l.contains("ERROR: AddressSanitizer: "))
    {
        let what = lines[i]
            .split("AddressSanitizer: ")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .unwrap_or("error");
        let frame = lines[i..]
            .iter()
            .find_map(|l| l.trim_start().strip_prefix("#0 "))
            .and_then(|f| f.split(" in ").nth(1))
            .and_then(|f| f.split_whitespace().next())
            .map_or("?", |f| f.split('.').next().unwrap_or(f));
        return format!("asan {what} in {frame}");
    }
    // UBSan (E7h.4): `<file>:<line>:<col>: runtime error: <what>`.
    if let Some(l) = lines.iter().find(|l| l.contains(": runtime error: ")) {
        let (at, what) = l.split_once(": runtime error: ").unwrap_or((l, ""));
        let at = at.rsplit('/').next().unwrap_or(at);
        // The kind of UB, without the operands, which vary per input:
        // `signed integer overflow: 1 + 2147483647 …` names its kind before
        // the colon; a message without one keeps its first three words.
        let what: String = match what.split_once(':') {
            Some((kind, _)) => kind.to_owned(),
            None => what
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" "),
        };
        return format!("ubsan {what} at {at}");
    }
    // An assert (tree-sitter's serialization bound, E7g): glibc prints
    // `<prog>: <path>:<line>: <function>: Assertion `<expr>' failed.`, so
    // two different asserts no longer share `signal 6` (E7h.3).
    if let Some(l) = lines.iter().find(|l| l.contains("Assertion `")) {
        let (head, expr) = l.split_once("Assertion `").unwrap_or((l, ""));
        let expr = expr.split('\'').next().unwrap_or(expr);
        let site = head
            .trim_end_matches(": ")
            .split(": ")
            .skip(1)
            .map(|part| part.rsplit('/').next().unwrap_or(part))
            .collect::<Vec<_>>()
            .join(" ");
        return format!("assert `{expr}` at {site}");
    }
    if let Some(l) = lines.iter().find(|l| GLIBC.iter().any(|g| l.contains(g))) {
        return format!("glibc: {}", l.trim());
    }
    if let Some(l) = lines.iter().find(|l| l.contains("panicked at")) {
        let site = l
            .split("panicked at ")
            .nth(1)
            .unwrap_or(l)
            .trim_end_matches(':');
        return format!("panic at {site}");
    }
    how.to_owned()
}

// -------------------------------------------------------------------
// `run`
// -------------------------------------------------------------------

struct Config {
    corpus: PathBuf,
    out: PathBuf,
    seconds: u64,
    edits: u32,
    seed: u64,
    limits: Limits,
    /// Mutate each grammar at least this many times, past `seconds` if
    /// need be (E7h.4): a budget in seconds alone gave CI's smoke 12
    /// mutated inputs for lean4 and 15 for zig.
    min_mutations: u64,
    /// The wall-clock cap on one grammar's mutations whatever the minimum.
    max_seconds: u64,
    /// Grammars to mutate for `long_seconds` instead (E7h.4): what a pull
    /// request changed, from `changed --since`.
    long: Vec<String>,
    long_seconds: u64,
}

#[derive(Default)]
struct Stats {
    seeds: usize,
    inputs: u64,
    parses: u64,
    bytes: u64,
    slowest_micros: u64,
    slowest_len: usize,
    elapsed: Duration,
    growth_kb: u64,
}

struct Finding {
    kind: Kind,
    signature: String,
    input: String,
    edits: u32,
    seed: u64,
    detail: String,
    /// What the worker ran before this input, oldest first (E7h.3), for a
    /// crash that needs it.
    history: Vec<Input>,
}

/// How a finding came back when triage ran it again (E7h.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Repro {
    /// Alone, in a fresh worker.
    Alone,
    /// Only after the last `n` inputs its worker had run before it.
    InSequence(usize),
    /// Neither way.
    No,
}

struct Report {
    grammar: &'static str,
    stats: Stats,
    findings: Vec<Triaged>,
    error: Option<String>,
}

struct Triaged {
    finding: Finding,
    repro: Repro,
    minimal: String,
    minimal_edits: u32,
    file: PathBuf,
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let (_, flags) = Flags::parse(args, 0)?;
    let cfg = Arc::new(Config {
        corpus: flags.path("corpus")?,
        out: flags.path("out")?,
        seconds: flags.num("seconds", 60)?,
        edits: u32::try_from(flags.num("edits", 8)?).map_err(|e| e.to_string())?,
        seed: flags.num("seed", 1)?,
        limits: Limits {
            hang: Duration::from_millis(flags.num("hang-ms", 10_000)?),
            rss_kb: flags.num("rss-mb", 1024)? * 1024,
        },
        min_mutations: flags.num("min-mutations", 0)?,
        max_seconds: flags.num("max-seconds", 0)?,
        long: flags.all("long").into_iter().map(str::to_owned).collect(),
        long_seconds: flags.num("long-seconds", 600)?,
    });
    for l in &cfg.long {
        entry(l)?;
    }
    let jobs = usize::try_from(flags.num("jobs", 4)?.max(1)).map_err(|e| e.to_string())?;
    let wanted = flags.all("grammar");
    for w in &wanted {
        entry(w)?;
    }
    let queue: Vec<&'static LanguageEntry> = BUILTIN_LANGUAGES
        .iter()
        .filter(|e| wanted.is_empty() || wanted.contains(&e.name))
        .collect();
    std::fs::create_dir_all(&cfg.out).map_err(|e| e.to_string())?;
    let queue = Arc::new(Mutex::new(queue));
    let reports = Arc::new(Mutex::new(Vec::new()));
    let threads: Vec<_> = (0..jobs)
        .map(|_| {
            let (queue, reports, cfg) = (queue.clone(), reports.clone(), cfg.clone());
            std::thread::spawn(move || {
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop());
                    let Some(lang) = next else { break };
                    let report = fuzz_grammar(lang, &cfg);
                    eprintln!("{}", summary_line(&report));
                    if let Ok(mut r) = reports.lock() {
                        r.push(report);
                    }
                }
            })
        })
        .collect();
    for t in threads {
        t.join().map_err(|_| "a fuzz thread panicked")?;
    }
    let mut reports = std::mem::take(&mut *reports.lock().map_err(|_| "poisoned")?);
    reports.sort_by_key(|r| r.grammar);
    write_report(&cfg, &reports)?;
    let failed = fails(&reports);
    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Whether the run fails: a grammar could not be fuzzed; a hang came back
/// (a parse that never returned, including one that grew past the raised
/// memory cap without returning: E7h.3); or a crash was seen at all. A
/// crash that came back alone or after its worker's own history fails the
/// run, and one that came back neither way fails it too (the workers are
/// single-threaded and every input they ran is replayed, so what does not
/// come back is not load), except a kill by signal 9, which is the host
/// reclaiming memory. Slow and large parses are reported, D36.
fn fails(reports: &[Report]) -> bool {
    reports.iter().any(|r| {
        r.error.is_some()
            || r.findings.iter().any(|t| match t.finding.kind {
                Kind::Crash => t.repro != Repro::No || t.finding.signature != "signal 9",
                Kind::Hang => t.repro != Repro::No,
                Kind::Slow | Kind::Alloc => false,
            })
    })
}

fn summary_line(r: &Report) -> String {
    format!(
        "{}: {} seeds, {} inputs, {} parses, {} findings{}",
        r.grammar,
        r.stats.seeds,
        r.stats.inputs,
        r.stats.parses,
        r.findings.len(),
        r.error
            .as_deref()
            .map_or_else(String::new, |e| format!(", ERROR {e}"))
    )
}

fn load_seeds(dir: &Path) -> Vec<String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .filter_map(|p| std::fs::read(p).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect()
}

/// A live worker for the next input: spawned if there is none, replaced
/// after `INPUTS_PER_WORKER` inputs with its RSS growth recorded.
fn keep_worker(
    worker: &mut Option<Worker>,
    spawned: &mut usize,
    grammar: &str,
    dir: &Path,
    stats: &mut Stats,
    limits: &Limits,
) -> Result<(), String> {
    if let Some(w) = worker.as_ref()
        && w.inputs >= INPUTS_PER_WORKER
    {
        stats.growth_kb = stats.growth_kb.max(w.growth_kb());
        *worker = None;
    }
    if worker.is_none() {
        *spawned += 1;
        *worker = Some(Worker::spawn(
            grammar,
            dir.join(format!("worker-{spawned}.stderr")),
            limits.hang,
        )?);
    }
    Ok(())
}

fn fuzz_grammar(lang: &'static LanguageEntry, cfg: &Config) -> Report {
    let mut report = Report {
        grammar: lang.name,
        stats: Stats::default(),
        findings: Vec::new(),
        error: None,
    };
    let started = Instant::now();
    let seeds = load_seeds(&cfg.corpus.join(lang.name));
    report.stats.seeds = seeds.len();
    if seeds.is_empty() {
        report.error = Some("no seeds".to_owned());
        return report;
    }
    let dir = cfg.out.join(lang.name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        report.error = Some(e.to_string());
        return report;
    }
    let dict = harvest(&seeds);
    let mutator = Mutator {
        seeds: &seeds,
        dict: &dict,
    };
    let mut rng = Rng(cfg.seed ^ fnv(lang.name.as_bytes()));
    let mut findings: Vec<Finding> = Vec::new();
    let mut spawned = 0;
    let mut worker: Option<Worker> = None;
    // Every seed first, then mutations for `seconds` (a long grammar's
    // `long_seconds`) and at least `min_mutations` of them, under the cap.
    let seconds = if cfg.long.iter().any(|l| l == lang.name) {
        cfg.long_seconds
    } else {
        cfg.seconds
    };
    let cap = if cfg.max_seconds > 0 {
        cfg.max_seconds.max(seconds)
    } else {
        u64::MAX
    };
    let mut window: Option<(Instant, Instant)> = None;
    let mut mutated: u64 = 0;
    let mut next_seed = 0;
    loop {
        let input = if next_seed < seeds.len() {
            next_seed += 1;
            seeds[next_seed - 1].clone()
        } else {
            let now = Instant::now();
            let (end, hard) = *window.get_or_insert_with(|| {
                (
                    now + Duration::from_secs(seconds),
                    now.checked_add(Duration::from_secs(cap))
                        .unwrap_or(now + Duration::from_hours(24)),
                )
            });
            if now >= hard || (now >= end && mutated >= cfg.min_mutations) {
                break;
            }
            mutated += 1;
            mutator.mutate(&mut rng)
        };
        let seed = rng.next();
        if let Err(e) = keep_worker(
            &mut worker,
            &mut spawned,
            lang.name,
            &dir,
            &mut report.stats,
            &cfg.limits,
        ) {
            report.error = Some(e);
            break;
        }
        let Some(w) = worker.as_mut() else { break };
        report.stats.inputs += 1;
        report.stats.bytes += input.len() as u64;
        let outcome = w.run(&input, cfg.edits, seed, &cfg.limits);
        if let Outcome::Failed { .. } = outcome {
            let history = w.history_before_last();
            worker = None;
            record_failure(&mut findings, outcome, input, cfg.edits, seed, history);
        } else if let Outcome::Done { parses, micros } = outcome {
            report.stats.parses += parses;
            if micros > report.stats.slowest_micros {
                report.stats.slowest_micros = micros;
                report.stats.slowest_len = input.len();
                // Kept with its seed, so the slowest column can be
                // replayed (`repro GRAMMAR slowest.input --seed N`).
                let _ = std::fs::write(dir.join("slowest.input"), &input);
                let _ = std::fs::write(dir.join("slowest.seed"), format!("{seed}\n"));
            }
        }
    }
    let last = worker.as_ref().map_or(0, Worker::growth_kb);
    report.stats.growth_kb = report.stats.growth_kb.max(last);
    drop(worker);
    report.stats.elapsed = started.elapsed();
    report.findings = findings
        .into_iter()
        .enumerate()
        .map(|(i, f)| triage(lang.name, f, &dir, i, &cfg.limits))
        .collect();
    report
}

/// Keep a failed input as a finding, at most three of one signature.
fn record_failure(
    findings: &mut Vec<Finding>,
    outcome: Outcome,
    input: String,
    edits: u32,
    seed: u64,
    history: Vec<Input>,
) {
    let Outcome::Failed {
        kind,
        signature,
        detail,
    } = outcome
    else {
        return;
    };
    if findings.iter().filter(|f| f.signature == signature).count() < 3 {
        findings.push(Finding {
            kind,
            signature,
            input,
            edits,
            seed,
            detail,
            history,
        });
    }
}

// -------------------------------------------------------------------
// Triage: confirm alone, then minimize.
// -------------------------------------------------------------------

fn outcome_alone(
    grammar: &str,
    text: &str,
    edits: u32,
    seed: u64,
    dir: &Path,
    limits: &Limits,
) -> Option<(Kind, String)> {
    let mut w = Worker::spawn(grammar, dir.join("triage.stderr"), limits.hang).ok()?;
    match w.run(text, edits, seed, limits) {
        Outcome::Done { .. } => None,
        Outcome::Failed {
            kind, signature, ..
        } => Some((kind, signature)),
    }
}

/// How long `text` takes alone in a fresh worker under `limits`, or
/// `None` if it did not return.
fn time_alone(
    grammar: &str,
    text: &str,
    edits: u32,
    seed: u64,
    dir: &Path,
    limits: &Limits,
) -> Option<u64> {
    let mut w = Worker::spawn(grammar, dir.join("triage.stderr"), limits.hang).ok()?;
    match w.run(text, edits, seed, limits) {
        Outcome::Done { micros, .. } => Some(micros),
        Outcome::Failed { .. } => None,
    }
}

/// A hang is re-run alone under this multiple of the hang limit; what
/// returns is slow, what does not is a hang.
const HANG_CONFIRM_FACTOR: u32 = 12;

/// An allocation is re-run alone under this multiple of the RSS limit
/// (and the hang confirmation's time); what does not return is a hang.
const ALLOC_CONFIRM_FACTOR: u64 = 4;

/// Run `prefix` then `last` in one fresh worker and return the first
/// failure, if any: how a crash that needs its worker's history is
/// reproduced (E7h.3).
fn sequence_outcome(
    grammar: &str,
    prefix: &[Input],
    last: &Input,
    dir: &Path,
    limits: &Limits,
) -> Option<(Kind, String)> {
    let mut w = Worker::spawn(grammar, dir.join("triage.stderr"), limits.hang).ok()?;
    for input in prefix.iter().chain(std::iter::once(last)) {
        if let Outcome::Failed {
            kind, signature, ..
        } = w.run(&input.text, input.edits, input.seed, limits)
        {
            return Some((kind, signature));
        }
    }
    None
}

/// The fewest inputs from the end of `f.history` after which `f`'s input
/// crashes again with its own signature, or `None` if the whole history
/// does not bring it back. Doubling, then a binary search below the first
/// length that works (E7h.3).
fn shortest_crashing_suffix(
    grammar: &str,
    f: &Finding,
    dir: &Path,
    limits: &Limits,
) -> Option<usize> {
    let last = Input {
        text: f.input.clone(),
        edits: f.edits,
        seed: f.seed,
    };
    let n = f.history.len();
    let crashes = |k: usize| {
        sequence_outcome(grammar, &f.history[n - k..], &last, dir, limits)
            .is_some_and(|(kind, sig)| kind == Kind::Crash && sig == f.signature)
    };
    let mut hi = 1;
    while hi < n && !crashes(hi) {
        hi = (hi * 2).min(n);
    }
    if !crashes(hi) {
        return None;
    }
    let mut lo = hi / 2; // the largest length known not to crash, or 0
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if crashes(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(hi)
}

/// The inputs a sequence finding needs, beside its note: `<stem>.sequence/`
/// holds the last `k` inputs its worker ran before it, in order, and a
/// manifest with each one's edits and seed (E7h.3).
fn write_sequence(stem: &Path, f: &Finding, k: usize) {
    let seq = stem.with_extension("sequence");
    let _ = std::fs::create_dir_all(&seq);
    let mut manifest = String::from("n\tedits\tseed\tfile\n");
    let start = f.history.len() - k;
    for (i, input) in f.history[start..].iter().enumerate() {
        let name = format!("{i:03}.input");
        let _ = std::fs::write(seq.join(&name), &input.text);
        let _ = writeln!(manifest, "{i}\t{}\t{}\t{name}", input.edits, input.seed);
    }
    let _ = writeln!(
        manifest,
        "{k}\t{}\t{}\t(the finding's input)",
        f.edits, f.seed
    );
    let _ = std::fs::write(seq.join("sequence.tsv"), manifest);
}

fn triage(grammar: &str, mut f: Finding, dir: &Path, index: usize, limits: &Limits) -> Triaged {
    let want = (f.kind, f.signature.clone());
    let same = |text: &str, edits: u32| {
        outcome_alone(grammar, text, edits, f.seed, dir, limits)
            .is_some_and(|got| got.0 == want.0 && (want.0 != Kind::Crash || got.1 == want.1))
    };
    let alone = same(&f.input, f.edits);
    let mut repro = if alone { Repro::Alone } else { Repro::No };
    // E7h.3: a crash that does not come back alone is replayed after the
    // inputs its worker ran before it, shortest suffix first; state a
    // scanner carries across parses is exactly what the daemon, which
    // parses for days, would meet.
    if !alone
        && f.kind == Kind::Crash
        && !f.history.is_empty()
        && let Some(k) = shortest_crashing_suffix(grammar, &f, dir, limits)
    {
        repro = Repro::InSequence(k);
    }
    let long = Limits {
        hang: limits.hang * HANG_CONFIRM_FACTOR,
        rss_kb: limits.rss_kb * ALLOC_CONFIRM_FACTOR,
    };
    // A hang is timed alone under twelve times the limit before anything
    // is minimized: one that returns is slow, which D36 files and never
    // fails, and keeps the input it was found with rather than spending
    // the minimizer's five minutes at a hang limit an attempt (E7h.4: the
    // depth operators make Lua's error recovery slow on most runs).
    if alone
        && f.kind == Kind::Hang
        && let Some(micros) = time_alone(grammar, &f.input, f.edits, f.seed, dir, &long)
    {
        f.kind = Kind::Slow;
        let _ = write!(
            f.detail,
            "\nslow, not hung: it returned alone in {} ms under a {} s limit (not minimized)",
            micros / 1000,
            long.hang.as_secs()
        );
    }
    let (minimal, minimal_edits) = if !alone || f.kind == Kind::Slow {
        (f.input.clone(), f.edits)
    } else if same(&f.input, 0) {
        (minimize(&f.input, |t| same(t, 0)), 0)
    } else {
        (minimize(&f.input, |t| same(t, f.edits)), f.edits)
    };
    // A hang whose input ran past twelve times the limit may still be a
    // large parse quadratic in its bytes (CMake on 247 KB of whitespace:
    // 210 s natively, 4 s at 34 KB; E7h). Its minimal input, which still
    // runs past the limit, is timed the same way, and one that returns is
    // slow; only one that does not is a hang.
    if alone
        && f.kind == Kind::Hang
        && let Some(micros) = time_alone(grammar, &minimal, minimal_edits, f.seed, dir, &long)
    {
        f.kind = Kind::Slow;
        let _ = write!(
            f.detail,
            "\nslow, not hung: the input ran past the {} s limit alone, and its minimal \
             one returned in {} ms",
            long.hang.as_secs(),
            micros / 1000
        );
    }
    if alone && f.kind == Kind::Alloc {
        confirm_alloc(grammar, &mut f, &minimal, minimal_edits, dir, limits, &long);
    }
    let stem = dir.join(format!("{}-{index}", f.kind.name()));
    let file = stem.with_extension("input");
    let _ = std::fs::write(&file, &minimal);
    let _ = std::fs::write(stem.with_extension("original"), &f.input);
    if let Repro::InSequence(k) = repro {
        write_sequence(&stem, &f, k);
    }
    let reproduced = match repro {
        Repro::Alone => "alone".to_owned(),
        Repro::InSequence(k) => {
            format!("after the {k} inputs before it (see the .sequence directory)")
        }
        Repro::No => "no".to_owned(),
    };
    let note = format!(
        "grammar: {grammar}\nkind: {}\nsignature: {}\nreproduced: {reproduced}\n\
         minimal: {} bytes (from {}), edits {minimal_edits}, seed {}\n\
         repro: pmacs_grammar_fuzz repro {grammar} {} --edits {minimal_edits} --seed {}\n\n{}",
        f.kind.name(),
        f.signature,
        minimal.len(),
        f.input.len(),
        f.seed,
        file.display(),
        f.seed,
        f.detail
    );
    let _ = std::fs::write(stem.with_extension("txt"), note);
    Triaged {
        finding: f,
        repro,
        minimal,
        minimal_edits,
        file,
    }
}

/// E7h.3: an allocation is confirmed the way a hang is. A parse that grows
/// past the RSS limit before the hang limit may never return at all (a
/// cycle that allocates each turn); run alone under twelve times the time
/// and four times the memory (`long`), one that still has not returned is
/// a hang, and only one that returns is large.
fn confirm_alloc(
    grammar: &str,
    f: &mut Finding,
    minimal: &str,
    minimal_edits: u32,
    dir: &Path,
    limits: &Limits,
    long: &Limits,
) {
    if let Some(micros) = time_alone(grammar, minimal, minimal_edits, f.seed, dir, long) {
        let _ = write!(
            f.detail,
            "\nlarge, and it returns: alone in {} ms under {} MB and {} s",
            micros / 1000,
            long.rss_kb / 1024,
            long.hang.as_secs()
        );
    } else {
        f.kind = Kind::Hang;
        f.signature = format!("never returned, growing past {} MB", limits.rss_kb / 1024);
        let _ = write!(
            f.detail,
            "\nnot large but hung: alone it had not returned under {} MB and {} s",
            long.rss_kb / 1024,
            long.hang.as_secs()
        );
    }
}

/// Delta debugging over lines, then characters, under a budget: keep a
/// candidate only while `still` holds for it.
fn minimize(input: &str, mut still: impl FnMut(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_mins(5);
    let mut cur = input.to_owned();
    for by_char in [false, true] {
        let mut units: Vec<String> = if by_char {
            if cur.len() > 4096 {
                break;
            }
            cur.chars().map(String::from).collect()
        } else {
            cur.split_inclusive('\n').map(str::to_owned).collect()
        };
        let mut n = 2;
        while units.len() >= 2 && Instant::now() < deadline {
            let chunk = units.len().div_ceil(n);
            let mut reduced = false;
            for start in (0..units.len()).step_by(chunk) {
                let mut rest = units.clone();
                rest.drain(start..(start + chunk).min(units.len()));
                if still(&rest.concat()) {
                    units = rest;
                    n = (n - 1).max(2);
                    reduced = true;
                    break;
                }
            }
            if !reduced {
                if n >= units.len() {
                    break;
                }
                n = (n * 2).min(units.len());
            }
        }
        cur = units.concat();
    }
    cur
}

// -------------------------------------------------------------------
// Report
// -------------------------------------------------------------------

fn write_report(cfg: &Config, reports: &[Report]) -> Result<(), String> {
    let mut md = String::new();
    let mut tsv = String::from(
        "grammar\tseeds\tinputs\tparses\tmutated\tMB\tslowest_ms\tseconds\tgrowth_mb\tcrashes\thangs\tslow\tallocs\tin_sequence\tunconfirmed\terror\n",
    );
    let _ = writeln!(
        md,
        "# Grammar fuzz report\n\n{} s per grammar after the seeds, {} edits per input, seed {}, \
         hang {} ms per parse, RSS {} MB. MB is the text fed in; slowest is one input's whole exercise, kept as \
         `GRAMMAR/slowest.input` with its `slowest.seed`; \
         s is the grammar's wall time, triage included; growth is the most a worker's RSS gained over \
         its inputs before it was replaced; mutated counts the inputs after the seeds. Crashes, \
         hangs, slow and allocs count findings that came back, alone or after the inputs \
         before them (in sequence counts the latter); the last column those that came back \
         neither way. A crash, however it came back, and a hang that came back fail the run.\n\n\
         | grammar | seeds | inputs | mutated | parses | MB | slowest ms (bytes) | s | growth MB | crashes | hangs | slow | allocs | in sequence | unconfirmed |\n\
         |---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|",
        cfg.seconds,
        cfg.edits,
        cfg.seed,
        cfg.limits.hang.as_millis(),
        cfg.limits.rss_kb / 1024
    );
    for r in reports {
        let count = |k: Kind| {
            r.findings
                .iter()
                .filter(|t| t.repro != Repro::No && t.finding.kind == k)
                .count()
        };
        let in_sequence = r
            .findings
            .iter()
            .filter(|t| matches!(t.repro, Repro::InSequence(_)))
            .count();
        let mutated = r.stats.inputs.saturating_sub(r.stats.seeds as u64);
        let s = &r.stats;
        let (c, h, a) = (count(Kind::Crash), count(Kind::Hang), count(Kind::Alloc));
        let slow_n = count(Kind::Slow);
        let growth = s.growth_kb / 1024;
        let unconfirmed = r.findings.iter().filter(|t| t.repro == Repro::No).count();
        let secs = s.elapsed.as_secs();
        let mb = s.bytes / (1024 * 1024);
        let slow = s.slowest_micros / 1000;
        let err = r.error.as_deref().unwrap_or("");
        let _ = writeln!(
            md,
            "| {} | {} | {} | {mutated} | {} | {mb} | {slow} ({}) | {secs} | {growth} | {c} | {h} | {slow_n} | {a} | {in_sequence} | {unconfirmed} |{}",
            r.grammar,
            s.seeds,
            s.inputs,
            s.parses,
            s.slowest_len,
            if err.is_empty() {
                String::new()
            } else {
                format!(" ERROR {err}")
            }
        );
        let _ = writeln!(
            tsv,
            "{}\t{}\t{}\t{}\t{mutated}\t{mb}\t{slow}\t{secs}\t{growth}\t{c}\t{h}\t{slow_n}\t{a}\t{in_sequence}\t{unconfirmed}\t{err}",
            r.grammar, s.seeds, s.inputs, s.parses
        );
    }
    for (heading, came_back) in [("Findings", true), ("Not reproduced", false)] {
        let _ = writeln!(md, "\n## {heading}\n");
        let mut none = true;
        for r in reports {
            for t in r
                .findings
                .iter()
                .filter(|t| (t.repro != Repro::No) == came_back)
            {
                none = false;
                let how = match t.repro {
                    Repro::Alone => "alone".to_owned(),
                    Repro::InSequence(k) => format!("after the {k} inputs before it"),
                    Repro::No => "no".to_owned(),
                };
                let _ = writeln!(
                    md,
                    "- **{}** {} `{}`, reproduced: {}, minimal {} bytes, edits {}: `{}`\n\n```\n{}\n```\n",
                    r.grammar,
                    t.finding.kind.name(),
                    t.finding.signature,
                    how,
                    t.minimal.len(),
                    t.minimal_edits,
                    t.file.display(),
                    shown(&t.minimal)
                );
            }
        }
        if none {
            let _ = writeln!(md, "None.");
        }
    }
    std::fs::write(cfg.out.join("report.md"), md).map_err(|e| e.to_string())?;
    std::fs::write(cfg.out.join("report.tsv"), tsv).map_err(|e| e.to_string())
}

/// A minimal input as the report shows it: escaped, and cut at 400
/// characters (the `.input` file beside it holds it whole).
fn shown(text: &str) -> String {
    let escaped: String = text.escape_debug().collect();
    match escaped.char_indices().nth(400) {
        Some((cut, _)) => format!("{}… ({} bytes; see the file)", &escaped[..cut], text.len()),
        None => escaped,
    }
}

// -------------------------------------------------------------------
// `changed`: which grammars a change touched (E7h.4).
// -------------------------------------------------------------------

/// Every `[[package]]`'s versions in a `Cargo.lock`, by name.
fn locked_versions(lock: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut name: Option<&str> = None;
    for line in lock.lines() {
        if line == "[[package]]" {
            name = None;
        } else if let Some(n) = line.strip_prefix("name = \"") {
            name = n.strip_suffix('"');
        } else if let (Some(n), Some(v)) = (name, line.strip_prefix("version = \"")) {
            out.entry(n.to_owned())
                .or_default()
                .insert(v.trim_end_matches('"').to_owned());
            name = None;
        }
    }
    out
}

/// `fuzz/corpora.tsv`'s rows as `(grammar, crate, whole row)`.
fn corpora_rows(tsv: &str) -> Vec<(String, String, String)> {
    tsv.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty() && !l.starts_with("grammar\t"))
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some((f.next()?.to_owned(), f.next()?.to_owned(), l.to_owned()))
        })
        .collect()
}

/// The grammars a change touched, and whether it touched every grammar
/// (the tree-sitter runtime's version moved). A grammar is touched when its
/// crate's locked version differs, its corpora row differs or is new, or a
/// file under `vendor/<its crate>/` changed.
fn changed_grammars(
    lock_then: &str,
    lock_now: &str,
    rows_then: &[(String, String, String)],
    rows_now: &[(String, String, String)],
    vendored: &BTreeSet<String>,
) -> (bool, BTreeSet<String>) {
    let then = locked_versions(lock_then);
    let now = locked_versions(lock_now);
    let all = then.get("tree-sitter") != now.get("tree-sitter");
    let mut touched = BTreeSet::new();
    for (grammar, krate, row) in rows_now {
        let moved = then.get(krate) != now.get(krate);
        let row_changed = !rows_then.iter().any(|(_, _, r)| r == row);
        if moved || row_changed || vendored.contains(krate) {
            touched.insert(grammar.clone());
        }
    }
    (all, touched)
}

fn git_text(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Print the grammars the change since `--since REF` touched, one a line,
/// or `*` when it touched them all; run from the repository root.
fn changed(args: &[String]) -> Result<ExitCode, String> {
    let (_, flags) = Flags::parse(args, 0)?;
    let since = flags.one("since").ok_or("--since REF is required")?;
    let lock_then = git_text(&["show", &format!("{since}:Cargo.lock")])?;
    let lock_now = std::fs::read_to_string("Cargo.lock").map_err(|e| format!("Cargo.lock: {e}"))?;
    let rows_then = corpora_rows(
        &git_text(&["show", &format!("{since}:fuzz/corpora.tsv")]).unwrap_or_default(),
    );
    let rows_now = corpora_rows(
        &std::fs::read_to_string("fuzz/corpora.tsv")
            .map_err(|e| format!("fuzz/corpora.tsv: {e}"))?,
    );
    let vendored: BTreeSet<String> = git_text(&["diff", "--name-only", since, "--", "vendor/"])?
        .lines()
        .filter_map(|p| {
            p.strip_prefix("vendor/")?
                .split('/')
                .next()
                .map(str::to_owned)
        })
        .collect();
    let (all, touched) = changed_grammars(&lock_then, &lock_now, &rows_then, &rows_now, &vendored);
    if all {
        println!("*");
    } else {
        for g in touched {
            println!("{g}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

// -------------------------------------------------------------------
// `repro`: one input, in this process, so a sanitizer or a debugger
// sees it directly.
// -------------------------------------------------------------------

fn repro(args: &[String]) -> Result<ExitCode, String> {
    let (pos, flags) = Flags::parse(args, 2)?;
    let lang = entry(&pos[0])?;
    let text = std::fs::read(&pos[1]).map_err(|e| format!("{}: {e}", pos[1]))?;
    let text = String::from_utf8(text).map_err(|e| e.to_string())?;
    let edits = u32::try_from(flags.num("edits", 0)?).map_err(|e| e.to_string())?;
    let mut session = Session::new(lang);
    session.trace = flags.num("trace", 0)? != 0;
    session.deadline = match flags.num("deadline-ms", 0)? {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    };
    match session.exercise(text, edits, flags.num("seed", 1)?, &mut || {}) {
        Ok(parses) => {
            println!("{}: {parses} parses, no fault", lang.name);
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            println!("{}: {e}", lang.name);
            Ok(ExitCode::from(1))
        }
    }
}

// -------------------------------------------------------------------
// `collect`: seeds from real files and upstream test corpora.
// -------------------------------------------------------------------

/// Exact filenames the editor maps to a grammar, mirroring
/// `pmacs.parse.filenames` in `builtin/runtime/syntax.lua`.
const FILENAMES: &[(&str, &str)] = &[
    ("Dockerfile", "dockerfile"),
    ("Containerfile", "dockerfile"),
    ("Makefile", "make"),
    ("makefile", "make"),
    ("GNUmakefile", "make"),
    ("BSDmakefile", "make"),
    ("CMakeLists.txt", "cmake"),
    (".bashrc", "bash"),
    (".bash_profile", "bash"),
    (".profile", "bash"),
    (".zshrc", "bash"),
    ("PKGBUILD", "bash"),
];

/// Grammars no file claims, fed from the grammar that injects them.
const INJECTED_FROM: &[(&str, &str)] = &[("markdown_inline", "markdown")];

fn grammar_for(registry: &SyntaxRegistry, path: &Path) -> Option<String> {
    let base = path.file_name()?.to_str()?;
    if let Some((_, g)) = FILENAMES.iter().find(|(n, _)| *n == base) {
        return Some((*g).to_owned());
    }
    registry.language_name_for_path(path.to_str()?)
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    if root.join(".git").exists()
        && let Ok(out) = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-files", "-z"])
            .output()
        && out.status.success()
    {
        return out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| root.join(String::from_utf8_lossy(p).as_ref()))
            .collect();
    }
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    let skip = ["target", "node_modules", "build", "dist", "__pycache__"];
                    if !name.starts_with('.') && !skip.contains(&name.as_str()) {
                        stack.push(p);
                    }
                }
                Ok(t) if t.is_file() => out.push(p),
                _ => {}
            }
        }
    }
    out
}

/// A seed as the editor would hold it: UTF-8 text without NUL, cut at a
/// line boundary.
fn as_seed(bytes: Vec<u8>, max_bytes: usize) -> Option<String> {
    if bytes.len() > 4 * 1024 * 1024 || bytes.contains(&0) {
        return None;
    }
    let mut s = String::from_utf8(bytes).ok()?;
    if s.len() > max_bytes {
        let mut cut = max_bytes;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        cut = s[..cut].rfind('\n').map_or(cut, |i| i + 1);
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    (!s.trim().is_empty()).then_some(s)
}

/// The examples of a tree-sitter test corpus file: a header (an `===`
/// line, the name, the same `===` line), the source, a `---` line with
/// the header's suffix, the tree. The divider is the last one before the
/// next header, since source (YAML, Markdown) may hold `---` lines itself.
fn corpus_examples(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let delim = |l: &str, c: char| {
        let run = l.chars().take_while(|x| *x == c).count();
        (run >= 3).then(|| l[run..].trim_end().to_owned())
    };
    let mut headers = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(s) = delim(lines[i], '=') {
            let close = (i + 2..(i + 6).min(lines.len()))
                .find(|&j| delim(lines[j], '=') == Some(s.clone()));
            if let Some(j) = close {
                headers.push((i, j + 1, s));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    let mut out = Vec::new();
    for (k, (_, body, suffix)) in headers.iter().enumerate() {
        let stop = headers.get(k + 1).map_or(lines.len(), |h| h.0);
        let divider = (*body..stop)
            .rev()
            .find(|&d| delim(lines[d], '-').as_ref() == Some(suffix));
        if let Some(d) = divider {
            let mut code = lines[*body..d].join("\n");
            code.push('\n');
            if !code.trim().is_empty() {
                out.push(code);
            }
        }
    }
    out
}

/// Files by `(grammar, source label)`, each `(sort key, relative path,
/// full path)`; the key is a hash of the relative path, so a capped
/// source is sampled the same way on every machine.
type Groups = BTreeMap<(String, String), Vec<(u64, String, String)>>;

fn gather(sources: &[&str]) -> Groups {
    let registry = SyntaxRegistry::new();
    let mut groups = Groups::new();
    for spec in sources {
        let (label, root) = spec.split_once('=').map_or_else(
            || {
                let root = Path::new(spec);
                let name = root
                    .file_name()
                    .map_or("src".into(), |n| n.to_string_lossy().into_owned());
                (name, root.to_path_buf())
            },
            |(label, root)| (label.to_owned(), PathBuf::from(root)),
        );
        for path in files_under(&root) {
            let Some(grammar) = grammar_for(&registry, &path) else {
                continue;
            };
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let key = fnv(rel.as_bytes());
            let full = path.display().to_string();
            groups
                .entry((grammar, label.clone()))
                .or_default()
                .push((key, rel, full));
        }
    }
    groups
}

fn collect(args: &[String]) -> Result<ExitCode, String> {
    let (_, flags) = Flags::parse(args, 0)?;
    let out = flags.path("out")?;
    let max_files = usize::try_from(flags.num("max-files", 400)?).map_err(|e| e.to_string())?;
    // E7h.4: a smaller cut makes every input cheaper, so a short run
    // mutates each grammar more (Zig's standard-library seeds run to 64 KB).
    let max_seed = usize::try_from(flags.num("max-seed-kb", (MAX_SEED_BYTES / 1024) as u64)?)
        .map_err(|e| e.to_string())?
        * 1024;
    let groups = gather(&flags.all("from"));
    let mut written: BTreeMap<String, usize> = BTreeMap::new();
    let mut manifest = String::from("grammar\tsource\tpath\tbytes\n");
    for ((grammar, label), mut files) in groups {
        files.sort();
        let mut taken = 0;
        for (key, rel, full) in files {
            if taken >= max_files {
                break;
            }
            let Some(text) = std::fs::read(&full).ok().and_then(|b| as_seed(b, max_seed)) else {
                continue;
            };
            taken += 1;
            let mut targets = vec![grammar.clone()];
            targets.extend(
                INJECTED_FROM
                    .iter()
                    .filter(|(_, from)| *from == grammar)
                    .map(|(g, _)| (*g).to_owned()),
            );
            for g in targets {
                write_seed(&out, &g, &format!("{label}--{key:016x}"), &text)?;
                *written.entry(g.clone()).or_default() += 1;
                let _ = writeln!(manifest, "{g}\t{label}\t{rel}\t{}", text.len());
            }
        }
    }
    for spec in flags.all("corpus") {
        let (grammar, root) = spec
            .split_once('=')
            .ok_or_else(|| format!("--corpus {spec}: want GRAMMAR=PATH"))?;
        entry(grammar)?;
        let mut files = files_under(Path::new(root));
        files.sort();
        // Every file: corpora are `.txt` by convention, make's are `.mk`.
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            for (n, code) in corpus_examples(&text).into_iter().enumerate() {
                let Some(code) = as_seed(code.into_bytes(), max_seed) else {
                    continue;
                };
                let key = fnv(format!("{}#{n}", path.display()).as_bytes());
                write_seed(&out, grammar, &format!("upstream--{key:016x}"), &code)?;
                *written.entry(grammar.to_owned()).or_default() += 1;
                let _ = writeln!(
                    manifest,
                    "{grammar}\tupstream\t{}#{n}\t{}",
                    path.display(),
                    code.len()
                );
            }
        }
    }
    std::fs::write(out.join("MANIFEST.tsv"), manifest).map_err(|e| e.to_string())?;
    let mut missing = Vec::new();
    for e in BUILTIN_LANGUAGES {
        let n = written.get(e.name).copied().unwrap_or(0);
        eprintln!("{}: {n} seeds", e.name);
        if n == 0 {
            missing.push(e.name);
        }
    }
    if missing.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Err(format!("no seeds for {}", missing.join(", ")))
    }
}

fn write_seed(out: &Path, grammar: &str, name: &str, text: &str) -> Result<(), String> {
    let dir = out.join(grammar);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(name), text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_examples_take_the_source_between_header_and_divider() {
        let text = "==========\nfirst\n==========\n\nfn a() {}\n\n---\n\n(source_file)\n\n\
                    ===|||\nsecond\n===|||\nx\n---|||\n(y)\n";
        assert_eq!(
            corpus_examples(text),
            vec!["\nfn a() {}\n\n".to_owned(), "x\n".to_owned()]
        );
    }

    #[test]
    fn edits_keep_text_and_positions_in_step() {
        let mut rng = Rng(7);
        let mut text = "fn main() {\n    let é = 1;\n}\n".to_owned();
        for _ in 0..500 {
            let (next, e) = random_edit(&text, &mut rng);
            assert_eq!(
                e.start_position,
                byte_to_point(text.as_bytes(), e.start_byte)
            );
            assert_eq!(
                e.new_end_position,
                byte_to_point(next.as_bytes(), e.new_end_byte)
            );
            assert_eq!(next.len() + e.old_end_byte, text.len() + e.new_end_byte);
            text = next;
        }
    }

    #[test]
    fn a_session_parses_and_reparses_every_grammar() {
        for lang in BUILTIN_LANGUAGES {
            let parses = Session::new(lang)
                .exercise("a (b) {c}\n".to_owned(), 3, 1, &mut || {})
                .expect("no deadline, so every parse returns");
            assert!(parses >= 4, "{}: {parses} parses", lang.name);
        }
    }

    #[test]
    fn what_fails_the_run_is_a_crash_or_a_hang_that_came_back() {
        let report =
            |kind: Kind, repro: Option<Repro>, signature: &str, error: Option<&str>| Report {
                grammar: "rust",
                stats: Stats::default(),
                findings: repro
                    .map(|repro| Triaged {
                        finding: Finding {
                            kind,
                            signature: signature.to_owned(),
                            input: String::new(),
                            edits: 0,
                            seed: 0,
                            detail: String::new(),
                            history: Vec::new(),
                        },
                        repro,
                        minimal: String::new(),
                        minimal_edits: 0,
                        file: PathBuf::new(),
                    })
                    .into_iter()
                    .collect(),
                error: error.map(str::to_owned),
            };
        let fails_with =
            |kind, repro, signature| fails(&[report(kind, Some(repro), signature, None)]);
        assert!(!fails(&[report(Kind::Hang, None, "", None)]), "no finding");
        assert!(
            fails_with(Kind::Hang, Repro::Alone, ""),
            "a hang that came back"
        );
        assert!(
            !fails_with(Kind::Hang, Repro::No, ""),
            "a hang of a loaded worker"
        );
        assert!(fails_with(Kind::Crash, Repro::Alone, "asan x in y"));
        assert!(
            fails_with(Kind::Crash, Repro::InSequence(2), "asan x in y"),
            "a crash that needs the inputs before it fails the run (E7h.3)"
        );
        assert!(
            fails_with(Kind::Crash, Repro::No, "asan x in y"),
            "a crash that came back neither way still fails it"
        );
        assert!(
            !fails_with(Kind::Crash, Repro::No, "signal 9"),
            "the host killing a worker for memory is not the grammar's crash"
        );
        assert!(
            !fails_with(Kind::Slow, Repro::Alone, ""),
            "a slow parse is reported, not failed"
        );
        assert!(
            !fails_with(Kind::Alloc, Repro::Alone, ""),
            "a large parse is reported, not failed"
        );
        assert!(fails(&[report(Kind::Hang, None, "", Some("no seeds"))]));
    }

    #[test]
    fn signatures_tell_asserts_and_ubsan_apart() {
        let assert = "pmacs_grammar_fuzz: /x/tree-sitter-0.26.8/src/./parser.c:409: \
                      ts_parser__external_scanner_serialize: Assertion `length <= 1024' failed.\n";
        assert_eq!(
            crash_signature(assert, "signal 6"),
            "assert `length <= 1024` at parser.c:409 ts_parser__external_scanner_serialize"
        );
        let ubsan = "src/scanner.c:42:7: runtime error: signed integer overflow: 2147483647 + 1 cannot be represented\n";
        assert_eq!(
            crash_signature(ubsan, "signal 6"),
            "ubsan signed integer overflow at scanner.c:42:7"
        );
    }

    #[test]
    fn changed_names_the_grammars_a_lock_bump_a_row_or_a_vendored_crate_touched() {
        let lock = |bash: &str, ts: &str| {
            format!(
                "[[package]]\nname = \"tree-sitter\"\nversion = \"{ts}\"\n\n\
                 [[package]]\nname = \"tree-sitter-bash\"\nversion = \"{bash}\"\n\n\
                 [[package]]\nname = \"tree-sitter-md\"\nversion = \"0.5.3\"\n"
            )
        };
        let rows = |md_commit: &str| {
            corpora_rows(&format!(
                "grammar\tcrate\tversion\n\
                 bash\ttree-sitter-bash\t0.25.1\n\
                 markdown\ttree-sitter-md\t0.5.3\t{md_commit}\n\
                 markdown_inline\ttree-sitter-md\t0.5.3\t{md_commit}\n"
            ))
        };
        let none = BTreeSet::new();
        // A lockfile-only bump of one grammar crate touches that grammar.
        let (all, touched) = changed_grammars(
            &lock("0.25.1", "0.26.8"),
            &lock("0.25.2", "0.26.8"),
            &rows("a"),
            &rows("a"),
            &none,
        );
        assert!(!all);
        assert_eq!(touched, BTreeSet::from(["bash".to_owned()]));
        // A vendored crate's change touches every grammar it carries.
        let md = BTreeSet::from(["tree-sitter-md".to_owned()]);
        let (_, touched) = changed_grammars(
            &lock("0.25.1", "0.26.8"),
            &lock("0.25.1", "0.26.8"),
            &rows("a"),
            &rows("a"),
            &md,
        );
        assert_eq!(
            touched,
            BTreeSet::from(["markdown".to_owned(), "markdown_inline".to_owned()])
        );
        // A re-pinned corpora row touches its grammars.
        let (_, touched) = changed_grammars(
            &lock("0.25.1", "0.26.8"),
            &lock("0.25.1", "0.26.8"),
            &rows("a"),
            &rows("b"),
            &none,
        );
        assert!(touched.contains("markdown"));
        // The runtime's bump touches them all.
        let (all, _) = changed_grammars(
            &lock("0.25.1", "0.26.8"),
            &lock("0.25.1", "0.27.0"),
            &rows("a"),
            &rows("a"),
            &none,
        );
        assert!(all);
    }

    #[test]
    fn the_depth_operators_nest_past_the_scanner_state_bounds() {
        // E7h.4: the serialization overruns sit at 254/255 and 511 levels.
        let mut rng = Rng(3);
        let mut deepest_prefix = 0;
        let mut deepest_stair = 0;
        for _ in 0..40 {
            let p = depth_prefix("> quoted\nplain\n", &mut rng);
            deepest_prefix = deepest_prefix.max(
                p.lines()
                    .map(|l| l.matches("> ").count())
                    .max()
                    .unwrap_or(0),
            );
            let st = depth_stair("import os\nif x:\n    y = 1\nz = \"s\"\nw = 2\n", &mut rng);
            // Levels of `if x:` stepping one space, with a string below.
            let levels = st.lines().filter(|l| l.trim() == "if x:").count();
            if st.contains(&format!("\n{}z = \"s\"\n", " ".repeat(levels))) {
                deepest_stair = deepest_stair.max(levels);
            }
            assert!(
                st.len() <= MAX_INPUT_BYTES + 64,
                "a stair fits the input cap"
            );
        }
        assert!(deepest_prefix >= 512, "a prefix past 511: {deepest_prefix}");
        assert!(
            deepest_stair >= 511,
            "a stair of block openers 511 deep with a string at the bottom: {deepest_stair}"
        );
    }

    #[test]
    fn minimize_keeps_what_the_predicate_needs() {
        let input = "one\ntwo\nNEEDLE\nthree\n";
        assert_eq!(minimize(input, |t| t.contains("NEED")), "NEED");
    }

    #[test]
    fn signatures_name_the_sanitizer_frame_and_glibc() {
        let asan = "==1==ERROR: AddressSanitizer: heap-buffer-overflow on address\n\
                    WRITE of size 4\n    #0 0x55 in advance.part.0 (/x+0x1)\n";
        assert_eq!(
            crash_signature(asan, "signal 6"),
            "asan heap-buffer-overflow in advance"
        );
        assert_eq!(
            crash_signature("corrupted size vs. prev_size\n", "signal 6"),
            "glibc: corrupted size vs. prev_size"
        );
        assert_eq!(crash_signature("", "signal 11"), "signal 11");
    }
}
