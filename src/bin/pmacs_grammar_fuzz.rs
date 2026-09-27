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
//!                        [--hang-ms N] [--rss-mb N]
//! pmacs_grammar_fuzz repro GRAMMAR FILE [--edits N --seed N]
//! ```
//!
//! Each grammar is driven through a worker process (`worker GRAMMAR`,
//! internal) so an abort, a hang or a runaway allocation costs the worker
//! and not the run. The worker does per input what the editor does per
//! file: `run_parse` cold with its injection layers, the settle step
//! (`SyntaxRegistry::resolve_layer_queries`), the highlight capture walk
//! over every layer whole and over a viewport, a walk of every node; then
//! `--edits` incremental reparses of the same text under typing, deletes,
//! pastes and batched edits, each settled the same way.
//!
//! `run` exits 0 when no grammar produced a finding, 1 when one did, and
//! 2 on a usage or setup error (a grammar with no seeds is one: the rule
//! is corpus-seeded).

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pmacs::syntax::{
    BUILTIN_LANGUAGES, LanguageEntry, ParseRequest, ParseTreeBundle, SyntaxRegistry, byte_to_point,
    compute_highlight_spans_for, default_injection_aliases, run_parse,
};

/// Seeds longer than this are cut at a line boundary: the editor parses
/// whole files, but a fuzzer's time goes further on many short inputs.
const MAX_SEED_BYTES: usize = 64 * 1024;
/// A mutated input is cut back to this.
const MAX_INPUT_BYTES: usize = 256 * 1024;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        eprintln!("usage: pmacs_grammar_fuzz list|collect|run|repro|worker ...");
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
    let (pos, _) = Flags::parse(args, 1)?;
    let lang = entry(&pos[0])?;
    let session = Session::new(lang);
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
        let parses = session.exercise(text, edits as u32, seed, &mut || {
            // A heartbeat per parse: the parent's hang limit is per parse.
            let _ = writeln!(out, "P").and_then(|()| out.flush());
        });
        let micros = started.elapsed().as_micros();
        writeln!(out, "E {parses} {micros} {}", peak_rss_kb("self")).map_err(|e| e.to_string())?;
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
}

impl Session {
    fn new(lang: &'static LanguageEntry) -> Self {
        Self {
            lang,
            language: (lang.loader)(),
            registry: SyntaxRegistry::new(),
            aliases: Arc::new(default_injection_aliases()),
        }
    }

    fn parse(
        &self,
        text: &str,
        prior: Option<tree_sitter::Tree>,
        edits: Vec<tree_sitter::InputEdit>,
    ) -> ParseTreeBundle {
        let req = ParseRequest {
            source: Arc::from(text.as_bytes()),
            language: self.language.clone(),
            language_name: self.lang.name.to_owned(),
            prior_tree: prior,
            edits,
            injection_aliases: self.aliases.clone(),
        };
        // No timeout and no cancellation are set, as in the editor, so a
        // parse without a tree is itself a finding.
        run_parse(req).unwrap_or_else(|e| panic!("run_parse returned no tree: {e}"))
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
    /// `beat` after each. Returns the number of parses run.
    fn exercise(&self, text: String, edits: u32, seed: u64, beat: &mut dyn FnMut()) -> u32 {
        let mut rng = Rng(seed);
        let bundle = self.parse(&text, None, Vec::new());
        self.settle(&bundle, &mut rng);
        beat();
        let mut parses = 1;
        let mut tree = bundle.root_tree().clone();
        let mut cur = text;
        let mut left = edits;
        // Retype a window one character at a time, as a person does.
        if left > 0 && !cur.is_empty() && rng.chance(1, 4) {
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
                let b = self.parse(&cur, Some(tree), vec![edit]);
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
            let b = self.parse(&cur, Some(tree), ops);
            self.settle(&b, &mut rng);
            beat();
            tree = b.root_tree().clone();
            parses += 1;
        }
        parses
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
        match rng.below(12) {
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
    Hang,
    Alloc,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Hang => "hang",
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

struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: PathBuf,
    baseline_kb: u64,
}

impl Worker {
    fn spawn(grammar: &str, stderr: PathBuf) -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let err = std::fs::File::create(&stderr).map_err(|e| e.to_string())?;
        let mut child = Command::new(exe)
            .args(["worker", grammar])
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
        })
    }

