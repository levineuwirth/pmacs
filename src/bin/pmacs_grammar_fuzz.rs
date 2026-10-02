// pmacs_grammar_fuzz.rs --- fuzz every bundled tree-sitter grammar (E7g).

//! `pmacs_grammar_fuzz` --- parse real and mutated source against every
//! grammar in [`pmacs::syntax::BUILTIN_LANGUAGES`] and report what aborts,
//! hangs, or allocates without bound.
//!
//! `#![forbid(unsafe_code)]` covers the Rust; the grammars are C, and one
//! of them (tree-sitter-haskell 0.23.1, E7g) corrupted the heap on the
//! most ordinary Haskell file there is. This binary is how a grammar earns
//! its place: `scripts/fuzz-grammars` builds it the way the grammars ship
//! (release optimization, the C instrumented, once per arm:
//! `AddressSanitizer` and `UndefinedBehaviorSanitizer`; `AddressSanitizer`
//! alone with strict aliasing restored; clang's `TypeSanitizer`), seeds it
//! from real files, and runs it.
//!
//! ```text
//! pmacs_grammar_fuzz list
//! pmacs_grammar_fuzz collect --out DIR [--from [LABEL=]PATH]...
//!                            [--corpus GRAMMAR=PATH]... [--max-files N]
//! pmacs_grammar_fuzz run --corpus DIR --out DIR [--seconds N] [--jobs N]
//!                        [--grammar NAME]... [--edits N] [--seed N]
//!                        [--hang-ms N] [--rss-mb N] [--min-mutations N]
//!                        [--max-seconds N] [--long NAME]... [--long-seconds N]
//!                        [--fail-on all|crashes]
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
//! heap check, an assert, a panic; or `TypeSanitizer` reported an access of
//! another type), hangs (a parse that has not returned after twelve times
//! the hang limit, alone, nor, when its minimal input returns, after ten
//! times that), memory cuts (a parse cut alone at four times `--rss-mb`),
//! slow parses (over the limit but returning), and allocations (one input
//! growing the worker's RSS by more than `--rss-mb`, returning under four
//! times it). Each is confirmed alone in a fresh worker on the input that
//! showed it, which decides its kind, then, unless it is slow, minimized,
//! with the minimum confirmed and reported beside it (E7h's fix round 2).
//!
//! `run` exits 1 on a crash, a hang or a memory cut, 2 on a usage or setup
//! error (a grammar with no seeds is one: the rule is corpus-seeded), and 0
//! otherwise. That is the owner's ruling at E7g (D36): what aborts or never
//! returns is not shipped, what is slow or large is filed. E7h.3 made the
//! line hold where review 1's planted defects showed it did not: every
//! parse runs under the hang limit as its deadline, so a parse that never
//! returns is cancelled and filed as a hang whatever it allocates; an
//! allocation is confirmed under four times the memory and twelve times the
//! time, one that still has not returned is a hang, and (E7h's fix round 1,
//! the owner's ruling) one cut at the four-times memory cap is `memory`,
//! "exceeded memory cap", with the RSS at the cut, which fails the run; the
//! time limit is a trigger and not a verdict (fix round 3, the owner's
//! ruling), so an input past twelve times it whose minimum returns is run
//! again under ten times that, slow if it returns there and a hang if not;
//! a finding `fuzz/accepted.tsv` names is reported and does not fail; and a
//! crash that
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
    /// The input touches `PMACS_FUZZ_SELFTEST_MB` (default 400) in 64 MB
    /// steps, frees it and returns: past four times a 64 MB limit, and
    /// returning (review 1's big-return plant; E7h fix round 1).
    BigReturn,
    /// The input touches `PMACS_FUZZ_SELFTEST_MB` (default 100) for each
    /// trigger it holds, then frees it and returns: memory that grows with
    /// the input, as #296's does (E7h review 2, High 2). One trigger passes
    /// a 64 MB limit and returns under four times it; four pass the cap.
    Scaled,
    /// The input touches `PMACS_FUZZ_SELFTEST_MB` (default 160), then the
    /// worker aborts: an allocation whose confirmation crashes (E7h review
    /// 2, Low 4).
    GrowCrash,
    /// The input holds `PMACS_FUZZ_SELFTEST_MB` (default 96) while it sleeps
    /// `PMACS_FUZZ_SELFTEST_MS` for each trigger it holds, then returns: a
    /// first parse that meets the memory limit before the time limit, on an
    /// input whose time grows with it, as bash's large inputs did on CI's
    /// `asan-strict` leg (E7h fix round 2).
    SizedAlloc,
    /// The input returns after `PMACS_FUZZ_SELFTEST_MS` doubled for each
    /// trigger past the first: a cost exponential in its input, as #301's is
    /// in its openers, so the minimal input returns where the input as found
    /// does not within ten times the confirmation's limit (E7h fix round 3).
    Doubling,
}