    fn run(&mut self, text: &str, edits: u32, seed: u64, limits: &Limits) -> Outcome {
        let header = format!("{} {edits} {seed}\n", text.len());
        let sent = self
            .stdin
            .write_all(header.as_bytes())
            .and_then(|()| self.stdin.write_all(text.as_bytes()))
            .and_then(|()| self.stdin.flush());
        let mut last = Instant::now();
        let pid = self.child.id().to_string();
        loop {
            if sent.is_err() {
                return self.died();
            }
            match self.lines.recv_timeout(Duration::from_millis(50)) {
                Ok(line) if line == "P" => last = Instant::now(),
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
            if rss > self.baseline_kb + limits.rss_kb {
                self.kill();
                return Outcome::Failed {
                    kind: Kind::Alloc,
                    signature: format!("RSS over {} MB above baseline", limits.rss_kb / 1024),
                    detail: format!("RSS {rss} kB, baseline {} kB", self.baseline_kb),
                };
            }
        }
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
}

#[derive(Default)]
struct Stats {
    seeds: usize,
    inputs: u64,
    parses: u64,
    bytes: u64,
    slowest_micros: u64,
    slowest_len: usize,
}

struct Finding {
    kind: Kind,
    signature: String,
    input: String,
    edits: u32,
    seed: u64,
    detail: String,
}

struct Report {
    grammar: &'static str,
    stats: Stats,
    findings: Vec<Triaged>,
    error: Option<String>,
}

struct Triaged {
    finding: Finding,
    reproduced: bool,
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
    });
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
    let failed = reports
        .iter()
        .any(|r| r.error.is_some() || !r.findings.is_empty());
    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
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

fn fuzz_grammar(lang: &'static LanguageEntry, cfg: &Config) -> Report {
    let mut report = Report {
        grammar: lang.name,
        stats: Stats::default(),
        findings: Vec::new(),
        error: None,
    };
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
    // Every seed first, then mutations for `seconds`.
    let mut deadline = None;
    let mut next_seed = 0;
    loop {
        let input = if next_seed < seeds.len() {
            next_seed += 1;
            seeds[next_seed - 1].clone()
        } else {
            let end =
                *deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(cfg.seconds));
            if Instant::now() >= end {
                break;
            }
            mutator.mutate(&mut rng)
        };
        let seed = rng.next();
        if worker.is_none() {
            spawned += 1;
            match Worker::spawn(lang.name, dir.join(format!("worker-{spawned}.stderr"))) {
                Ok(w) => worker = Some(w),
                Err(e) => {
                    report.error = Some(e);
                    break;
                }
            }
        }
        let Some(w) = worker.as_mut() else { break };
        report.stats.inputs += 1;
        report.stats.bytes += input.len() as u64;
        match w.run(&input, cfg.edits, seed, &cfg.limits) {
            Outcome::Done { parses, micros } => {
                report.stats.parses += parses;
                if micros > report.stats.slowest_micros {
                    report.stats.slowest_micros = micros;
                    report.stats.slowest_len = input.len();
                }
            }
            Outcome::Failed {
                kind,
                signature,
                detail,
            } => {
                worker = None;
                let seen = findings.iter().filter(|f| f.signature == signature).count();
                if seen < 3 {
                    findings.push(Finding {
                        kind,
                        signature,
                        input,
                        edits: cfg.edits,
                        seed,
                        detail,
                    });
                }
            }
        }
    }
    drop(worker);
    report.findings = findings
        .into_iter()
        .enumerate()
        .map(|(i, f)| triage(lang.name, f, &dir, i, &cfg.limits))
        .collect();
    report
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
    let mut w = Worker::spawn(grammar, dir.join("triage.stderr")).ok()?;
    match w.run(text, edits, seed, limits) {
        Outcome::Done { .. } => None,
        Outcome::Failed {
            kind, signature, ..
        } => Some((kind, signature)),
    }
}