impl SelfTest {
    fn from_env() -> Option<Self> {
        match std::env::var("PMACS_FUZZ_SELFTEST").ok()?.as_str() {
            "crash" => Some(Self::Crash),
            "hang" => Some(Self::Hang),
            "grow" => Some(Self::Grow),
            "slow" => Some(Self::Slow),
            "sized" => Some(Self::Sized),
            "sizedalloc" => Some(Self::SizedAlloc),
            "doubling" => Some(Self::Doubling),
            "sequence" => Some(Self::Sequence),
            "bigreturn" => Some(Self::BigReturn),
            "scaled" => Some(Self::Scaled),
            "growcrash" => Some(Self::GrowCrash),
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
            Self::Doubling => {
                let ms: u64 = std::env::var("PMACS_FUZZ_SELFTEST_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2500);
                let times = text.matches(SELFTEST_TRIGGER).count().clamp(1, 40) as u32;
                std::thread::sleep(Duration::from_millis(ms.saturating_mul(1 << (times - 1))));
            }
            Self::SizedAlloc => {
                let mb: usize = std::env::var("PMACS_FUZZ_SELFTEST_MB")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(96);
                let ms: u64 = std::env::var("PMACS_FUZZ_SELFTEST_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2500);
                let held = vec![1u8; mb << 20];
                std::hint::black_box(&held);
                let times = text.matches(SELFTEST_TRIGGER).count() as u64;
                std::thread::sleep(Duration::from_millis(ms * times));
                drop(held);
            }
            Self::Sequence => {
                if SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 >= 3 {
                    std::process::abort();
                }
            }
            Self::BigReturn | Self::Scaled | Self::GrowCrash => {
                let each: usize = std::env::var("PMACS_FUZZ_SELFTEST_MB")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(match self {
                        Self::BigReturn => 400,
                        Self::Scaled => 100,
                        _ => 160,
                    });
                let mb = if self == Self::Scaled {
                    each * text.matches(SELFTEST_TRIGGER).count()
                } else {
                    each
                };
                // Review 1's plant in 64 MB steps as it was; the others finer.
                // The crashing plant slowly enough that a poll sees it pass the
                // first limit before it aborts.
                let (step, pause) = match self {
                    Self::BigReturn => (64, 20),
                    Self::GrowCrash => (16, 25),
                    _ => (16, 5),
                };
                let mut held: Vec<Vec<u8>> = Vec::new();
                for _ in 0..mb.div_ceil(step) {
                    held.push(vec![1; step << 20]);
                    std::hint::black_box(&held);
                    std::thread::sleep(Duration::from_millis(pause));
                }
                if self == Self::GrowCrash {
                    std::process::abort();
                }
                drop(held);
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
    /// A parse cut, alone, at the confirmation's memory cap (four times the
    /// RSS limit): the owner's ruling at E7h's fix round 1, on review 1's
    /// Medium 2. Whether it would have returned is not known and does not
    /// matter: a parse that grows so far takes the editor out of memory, as
    /// #296's does, so it fails the run. It is not a hang, which is the time
    /// limit's, and not slow.
    Memory,
    /// A parse over the hang limit that did return alone.
    Slow,
    Alloc,
}

impl Kind {
    fn from_name(name: &str) -> Option<Self> {
        [
            Self::Crash,
            Self::Hang,
            Self::Memory,
            Self::Slow,
            Self::Alloc,
        ]
        .into_iter()
        .find(|k| k.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Hang => "hang",
            Self::Memory => "memory",
            Self::Slow => "slow",
            Self::Alloc => "alloc",
        }
    }
}

enum Outcome {
    Done {
        parses: u64,
        micros: u64,
        /// The worker's peak RSS (`VmHWM`) in kB when the input returned:
        /// the input's own peak in a worker spawned for it alone.
        peak_kb: u64,
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
    /// How much of `stderr` has been read for a report that did not stop
    /// the worker (`sanitizer_report`).
    stderr_seen: u64,
    /// When the last input was cut for its memory: the worker's peak RSS
    /// in kB and how long the input had run.
    memory_cut: Option<(u64, Duration)>,
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
            stderr_seen: 0,
            memory_cut: None,
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
        let started = last;
        self.memory_cut = None;
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
                    if let Some((signature, detail)) = self.sanitizer_report() {
                        self.kill();
                        return Outcome::Failed {
                            kind: Kind::Crash,
                            signature,
                            detail,
                        };
                    }
                    let mut f = line.split_whitespace().skip(1).map(str::parse::<u64>);
                    let parses = f.next().and_then(Result::ok).unwrap_or(0);
                    let micros = f.next().and_then(Result::ok).unwrap_or(0);
                    let peak_kb = f.next().and_then(Result::ok).unwrap_or(0);
                    return Outcome::Done {
                        parses,
                        micros,
                        peak_kb,
                    };
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
                let peak_kb = proc_status_kb(&pid, "VmHWM:").max(rss);
                self.memory_cut = Some((peak_kb, started.elapsed()));
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

    /// A report a sanitizer wrote without stopping the worker while the
    /// last input ran. clang's `TypeSanitizer` (the `tysan` arm, E7h fix
    /// round 1) prints an aliasing violation and carries on --- it has no
    /// `halt_on_error` --- so its findings are read from the worker's stderr
    /// rather than from its death, and filed as crashes, which fail the run.
    /// Only a report whose access type differs from the object's is one
    /// ([`tysan_violation`]).
    ///
    /// The new output is read in bounded chunks and the scan stops at the
    /// first report of the class, so the parent never holds more than a few
    /// megabytes of it (E7h review 2, Low 3: one input's symbolized reports
    /// ran to 112 MB, and the `tysan` arm's parent, which links
    /// `TypeSanitizer`'s runtime too, reached 18 GB reading them whole).
    fn sanitizer_report(&mut self) -> Option<(String, String)> {
        use std::io::{Seek, SeekFrom};
        let len = std::fs::metadata(&self.stderr).map_or(0, |m| m.len());
        if len <= self.stderr_seen {
            return None;
        }
        let mut file = std::fs::File::open(&self.stderr).ok()?;
        file.seek(SeekFrom::Start(self.stderr_seen)).ok()?;
        let mut left = len - self.stderr_seen;
        self.stderr_seen = len;
        // What is kept between chunks: the text from the last report's
        // start, which may continue in the next chunk.
        let mut carry = String::new();
        let mut chunk = vec![0; STDERR_CHUNK];
        while left > 0 {
            let want = usize::try_from(left.min(STDERR_CHUNK as u64)).ok()?;
            let got = file.read(&mut chunk[..want]).ok()?;
            if got == 0 {
                break;
            }
            left -= got as u64;
            carry.push_str(&String::from_utf8_lossy(&chunk[..got]));
            // Judge every report that is complete (another starts after it,
            // or the output has ended).
            let last_start = carry.rfind(TYSAN_MARKER).unwrap_or(0);
            let judged = if left == 0 { carry.len() } else { last_start };
            if let Some(report) = tysan_violation(&carry[..judged]) {
                let report = clip(report, STDERR_KEPT);
                return Some((crash_signature(&report, "tysan report"), report));
            }
            carry.drain(..judged);
            if carry.len() > STDERR_KEPT {
                // One report larger than anything kept: judge what is there.
                let report = clip(&carry, STDERR_KEPT);
                carry.clear();
                if let Some(r) = tysan_violation(&report) {
                    let r = r.to_owned();
                    return Some((crash_signature(&r, "tysan report"), r));
                }
            }
        }
        None
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
        let log = tail(&self.stderr, STDERR_KEPT as u64);
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

/// How a `TypeSanitizer` report begins.
const TYSAN_MARKER: &str = "ERROR: TypeSanitizer: ";

/// A worker's stderr is read this much at a time.
const STDERR_CHUNK: usize = 1 << 20;

/// The most of a worker's stderr a finding keeps: a crash's last 64 KB, a
/// report's first.
const STDERR_KEPT: usize = 64 * 1024;

/// The last `bytes` of the file at `path`, read without the rest of it.
fn tail(path: &Path, bytes: u64) -> String {
    use std::io::{Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(bytes)));
    let mut buf = Vec::new();
    let _ = file.take(bytes).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// `text` cut to at most `bytes`, on a character boundary.
fn clip(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
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

/// The first `TypeSanitizer` report in `log` whose access type differs from
/// the type of the object it reads or writes, as a slice from its `ERROR`
/// line to the next report. The aliasing class E7g found is exactly that
/// (`p1 int` over `any pointer`: tree-sitter-haskell's `array_push` reading
/// back as `int *` the pointer its grow wrote as `void *`; bash's and html's
/// the same through `char *`). A report of one type over itself (`int` over
/// `int`, `in <struct A> at offset a` against `in <struct B> at offset b`)
/// is an `int` member read through a different enclosing struct than the
/// one it was written through: the effective type is the same, which C
/// permits, and tree-sitter's own lexer does it on every token
/// (`ts_lexer_finish`, `ts_parser__lex`: hundreds of thousands a run).
fn tysan_violation(log: &str) -> Option<&str> {
    const MARKER: &str = TYSAN_MARKER;
    let mut starts: Vec<usize> = log.match_indices(MARKER).map(|(i, _)| i).collect();
    starts.push(log.len());
    starts.windows(2).map(|w| &log[w[0]..w[1]]).find(|report| {
        let access = report.split_once(" with type ").map(|(_, r)| r);
        let object = report
            .split_once(" accesses an existing object of type ")
            .map(|(_, r)| r);
        let ty = |s: &str, stops: &[&str]| {
            let end = stops
                .iter()
                .filter_map(|p| s.find(p))
                .min()
                .unwrap_or(s.len());
            s[..end].trim().to_owned()
        };
        match (access, object) {
            (Some(a), Some(o)) => ty(a, &[" (in ", " accesses"]) != ty(o, &[" (in ", "\n"]),
            _ => true,
        }
    })
}

/// A short, stable name for a crash: the sanitizer's error and first
/// frame, glibc's complaint, the panic site, or the signal.
fn crash_signature(log: &str, how: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    // AddressSanitizer, and clang's TypeSanitizer (the `tysan` arm):
    // `==N==ERROR: <Sanitizer>: <what> on address …`, then a `#0` frame.
    for (tool, short) in [("AddressSanitizer", "asan"), ("TypeSanitizer", "tysan")] {
        let marker = format!("ERROR: {tool}: ");
        let Some(i) = lines.iter().position(|l| l.contains(&marker)) else {
            continue;
        };
        let what = lines[i]
            .split(&marker)
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .unwrap_or("error");
        let frame = lines[i..]
            .iter()
            .find_map(|l| l.trim_start().strip_prefix("#0 "))
            .and_then(|f| f.split(" in ").nth(1))
            .and_then(|f| f.split_whitespace().next())
            .map_or("?", |f| f.split('.').next().unwrap_or(f));
        return format!("{short} {what} in {frame}");
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
    /// The findings the owner has accepted (`--accepted`, `fuzz/accepted.tsv`).
    accepted: Vec<Accepted>,
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
    /// The issue of the accepted-list entry this finding matches, if any:
    /// reported "known, accepted (#N)", and it does not fail the run.
    accepted: Option<u32>,
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
        accepted: match flags.one("accepted") {
            Some(path) => load_accepted(Path::new(path))?,
            None => Vec::new(),
        },
    });
    for l in &cfg.long {
        entry(l)?;
    }
    let jobs = usize::try_from(flags.num("jobs", 4)?.max(1)).map_err(|e| e.to_string())?;
    let policy = match flags.one("fail-on").unwrap_or("all") {
        "all" => FailOn::All,
        "crashes" => FailOn::Crashes,
        other => return Err(format!("--fail-on {other}: `all` or `crashes`")),
    };
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
    let failed = fails(&reports, policy);
    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Which findings fail a run. `scripts/fuzz-grammars` passes `crashes` for
/// its `tysan` arm (E7h fix round 1): that build exists to see aliasing
/// violations, and its time and memory are `TypeSanitizer`'s (shadow memory
/// alone is several times a parse's), so its hangs and allocations are
/// reported and the `ubsan` arm rules on those classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailOn {
    All,
    Crashes,
}

/// Whether the run fails: a grammar could not be fuzzed; a hang came back
/// (a parse that never returned, E7h.3); a parse was cut alone at the
/// confirmation's memory cap (E7h fix round 1); or a crash was seen at all. A
/// crash that came back alone or after its worker's own history fails the
/// run, and one that came back neither way fails it too (the workers are
/// single-threaded and every input they ran is replayed, so what does not
/// come back is not load), except a kill by signal 9, which is the host
/// reclaiming memory. Slow and large parses are reported, D36.
fn fails(reports: &[Report], policy: FailOn) -> bool {
    reports.iter().any(|r| {
        r.error.is_some()
            || r.findings.iter().any(|t| {
                t.accepted.is_none()
                    && match t.finding.kind {
                        Kind::Crash => t.repro != Repro::No || t.finding.signature != "signal 9",
                        Kind::Hang | Kind::Memory => policy == FailOn::All && t.repro != Repro::No,
                        Kind::Slow | Kind::Alloc => false,
                    }
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
        } else if let Outcome::Done { parses, micros, .. } = outcome {
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
    report.findings = findings
        .into_iter()
        .enumerate()
        .map(|(i, f)| triage(lang.name, f, &dir, i, &cfg.limits, &cfg.accepted))
        .collect();
    // After triage, as the report says: confirming, timing and minimizing
    // findings is where a grammar's minutes go (E7h: cmake's three slow
    // findings, each timed twice under twelve times the limit).
    report.stats.elapsed = started.elapsed();
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

/// How an input run alone in a fresh worker under a confirmation's limits
/// came out.
enum Alone {
    /// It returned: in this many microseconds, at this peak RSS in kB.
    Returned { micros: u64, peak_kb: u64 },
    /// It passed the memory limit and was cut there: the worker's RSS in kB
    /// at the cut (its high-water mark then, which is the cap and one
    /// poll's growth, not the parse's own peak), and how long it had run.
    OverMemory { cut_kb: u64, after: Duration },
    /// It ran out of time without returning.
    NotReturned,
    /// Its worker died under the confirmation, with this signature and
    /// detail: a crash, which is filed as one (E7h review 2, Low 4; it had
    /// been filed as a hang that "never returned").
    Crashed { signature: String, detail: String },
    /// Its worker did not start.
    NoWorker,
}

impl Alone {
    /// What the input did, as a note says it.
    fn said(&self, long: &Limits) -> String {
        match self {
            Self::Returned { micros, peak_kb } => format!(
                "returned in {} ms at a peak RSS of {}",
                micros / 1000,
                size(*peak_kb)
            ),
            Self::OverMemory { cut_kb, after } => format!(
                "passed the {} cap and was cut there at {} after {:.1} s",
                size(long.rss_kb),
                size(*cut_kb),
                after.as_secs_f64()
            ),
            Self::NotReturned => format!(
                "did not return in {} s under {}",
                long.hang.as_secs(),
                size(long.rss_kb)
            ),
            Self::Crashed { signature, .. } => format!("crashed: {signature}"),
            Self::NoWorker => "could not be run: its worker did not start".to_owned(),
        }
    }
}

fn time_alone(
    grammar: &str,
    text: &str,
    edits: u32,
    seed: u64,
    dir: &Path,
    limits: &Limits,
) -> Alone {
    let Ok(mut w) = Worker::spawn(grammar, dir.join("triage.stderr"), limits.hang) else {
        return Alone::NoWorker;
    };
    match w.run(text, edits, seed, limits) {
        Outcome::Done {
            micros, peak_kb, ..
        } => Alone::Returned { micros, peak_kb },
        Outcome::Failed {
            kind: Kind::Alloc, ..
        } => w
            .memory_cut
            .map_or(Alone::NotReturned, |(cut_kb, after)| Alone::OverMemory {
                cut_kb,
                after,
            }),
        Outcome::Failed {
            kind: Kind::Crash,
            signature,
            detail,
        } => Alone::Crashed { signature, detail },
        Outcome::Failed { .. } => Alone::NotReturned,
    }
}

/// `kb` as the report states a size: whole gigabytes where it is one, a
/// tenth of one past a gigabyte, megabytes below.
fn size(kb: u64) -> String {
    const GB: u64 = 1024 * 1024;
    if kb >= GB && kb.is_multiple_of(GB) {
        format!("{} GB", kb / GB)
    } else if kb >= GB {
        #[allow(clippy::cast_precision_loss)]
        let gb = kb as f64 / GB as f64;
        format!("{gb:.1} GB")
    } else {
        format!("{} MB", kb / 1024)
    }
}

/// A finding cut alone at the confirmation's memory cap: its own kind,
/// named with the cap and the RSS it was cut at (E7h fix round 1). The RSS
/// at the cut is the cap and one poll's growth, not the parse's own peak,
/// which the harness never sees (E7h review 2, Low 7).
fn over_memory(f: &mut Finding, cut_kb: u64, after: Duration, long: &Limits) {
    f.kind = Kind::Memory;
    f.signature = format!(
        "exceeded memory cap at {} (cut at {} after {:.1} s)",
        size(long.rss_kb),
        size(cut_kb),
        after.as_secs_f64()
    );
    let _ = write!(
        f.detail,
        "\nexceeded memory cap: alone it passed {} and was cut there at {} after {:.1} s, \
         inside the {} s limit; its own peak is not known, nor whether it would have \
         returned. A memory cut fails the run, since a parse that grows this far takes the \
         editor out of memory (#296), unless fuzz/accepted.tsv names it.",
        size(long.rss_kb),
        size(cut_kb),
        after.as_secs_f64(),
        long.hang.as_secs()
    );
}

/// What the input as found did alone under the confirmation's limits
/// decides an allocation's or a hang's kind (E7h review 2, High 2): a cut
/// at the memory cap is `memory`, a crash is a crash, an allocation that
/// returns is large, one that runs out of time never returned, and a hang
/// that returns is slow. Until then an allocation was minimized first and
/// only its minimum confirmed, so a parse whose memory grows with its
/// input (#296) was confirmed at the size just past the first limit, where
/// it returns, and passed.
fn classify(f: &mut Finding, found: &Alone, limits: &Limits, long: &Limits) {
    let first = f.kind;
    let _ = write!(
        f.detail,
        "\nthe input as found, alone: {}",
        found.said(long)
    );
    match found {
        Alone::OverMemory { cut_kb, after } => over_memory(f, *cut_kb, *after, long),
        Alone::Crashed { signature, detail } => {
            f.kind = Kind::Crash;
            f.signature.clone_from(signature);
            let _ = write!(
                f.detail,
                "\ncrashed under the confirmation's limits, after the first pass met its {} \
                 limit:\n{detail}",
                if first == Kind::Hang {
                    "time"
                } else {
                    "memory"
                }
            );
        }
        Alone::Returned { micros, .. } if first == Kind::Hang => {
            f.kind = Kind::Slow;
            // The limit is per parse, and an input is several parses (its
            // edits'), so the whole can take longer than one may (E7h review
            // 1, Low 7: "returned alone in 142425 ms under a 120 s limit").
            let _ = write!(
                f.detail,
                "\nslow, not hung: alone, no parse ran past the {} s limit; the input's \
                 parses took {} ms in all (not minimized)",
                long.hang.as_secs(),
                micros / 1000
            );
        }
        Alone::Returned { .. } => {
            let _ = write!(
                f.detail,
                "\nlarge, and it returns: under {} and {} s",
                size(long.rss_kb),
                long.hang.as_secs()
            );
        }
        Alone::NotReturned if first == Kind::Alloc => {
            f.kind = Kind::Hang;
            f.signature = format!("never returned, growing past {} MB", limits.rss_kb / 1024);
            let _ = write!(
                f.detail,
                "\nnot large but hung: alone it had not returned in {} s, under {}",
                long.hang.as_secs(),
                size(long.rss_kb)
            );
        }
        Alone::NotReturned | Alone::NoWorker => {}
    }
}

/// A hang is re-run alone under this multiple of the hang limit; what
/// returns is slow, what does not is a hang.
const HANG_CONFIRM_FACTOR: u32 = 12;

/// A hang whose minimal input returns is run alone once more under this
/// multiple of the confirmation's time limit: what returns is slow, what
/// does not is a hang (the owner's ruling at E7h's fix round 3).
const HANG_EXTENSION_FACTOR: u32 = 10;

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

/// The time limit is a trigger, not a verdict (the owner's ruling at
/// E7h's fix round 3). An input as found that ran past twelve times
/// the limit, but whose minimum returns, may be a parse quadratic in
/// its bytes (`cmake` on 247 KB of whitespace: 210 s natively, 4 s at
/// 34 KB) or one that never terminates (#301's nested openers,
/// exponential in their depth). So it is run alone once more under
/// ten times that limit: one that returns is slow, filed with its
/// time; one that does not is a hang and fails the run. Before, the
/// minimum alone decided, which filed an exponential slow, and from
/// `4c521c7` until this round it did so whichever limit the first
/// parse met. A memory cut or a crash in that run is filed as one.
fn extend_hang(grammar: &str, f: &mut Finding, dir: &Path, limits: &Limits, long: &Limits) {
    let extended = Limits {
        hang: long.hang * HANG_EXTENSION_FACTOR,
        rss_kb: long.rss_kb,
    };
    let again = time_alone(grammar, &f.input, f.edits, f.seed, dir, &extended);
    let _ = write!(
        f.detail,
        "\nthe input as found, alone again under ten times that limit: {}",
        again.said(&extended)
    );
    match again {
        Alone::Returned { micros, .. } => {
            f.kind = Kind::Slow;
            // A first parse that met the memory limit was named a
            // hang by `classify`; slow, it carries the time limit's
            // name.
            f.signature = format!("one parse over {} ms", limits.hang.as_millis());
            let _ = write!(
                f.detail,
                "\nslow, not hung: alone a parse of it ran past the {} s limit, and run again \
                     no parse ran past {} s; its parses took {} ms in all (the limits are per \
                     parse, and an input with edits is several)",
                long.hang.as_secs(),
                extended.hang.as_secs(),
                micros / 1000
            );
        }
        Alone::OverMemory { cut_kb, after } => {
            over_memory(f, cut_kb, after, &extended);
        }
        Alone::Crashed { signature, detail } => {
            f.kind = Kind::Crash;
            f.signature = signature;
            let _ = write!(f.detail, "\ncrashed under the extended limit:\n{detail}");
        }
        Alone::NotReturned => {
            let _ = write!(
                f.detail,
                "\nhung: alone a parse of it did not return under ten times the {} s limit, \
                     though its minimum's do; a parse that does not terminate on its input \
                     fails the run",
                long.hang.as_secs()
            );
        }
        Alone::NoWorker => {}
    }
}

fn triage(
    grammar: &str,
    mut f: Finding,
    dir: &Path,
    index: usize,
    limits: &Limits,
    accepted: &[Accepted],
) -> Triaged {
    let want = (f.kind, f.signature.clone());
    let seed = f.seed;
    let same = |text: &str, edits: u32| {
        outcome_alone(grammar, text, edits, seed, dir, limits)
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
    // E7h review 2, High 2: an allocation or a hang is confirmed alone on
    // the input that showed it, under twelve times the time and four times
    // the memory, and what that input did decides its kind (`classify`).
    // Then it is minimized against the first pass's limits, and the minimum
    // is confirmed the same way and reported beside it. A hang is still
    // timed before it is minimized, as since E7h.4: one that returns is
    // slow, which D36 files and never fails, and keeps its input rather
    // than spending the minimizer's five minutes at a hang limit an
    // attempt (the depth operators make Lua's error recovery slow on most
    // runs).
    let confirmed = alone && matches!(f.kind, Kind::Hang | Kind::Alloc);
    if confirmed {
        let found = time_alone(grammar, &f.input, f.edits, f.seed, dir, &long);
        classify(&mut f, &found, limits, &long);
    }
    let (minimal, minimal_edits) = if !alone || f.kind == Kind::Slow {
        (f.input.clone(), f.edits)
    } else if same(&f.input, 0) {
        (minimize(&f.input, |t| same(t, 0)), 0)
    } else {
        (minimize(&f.input, |t| same(t, f.edits)), f.edits)
    };
    if confirmed && f.kind != Kind::Slow && minimal != f.input {
        let least = time_alone(grammar, &minimal, minimal_edits, f.seed, dir, &long);
        let _ = write!(
            f.detail,
            "\nits minimum ({} bytes), alone: {}",
            minimal.len(),
            least.said(&long)
        );
        if f.kind == Kind::Hang && matches!(least, Alone::Returned { .. }) {
            extend_hang(grammar, &mut f, dir, limits, &long);
        }
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
    let entry = if repro == Repro::No {
        None
    } else {
        accepted_entry(accepted, grammar, f.kind, &minimal)
    };
    let known = entry.map_or(String::new(), |e| {
        format!(
            "accepted: known, accepted (#{}); it does not fail the run until {}\n",
            e.issue, e.removal
        )
    });
    let note = format!(
        "grammar: {grammar}\nkind: {}\nsignature: {}\nreproduced: {reproduced}\n{known}\
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
        accepted: entry.map(|e| e.issue),
    }
}

/// Delta debugging over lines, then characters, under a budget: keep a
/// candidate only while `still` holds for it.
fn minimize(input: &str, mut still: impl FnMut(&str) -> bool) -> String {
    // Five minutes, or `PMACS_FUZZ_MINIMIZE_SECONDS` (a test's knob, beside
    // the self-test plants': a row that only needs a finding's kind need not
    // wait out the budget).
    let budget = std::env::var("PMACS_FUZZ_MINIMIZE_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let deadline = Instant::now() + Duration::from_secs(budget);
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

// -------------------------------------------------------------------
// The accepted findings (E7h review 2, High 1; the owner's ruling at
// E7h's fix round 2).
// -------------------------------------------------------------------

/// A finding the owner has accepted, from `fuzz/accepted.tsv`: one row
/// each, the issue that records it, the grammars it is reached through, the
/// kinds it is seen as, the reproductions that identify it and the condition
/// that removes it. An entry is keyed to the defect, not to the route the
/// fuzzer took to it: `markdown_inline`'s classes are reached directly and
/// through markdown's inline injection, and their rows name both (the
/// owner's ruling at E7h's fix round 3). A finding matches when it is of one
/// of those grammars and one of those kinds and its
/// minimal input is covered, at least `ACCEPT_MIN_COVERAGE` of its
/// non-whitespace characters (indentation is layout, since fix round 3),
/// by the repeated unit of one of the reproductions (`dominant_unit`): the
/// nested image openers of #301 (`![f`, `*f[`), the delimiter runs of #296
/// (`_`, `*`). A different defect in the same grammar does not match: its
/// minimum is what triggers it, not a run of those units, and minimizing
/// strips whatever of them it was found inside. What the units cannot tell
/// apart is a different defect whose minimal input is itself such a run;
/// a variant of the class with another unit is not accepted until the owner
/// adds its reproduction.
struct Accepted {
    issue: u32,
    grammars: Vec<String>,
    kinds: Vec<Kind>,
    /// The repeated unit of each reproduction, as `dominant_unit` names it.
    units: Vec<String>,
    removal: String,
}

/// How much of a finding's minimal input one of an entry's units must cover,
/// and of a reproduction its own unit, for the entry to name it.
const ACCEPT_MIN_COVERAGE: f64 = 0.5;

/// The list at `path`: tab-separated, `issue grammar kinds reproductions
/// removal`, kinds and reproductions comma-separated, a reproduction's path
/// relative to the list's directory, `#` lines and the header skipped.
fn load_accepted(path: &Path) -> Result<Vec<Accepted>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut entries = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') || line.starts_with("issue\t") {
            continue;
        }
        let at = || format!("{}:{}", path.display(), n + 1);
        let cells: Vec<&str> = line.split('\t').collect();
        let [issue, grammar, kinds, reproductions, removal] = cells[..] else {
            return Err(format!(
                "{}: five tab-separated cells, not {}",
                at(),
                cells.len()
            ));
        };
        let issue: u32 = issue
            .trim_start_matches('#')
            .parse()
            .map_err(|_| format!("{}: `{issue}` is not an issue number", at()))?;
        let grammars = grammar
            .split(',')
            .map(|g| {
                entry(g.trim())
                    .map(|e| e.name.to_owned())
                    .map_err(|e| format!("{}: {e}", at()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let kinds = kinds
            .split(',')
            .map(|k| Kind::from_name(k.trim()).ok_or_else(|| format!("{}: no kind `{k}`", at())))
            .collect::<Result<Vec<_>, _>>()?;
        if removal.trim().is_empty() {
            return Err(format!(
                "{}: an entry names the condition that removes it",
                at()
            ));
        }
        let mut units = Vec::new();
        for r in reproductions.split(',') {
            let file = base.join(r.trim());
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("{}: {}: {e}", at(), file.display()))?;
            let (unit, covered) = dominant_unit(&text)
                .ok_or_else(|| format!("{}: {} has no repeated unit", at(), file.display()))?;
            if covered < ACCEPT_MIN_COVERAGE {
                return Err(format!(
                    "{}: {}'s unit `{unit}` covers {:.0}% of it, under {:.0}%",
                    at(),
                    file.display(),
                    covered * 100.0,
                    ACCEPT_MIN_COVERAGE * 100.0
                ));
            }
            if !units.contains(&unit) {
                units.push(unit);
            }
        }
        entries.push(Accepted {
            issue,
            grammars,
            kinds,
            units,
            removal: removal.trim().to_owned(),
        });
    }
    Ok(entries)
}

/// The entry, if any, that names a finding of `kind` in `grammar` whose
/// minimal input is `minimal`.
fn accepted_entry<'a>(
    list: &'a [Accepted],
    grammar: &str,
    kind: Kind,
    minimal: &str,
) -> Option<&'a Accepted> {
    list.iter().find(|e| {
        e.grammars.iter().any(|g| g == grammar)
            && e.kinds.contains(&kind)
            && e.units
                .iter()
                .any(|u| coverage(minimal, u) >= ACCEPT_MIN_COVERAGE)
    })
}

/// The share of `text`'s bytes that non-overlapping occurrences of `unit`,
/// in its best rotation, cover.
fn coverage(text: &str, unit: &str) -> f64 {
    // Counted over the text's non-whitespace characters: indentation and
    // line breaks are layout, the route a fuzzer took, not the defect. #296's
    // paragraph reached markdown_inline with lazy-continuation lines indented
    // hundreds of spaces, which a minimizer working by lines cannot remove,
    // and the same 3,060 underscores cost the same with them or without
    // (E7h fix round 3).
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let text = text.as_str();
    if text.is_empty() || unit.is_empty() {
        return 0.0;
    }
    let chars: Vec<char> = unit.chars().collect();
    let best = (0..chars.len())
        .map(|i| {
            let rotation: String = chars[i..].iter().chain(&chars[..i]).collect();
            text.matches(rotation.as_str()).count() * rotation.len()
        })
        .max()
        .unwrap_or(0);
    #[allow(clippy::cast_precision_loss)]
    let share = best as f64 / text.len() as f64;
    share
}

/// The run of one to four characters (no newline) that covers most of
/// `text`, reduced to its primitive root and named by its least rotation
/// (`f![` and `[f!` are both `![f`), with the share it covers.
fn dominant_unit(text: &str) -> Option<(String, f64)> {
    // Over the non-whitespace characters, as `coverage` counts.
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let text = text.as_str();
    let chars: Vec<char> = text.chars().collect();
    // Each length's most frequent window, then the one whose occurrences,
    // counted without overlap, cover most of the text: overlapping counts
    // would favour a window one longer than a short period.
    let mut best: Option<(f64, String)> = None;
    for len in 1..=4 {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for w in chars.windows(len) {
            if !w.contains(&'\n') {
                *counts.entry(w.iter().collect()).or_insert(0) += 1;
            }
        }
        let Some(top) = counts
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(u, _)| u.clone())
        else {
            continue;
        };
        let covered = coverage(text, &top);
        if best.as_ref().is_none_or(|(c, _)| covered > *c + 1e-9) {
            best = Some((covered, top));
        }
    }
    let (_, unit) = best?;
    let chars: Vec<char> = unit.chars().collect();
    let root_len = (1..=chars.len())
        .find(|d| {
            chars.len().is_multiple_of(*d) && (0..chars.len()).all(|i| chars[i] == chars[i % d])
        })
        .unwrap_or(chars.len());
    let root = &chars[..root_len];
    let canonical = (0..root.len())
        .map(|i| root[i..].iter().chain(&root[..i]).collect::<String>())
        .min()?;
    let covered = coverage(text, &canonical);
    Some((canonical, covered))
}

fn write_report(cfg: &Config, reports: &[Report]) -> Result<(), String> {
    let mut md = String::new();
    let mut tsv = String::from(
        "grammar\tseeds\tinputs\tparses\tmutated\tMB\tslowest_ms\tseconds\tgrowth_mb\tcrashes\thangs\tmemory\tslow\tallocs\tin_sequence\tunconfirmed\taccepted\terror\n",
    );
    let _ = writeln!(
        md,
        "# Grammar fuzz report\n\n{} s per grammar after the seeds, {} edits per input, seed {}, \
         hang {} ms per parse, RSS {} MB. MB is the text fed in; slowest is one input's whole exercise, kept as \
         `GRAMMAR/slowest.input` with its `slowest.seed`; \
         s is the grammar's wall time, triage included; growth is the most a worker's RSS gained over \
         its inputs before it was replaced; mutated counts the inputs after the seeds. Crashes, \
         hangs, memory, slow and allocs count findings that came back, alone or after the inputs \
         before them (in sequence counts the latter), memory those cut at four times the RSS \
         limit; unconfirmed those that came back neither way. A crash, however it came \
         back, a hang that came back and a memory cut fail the run, unless the accepted \
         list (`fuzz/accepted.tsv`, the owner's) names the finding: accepted counts those, \
         reported known, accepted (#N).\n\n\
         | grammar | seeds | inputs | mutated | parses | MB | slowest ms (bytes) | s | growth MB | crashes | hangs | memory | slow | allocs | in sequence | unconfirmed | accepted |\n\
         |---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|",
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
        let (mem, slow_n) = (count(Kind::Memory), count(Kind::Slow));
        let growth = s.growth_kb / 1024;
        let unconfirmed = r.findings.iter().filter(|t| t.repro == Repro::No).count();
        let accepted = r.findings.iter().filter(|t| t.accepted.is_some()).count();
        let secs = s.elapsed.as_secs();
        let mb = s.bytes / (1024 * 1024);
        let slow = s.slowest_micros / 1000;
        let err = r.error.as_deref().unwrap_or("");
        let _ = writeln!(
            md,
            "| {} | {} | {} | {mutated} | {} | {mb} | {slow} ({}) | {secs} | {growth} | {c} | {h} | {mem} | {slow_n} | {a} | {in_sequence} | {unconfirmed} | {accepted} |{}",
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
            "{}\t{}\t{}\t{}\t{mutated}\t{mb}\t{slow}\t{secs}\t{growth}\t{c}\t{h}\t{mem}\t{slow_n}\t{a}\t{in_sequence}\t{unconfirmed}\t{accepted}\t{err}",
            r.grammar, s.seeds, s.inputs, s.parses
        );
    }
    md.push_str(&findings_md(reports));
    std::fs::write(cfg.out.join("report.md"), md).map_err(|e| e.to_string())?;
    std::fs::write(cfg.out.join("report.tsv"), tsv).map_err(|e| e.to_string())
}

/// The report's findings, those that came back and those that did not.
fn findings_md(reports: &[Report]) -> String {
    let mut md = String::new();
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
                let known = t
                    .accepted
                    .map_or(String::new(), |n| format!(", known, accepted (#{n})"));
                let _ = writeln!(
                    md,
                    "- **{}** {} `{}`, reproduced: {}{known}, minimal {} bytes, edits {}: `{}`\n\n```\n{}\n```\n",
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
    md
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
    fn the_accepted_list_names_its_findings_and_no_other() {
        // E7h fix round 2, the owner's ruling on review 2's High 1: each row
        // names its findings by the repeated unit of its reproductions.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let list = load_accepted(&root.join("fuzz/accepted.tsv")).expect("the list loads");
        let issues: Vec<u32> = list.iter().map(|e| e.issue).collect();
        assert_eq!(issues, [296, 301]);
        let units = |issue| {
            list.iter()
                .find(|e| e.issue == issue)
                .map(|e| e.units.clone())
                .unwrap_or_default()
        };
        assert_eq!(units(296), ["_", "*"]);
        assert_eq!(units(301), ["![f", "*f["]);
        let read = |p: &str| std::fs::read_to_string(root.join(p)).expect(p);
        let named = |kind, text: &str| {
            accepted_entry(&list, "markdown_inline", kind, text).map(|e| e.issue)
        };
        // #301's three minimal inputs from review 2's runs, and #296's 4 KB
        // minimum and a run of asterisks.
        for p in [
            "tests/e7h_review2/markdown-inline-84.input",
            "tests/e7h_review2/markdown-inline-hang-596.input",
            "tests/e7h_review2/markdown-inline-hang-7672.input",
        ] {
            assert_eq!(named(Kind::Hang, &read(p)), Some(301), "{p}");
            assert_eq!(named(Kind::Memory, &read(p)), None, "{p} as a memory cut");
            // Reached through markdown's inline injection, it is the same
            // defect (E7h fix round 3); in a grammar that does not reach
            // markdown_inline it is not.
            assert_eq!(
                accepted_entry(&list, "markdown", Kind::Hang, &read(p)).map(|e| e.issue),
                Some(301),
                "{p} through markdown"
            );
            assert_eq!(
                accepted_entry(&list, "rust", Kind::Hang, &read(p)).map(|e| e.issue),
                None,
                "{p} in another grammar"
            );
        }
        let through = read("fuzz/repro/markdown-296-through-injection-20515.input");
        assert_eq!(
            accepted_entry(&list, "markdown", Kind::Memory, &through).map(|e| e.issue),
            Some(296),
            "#296 through markdown's injection, as the 600 s run met it"
        );
        assert_eq!(
            accepted_entry(&list, "markdown", Kind::Memory, SELFTEST_TRIGGER).map(|e| e.issue),
            None,
            "a different defect through markdown"
        );
        // Fix round 3's 600 s run: #296's underscores on lazy-continuation
        // lines indented hundreds of spaces (59% of the bytes), which the
        // minimizer, working by lines past 4 KB, kept. Coverage is counted
        // over the non-whitespace characters, so it is #296 by either route;
        // indentation around a planted trigger does not make it the class.
        let indented = read("fuzz/repro/markdown-inline-296-indented-7414.input");
        for g in ["markdown_inline", "markdown"] {
            assert_eq!(
                accepted_entry(&list, g, Kind::Memory, &indented).map(|e| e.issue),
                Some(296),
                "#296's indented paragraph through {g}"
            );
        }
        let padded = format!("{}{SELFTEST_TRIGGER}\n{}", " ".repeat(400), " ".repeat(400));
        assert_eq!(
            named(Kind::Memory, &padded),
            None,
            "a trigger padded with spaces"
        );
        let underscores = vec![format!("{}a `_`_", "_".repeat(582)); 7].join("\n");
        assert_eq!(named(Kind::Memory, &underscores), Some(296));
        assert_eq!(named(Kind::Memory, &"*".repeat(9000)), Some(296));
        assert_eq!(
            named(Kind::Hang, &underscores),
            None,
            "#296 is accepted as a memory cut only"
        );
        // A different defect: its minimum is its trigger, and the trigger
        // inside either class's text is not covered by its units.
        assert_eq!(named(Kind::Hang, SELFTEST_TRIGGER), None);
        assert_eq!(named(Kind::Memory, SELFTEST_TRIGGER), None);
        let inside = format!("{}{SELFTEST_TRIGGER}", "f![".repeat(4));
        assert_eq!(named(Kind::Hang, &inside), None, "{inside}");
        assert_eq!(
            dominant_unit(&"f![".repeat(20)).map(|u| u.0).as_deref(),
            Some("![f")
        );
        assert_eq!(
            dominant_unit(&"_".repeat(20)).map(|u| u.0).as_deref(),
            Some("_")
        );
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
                        accepted: None,
                    })
                    .into_iter()
                    .collect(),
                error: error.map(str::to_owned),
            };
        let fails_with = |kind, repro, signature| {
            fails(&[report(kind, Some(repro), signature, None)], FailOn::All)
        };
        assert!(
            !fails(&[report(Kind::Hang, None, "", None)], FailOn::All),
            "no finding"
        );
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
        assert!(fails(
            &[report(Kind::Hang, None, "", Some("no seeds"))],
            FailOn::All
        ));
        let crashes = |kind, signature| {
            fails(
                &[report(kind, Some(Repro::Alone), signature, None)],
                FailOn::Crashes,
            )
        };
        assert!(
            crashes(Kind::Crash, "tysan type-aliasing-violation in advance"),
            "under `--fail-on crashes` a crash still fails the run"
        );
        assert!(
            !crashes(Kind::Hang, ""),
            "and a hang, which is the sanitizer's time as much as the grammar's, is reported"
        );
        assert!(
            fails_with(Kind::Memory, Repro::Alone, "exceeded memory cap at 4 GB"),
            "a parse cut at the memory cap fails the run (the owner's ruling, E7h fix round 1)"
        );
        assert!(
            !crashes(Kind::Memory, ""),
            "except under `--fail-on crashes`, whose memory is the sanitizer's"
        );
        let mut known = report(Kind::Hang, Some(Repro::Alone), "", None);
        known.findings[0].accepted = Some(301);
        assert!(
            !fails(&[known], FailOn::All),
            "a finding the accepted list names is reported, not failed (E7h fix round 2)"
        );
        let mut known = report(Kind::Crash, Some(Repro::Alone), "asan x in y", None);
        known.findings[0].accepted = Some(1);
        assert!(!fails(&[known], FailOn::All));
        assert_eq!(size(4 * 1024 * 1024), "4 GB");
        assert_eq!(size(6_743_000), "6.4 GB");
        assert_eq!(size(256 * 1024), "256 MB");
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
        let tysan = "==7==ERROR: TypeSanitizer: type-aliasing-violation on address 0x1 \
                     (pc 0x2 bp 0x3 sp 0x4 tid 7)\n\
                     READ of size 8 at 0x1 with type p1 int accesses an existing object of \
                     type any pointer\n    #0 0x55 in advance /x/scanner.c:651:3\n";
        assert_eq!(
            crash_signature(tysan, "tysan report"),
            "tysan type-aliasing-violation in advance"
        );
        let same = "==7==ERROR: TypeSanitizer: type-aliasing-violation on address 0x9\n\
                    READ of size 4 at 0x9 with type int (in TSParser at offset 80) accesses \
                    an existing object of type int (in <anonymous type> at offset 56)\n    \
                    #0 0x56 in ts_parser__lex /x/parser.c:649:84\n\n";
        assert_eq!(tysan_violation(same), None, "int over int is not the class");
        let both = format!("{same}{tysan}");
        assert_eq!(
            tysan_violation(&both).map(|r| crash_signature(r, "tysan report")),
            Some("tysan type-aliasing-violation in advance".to_owned()),
            "the pointer over a pointer of another type, after an int over an int"
        );
        assert_eq!(
            crash_signature("corrupted size vs. prev_size\n", "signal 6"),
            "glibc: corrupted size vs. prev_size"
        );
        assert_eq!(crash_signature("", "signal 11"), "signal 11");
    }
}