fn triage(grammar: &str, f: Finding, dir: &Path, index: usize, limits: &Limits) -> Triaged {
    let want = (f.kind, f.signature.clone());
    let same = |text: &str, edits: u32| {
        outcome_alone(grammar, text, edits, f.seed, dir, limits)
            .is_some_and(|got| got.0 == want.0 && (want.0 != Kind::Crash || got.1 == want.1))
    };
    let reproduced = same(&f.input, f.edits);
    let (minimal, minimal_edits) = match (reproduced, reproduced && same(&f.input, 0)) {
        (false, _) => (f.input.clone(), f.edits),
        (true, true) => (minimize(&f.input, |t| same(t, 0)), 0),
        (true, false) => (minimize(&f.input, |t| same(t, f.edits)), f.edits),
    };
    let stem = dir.join(format!("{}-{index}", f.kind.name()));
    let file = stem.with_extension("input");
    let _ = std::fs::write(&file, &minimal);
    let _ = std::fs::write(stem.with_extension("original"), &f.input);
    let note = format!(
        "grammar: {grammar}\nkind: {}\nsignature: {}\nreproduced alone: {reproduced}\n\
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
        reproduced,
        minimal,
        minimal_edits,
        file,
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
        "grammar\tseeds\tinputs\tparses\tMB\tslowest_ms\tcrashes\thangs\tallocs\terror\n",
    );
    let _ = writeln!(
        md,
        "# Grammar fuzz report\n\n{} s per grammar after the seeds, {} edits per input, seed {}, \
         hang {} ms per parse, RSS {} MB. MB is the text fed in; slowest is one input's whole exercise.\n\n\
         | grammar | seeds | inputs | parses | MB | slowest ms (bytes) | crashes | hangs | allocs |\n\
         |---|---|---|---|---|---|---|---|---|",
        cfg.seconds,
        cfg.edits,
        cfg.seed,
        cfg.limits.hang.as_millis(),
        cfg.limits.rss_kb / 1024
    );
    for r in reports {
        let count = |k: Kind| r.findings.iter().filter(|t| t.finding.kind == k).count();
        let s = &r.stats;
        let (c, h, a) = (count(Kind::Crash), count(Kind::Hang), count(Kind::Alloc));
        let mb = s.bytes / (1024 * 1024);
        let slow = s.slowest_micros / 1000;
        let err = r.error.as_deref().unwrap_or("");
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {mb} | {slow} ({}) | {c} | {h} | {a} |{}",
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
            "{}\t{}\t{}\t{}\t{mb}\t{slow}\t{c}\t{h}\t{a}\t{err}",
            r.grammar, s.seeds, s.inputs, s.parses
        );
    }
    let _ = writeln!(md, "\n## Findings\n");
    let mut none = true;
    for r in reports {
        for t in &r.findings {
            none = false;
            let _ = writeln!(
                md,
                "- **{}** {} `{}`, reproduced alone: {}, minimal {} bytes, edits {}: `{}`\n\n```\n{}\n```\n",
                r.grammar,
                t.finding.kind.name(),
                t.finding.signature,
                t.reproduced,
                t.minimal.len(),
                t.minimal_edits,
                t.file.display(),
                t.minimal.escape_debug()
            );
        }
    }
    if none {
        let _ = writeln!(md, "None.");
    }
    std::fs::write(cfg.out.join("report.md"), md).map_err(|e| e.to_string())?;
    std::fs::write(cfg.out.join("report.tsv"), tsv).map_err(|e| e.to_string())
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
    let parses = Session::new(lang).exercise(text, edits, flags.num("seed", 1)?, &mut || {});
    println!("{}: {parses} parses, no fault", lang.name);
    Ok(ExitCode::SUCCESS)
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
fn as_seed(bytes: Vec<u8>) -> Option<String> {
    if bytes.len() > 4 * 1024 * 1024 || bytes.contains(&0) {
        return None;
    }
    let mut s = String::from_utf8(bytes).ok()?;
    if s.len() > MAX_SEED_BYTES {
        let mut cut = MAX_SEED_BYTES;
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
            let Some(text) = std::fs::read(&full).ok().and_then(as_seed) else {
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
                let Some(code) = as_seed(code.into_bytes()) else {
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
            let parses = Session::new(lang).exercise("a (b) {c}\n".to_owned(), 3, 1, &mut || {});
            assert!(parses >= 4, "{}: {parses} parses", lang.name);
        }
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
