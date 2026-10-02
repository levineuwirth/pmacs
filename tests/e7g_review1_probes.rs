// tests/e7g_review1_probes.rs --- E7g review 1, the states the phase's own
// witnesses did not choose.

//! C7g unshipped four grammar crates and left a fuzz harness, a CI job and
//! a rule behind. These rows probe what that leaves a user and a later
//! session with: every extension the unshipped grammars claimed still
//! reaches its server, by filetype alone; a markdown or HTML file carrying
//! the reproductions inside a fence, a front matter block or a `<script>`
//! settles with those regions plain; the fuzz job runs when the query overlay the harness walks changes; and
//! no grammar that ships pushes through the vendored `array.h` whose
//! strict-aliasing violation GCC turned into tree-sitter-haskell's heap
//! overflow.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::{BufferIdLua, StateDir};
use pmacs::syntax::BUILTIN_LANGUAGES;

#[path = "common/iso.rs"]
mod iso;

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_owned()).eval().unwrap()
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn wait(s: &mut EditorState, secs: u64, mut pred: impl FnMut(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if pred(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7g-r1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn editor_in(dir: &Path) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(
        &s,
        &format!(
            "pmacs.project.set_search_boundary({:?})",
            dir.display().to_string()
        ),
    );
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn open(s: &EditorState, path: &Path) {
    exec(
        s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ),
    );
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The four reproductions C7g unshipped a crate for.
const TWO_PRAGMAS: &str = "{-# LANGUAGE OverloadedStrings #-}\n\
                           {-# LANGUAGE ScopedTypeVariables #-}\n";
const JS_HANG: &str = "[t,t[t\n[at\n[ ,at\n[ ,5 ];";

fn tsx_hang() -> String {
    format!("({}]", "\n".repeat(30))
}

fn nested_yaml(depth: usize) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for i in 0..depth {
        let _ = writeln!(text, "{}k{i}:", " ".repeat(i));
    }
    text
}

/// The `didOpen` notifications the fake server at `sink` received, as the
/// text each opened.
fn opened(sink: &Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(sink) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("method").and_then(|m| m.as_str()) == Some("textDocument/didOpen"))
        .filter_map(|v| Some(v.get("text")?.as_str()?.to_owned()))
        .collect()
}

#[test]
fn e7g_review1_every_extension_an_unshipped_grammar_claimed_reaches_its_server() {
    // At `0b72108` the table claimed these extensions, and `.hs`, for
    // Haskell, YAML and the JavaScript family; C7g's rows check three of
    // them (`.hs`, `.yaml`, `.js`/`.tsx` by language id) and none by a
    // server receiving the file. Each opens its own reproduction, with only
    // its language's server configured (the fake, recording what it opens),
    // so the didOpen is the filetype map's doing and nothing else's. `.hs`
    // left the list at E7h, when Haskell's grammar came back
    // (`e7h_tree_sitter_haskell_ships_from_crates_io_on_the_conforming_array_header`),
    // and `.yaml` and `.yml` with YAML's
    // (`e7h_a_yaml_file_nested_254_deep_opens_and_parses_as_yaml`).
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let cases: [(&str, &str, String); 8] = [
        ("js", "javascript", JS_HANG.to_owned()),
        ("mjs", "javascript", JS_HANG.to_owned()),
        ("cjs", "javascript", JS_HANG.to_owned()),
        ("jsx", "javascriptreact", JS_HANG.to_owned()),
        ("ts", "typescript", JS_HANG.to_owned()),
        ("mts", "typescript", JS_HANG.to_owned()),
        ("cts", "typescript", JS_HANG.to_owned()),
        ("tsx", "typescriptreact", tsx_hang()),
    ];
    let mut failures = Vec::new();
    for (ext, language, text) in cases {
        let dir = temp_dir(&format!("ft-{ext}"));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let sink = dir.join("sink.jsonl");
        let s = editor_in(&dir);
        exec(
            &s,
            &format!(
                "pmacs.lsp.config[{language:?}] = {{
                   command = {fake:?},
                   env = {{ PMACS_FAKE_LSP_CHANGE_SINK = {:?} }},
                 }}",
                sink.display().to_string()
            ),
        );
        let file = dir.join(format!("repro.{ext}"));
        std::fs::write(&file, &text).unwrap();
        open(&s, &file);
        let mut s = s;
        let reached = wait(&mut s, 10, |_| opened(&sink).contains(&text));
        let grammar: Option<String> = eval(
            &s,
            &format!("return pmacs.parse.language_for_path('repro.{ext}')"),
        );
        let attached: Option<String> = eval(
            &s,
            "local rec = pmacs.lsp.active_attachment() return rec and rec.language or nil",
        );
        let tree: Option<String> = eval(
            &s,
            "local t = pmacs.parse.tree(pmacs.window.buffer()) return t and t:language() or nil",
        );
        if !reached || grammar.is_some() || attached.as_deref() != Some(language) || tree.is_some()
        {
            failures.push(format!(
                ".{ext}: server received it {reached}, grammar {grammar:?}, \
                 attached as {attached:?} (want {language}), tree {tree:?}"
            ));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn e7g_review1_the_reproductions_inside_markdown_and_html_settle_plain() {
    // The injection route to an unshipped grammar: a fence named for it (by
    // its name or the aliases `0b72108` carried), markdown's `---` front
    // matter (which injected YAML while YAML shipped), and HTML's
    // `<script>`, each holding the input that aborted or hung it. The file
    // must settle with only grammars that ship among its layers.
    let dir = temp_dir("inject");
    let fence = |tag: &str, body: &str| format!("```{tag}\n{body}\n```\n\n");
    let mut md = format!("---\n{}---\n\n# Probe\n\n", nested_yaml(254));
    for (tag, body) in [
        ("yaml", nested_yaml(254)),
        ("yml", nested_yaml(254)),
        ("haskell", TWO_PRAGMAS.to_owned()),
        ("hs", TWO_PRAGMAS.to_owned()),
        ("javascript", JS_HANG.to_owned()),
        ("js", JS_HANG.to_owned()),
        ("jsx", JS_HANG.to_owned()),
        ("typescript", JS_HANG.to_owned()),
        ("ts", JS_HANG.to_owned()),
        ("tsx", tsx_hang()),
        ("html", format!("<script>{JS_HANG}</script>")),
        ("rust", "fn main() {}".to_owned()),
    ] {
        md.push_str(&fence(tag, &body));
    }
    let html = format!(
        "<html><body><script>{JS_HANG}</script>\n<script type=\"module\">{}</script></body></html>\n",
        tsx_hang()
    );
    let shipped: BTreeSet<&str> = BUILTIN_LANGUAGES.iter().map(|l| l.name).collect();
    for (name, text, root) in [("probe.md", md, "markdown"), ("probe.html", html, "html")] {
        let file = dir.join(name);
        std::fs::write(&file, &text).unwrap();
        let s = editor_in(&dir);
        open(&s, &file);
        let mut s = s;
        let buffer: BufferIdLua = eval(&s, "return pmacs.window.buffer()");
        let settled = wait(&mut s, 20, |s| {
            s.syntax_registry
                .view(buffer.0)
                .and_then(|h| h.current())
                .is_some()
        });
        assert!(settled, "{name}: a parse settles");
        let bundle = s
            .syntax_registry
            .view(buffer.0)
            .and_then(|h| h.current())
            .unwrap();
        let layers: Vec<String> = bundle
            .layers
            .iter()
            .map(|l| l.language_name.clone())
            .collect();
        assert_eq!(layers.first().map(String::as_str), Some(root), "{name}");
        assert!(
            layers.iter().all(|l| shipped.contains(l.as_str())),
            "{name}: every layer is a shipped grammar: {layers:?}"
        );
        if root == "markdown" {
            assert!(
                layers.iter().any(|l| l == "rust"),
                "control: a fence in a shipped language still injects: {layers:?}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e7g_review1_the_fuzz_job_runs_when_a_query_overlay_changes() {
    // `src/syntax.rs` `include_str!`s `builtin/queries/latex/highlights.scm`
    // as LaTeX's highlights query, and the harness's settle step walks it
    // (TOML's quadratic walk, #292, was a query's shape, not a parse's).
    // An edit to the overlay changes what the harness exercises and what
    // ships, and touches no path the job listens on.
    // The table moved into `pmacs-syntax` at E7i.
    let syntax = read("pmacs-syntax/src/lib.rs");
    assert!(
        syntax.contains("builtin/queries/latex/highlights.scm"),
        "control: the overlay is compiled into the table"
    );
    // Since E7h's fix round 1 the job asks `scripts/grammar-fuzz-needed`
    // rather than a workflow path filter.
    let out =
        Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/grammar-fuzz-needed"))
            .arg("--paths")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(b"builtin/queries/latex/highlights.scm\n")?;
                child.wait_with_output()
            })
            .expect("scripts/grammar-fuzz-needed runs");
    assert!(
        String::from_utf8_lossy(&out.stdout).starts_with("run=true\n"),
        "builtin/queries/** makes the grammar fuzz job fuzz: {out:?}"
    );
}

/// The host's target triple, from `cargo -vV`. `cargo metadata` takes it as
/// `--filter-platform`, so it resolves only the crates a host build uses:
/// unfiltered and offline it needs every locked crate's source, and CI's
/// runners hold only the host's (at `cbcff4b` every test leg failed here on
/// `android-activity`, which only an Android build fetches).
fn host() -> String {
    let out = Command::new(env!("CARGO"))
        .arg("-vV")
        .output()
        .expect("cargo -vV");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .expect("cargo -vV names the host")
        .to_owned()
}

/// Every crate behind a shipped grammar and the directory its source is
/// in, from `cargo metadata` against the lock.
fn grammar_crate_dirs() -> Vec<(String, PathBuf)> {
    let krates: BTreeSet<String> = read("fuzz/corpora.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty() && !l.starts_with("grammar\t"))
        .map(|l| l.split('\t').nth(1).unwrap().to_owned())
        .collect();
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--offline", "--locked"])
        .args(["--filter-platform", &host()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut dirs = Vec::new();
    for p in meta["packages"].as_array().unwrap() {
        let name = p["name"].as_str().unwrap();
        if krates.contains(name) {
            let manifest = PathBuf::from(p["manifest_path"].as_str().unwrap());
            dirs.push((name.to_owned(), manifest.parent().unwrap().to_path_buf()));
        }
    }
    assert_eq!(
        dirs.iter().map(|d| d.0.clone()).collect::<BTreeSet<_>>(),
        krates,
        "control: every grammar crate resolved"
    );
    dirs
}

fn scanners_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            scanners_under(&p, out);
        } else if p.file_name().is_some_and(|n| n == "scanner.c") {
            out.push(p);
        }
    }
}

/// The shipped scanners that still push through the pre-0.24 `array.h`.
/// E7h.1 recorded bash, haskell and html here under the owner's ruling
/// that `-fno-strict-aliasing` was the class fix; review 1 found the flag
/// reaches only builds cargo starts at the repository root (a `cargo
/// install --git` editor aborted on the two-pragma file), and at E7h's fix
/// round 1 the owner ruled the three vendored with the conforming header,
/// as python and yaml already were. So the residual is empty, and a grammar
/// on the old header can join the table only by an edit here, which is the
/// point: the flag, which `build.rs` refuses to build without since the
/// same round, is then all that stands between it and GCC 16.
const ALIASING_HEADER_RESIDUAL: &[&str] = &[];

#[test]
fn e7g_review1_every_scanner_on_the_aliasing_array_header_is_recorded_and_built_without_strict_aliasing()
 {
    // tree-sitter-haskell 0.23.1 aborted the editor because its scanner's
    // `array_push` grows the array through an `(Array *)` cast, writing
    // `contents` as `void *` and reading it back as `T *`: undefined under
    // C's aliasing rule, which GCC 16 at -O2 and -O3 compiles into a push
    // through the stale pointer (clang's TypeSanitizer reports it as a
    // type-aliasing violation; GCC 13.3, CI's, and clang happen not to
    // exploit it). The vendored `tree_sitter/array.h` that does this is
    // the pre-0.24 CLI's; grammars regenerated since assign `contents` from
    // the grow's return. Review 1 found bash, html and python shipping it.
    // E7h.1 set -fno-strict-aliasing (`.cargo/config.toml`) for every
    // compiler, and E7h's fix round 1 gave every such scanner the fixed
    // header in its vendored copy, so what is asserted is that the flag is
    // set and every scanner still on the old header is one recorded above:
    // none.
    let config = read(".cargo/config.toml");
    for var in ["HOST_CFLAGS", "TARGET_CFLAGS"] {
        assert!(
            config.lines().any(|l| l.starts_with(var)
                && l.contains("-fno-strict-aliasing")
                && l.contains("force = true")),
            "{var} forces -fno-strict-aliasing in .cargo/config.toml; without it the \
             scanners below execute the UB that took tree-sitter-haskell down"
        );
    }
    let mut exposed = Vec::new();
    for (krate, dir) in grammar_crate_dirs() {
        let mut scanners = Vec::new();
        scanners_under(&dir, &mut scanners);
        for scanner in scanners {
            let src = std::fs::read_to_string(&scanner).unwrap();
            // Every macro of the old header that writes `contents` through
            // the cast, directly or through another (E7h review 2: the row
            // named four of the thirteen; `array_reserve` and `array_delete`,
            // which bash, haskell, html and python call, were not among
            // them). `tests/e7h_review2_probes.rs` derives the thirteen from
            // the header and holds this list to them.
            let pushes = [
                "array_reserve(",
                "array_delete(",
                "array_push(",
                "array_grow_by(",
                "array_push_all(",
                "array_extend(",
                "array_splice(",
                "array_insert(",
                "array_erase(",
                "array_assign(",
                "array_swap(",
                "array_insert_sorted_with(",
                "array_insert_sorted_by(",
            ]
            .iter()
            .any(|m| src.contains(m));
            let header = scanner.parent().unwrap().join("tree_sitter/array.h");
            let aliasing = std::fs::read_to_string(&header)
                .is_ok_and(|h| h.contains("_array__grow((Array *)(self)"));
            if pushes && aliasing {
                exposed.push(format!(
                    "{krate}: {}",
                    scanner.strip_prefix(&dir).unwrap().display()
                ));
            }
        }
    }
    exposed.sort();
    assert_eq!(
        exposed, ALIASING_HEADER_RESIDUAL,
        "the shipped scanners that push through the strict-aliasing `array.h` \
         that took tree-sitter-haskell 0.23.1 down under GCC -O2 are exactly the \
         recorded residual"
    );
}

/// The tree-sitter runtime's source directory, as this workspace resolves
/// it for the host.
fn runtime_dir() -> PathBuf {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--offline", "--locked"])
        .args(["--filter-platform", &host()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "tree-sitter")
        .map(|p| {
            PathBuf::from(p["manifest_path"].as_str().unwrap())
                .parent()
                .unwrap()
                .to_path_buf()
        })
        .expect("cargo metadata resolves the tree-sitter runtime")
}

/// Every C file under `dir`.
fn c_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            c_under(&p, out);
        } else if p.extension().is_some_and(|x| x == "c" || x == "h") {
            out.push(p);
        }
    }
}

/// The grammar C files under `dir`, but the generated `parser.c`, which
/// calls no library function and is up to 42 MB.
fn grammar_c_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            grammar_c_under(&p, out);
        } else if p
            .extension()
            .is_some_and(|x| x == "c" || x == "h" || x == "cc")
            && p.file_name().is_some_and(|n| n != "parser.c")
        {
            out.push(p);
        }
    }
}

#[test]
fn e7h2_no_shipped_grammar_passes_a_codepoint_to_a_narrow_ctype_function() {
    // E7h's fix round 2: tree-sitter-md's block scanner called
    // `isdigit(lexer->lookahead)` and tree-sitter-bash's brace-range scan
    // did the same. `lookahead` is a codepoint and the narrow `<ctype.h>`
    // functions are defined only for an `unsigned char` or EOF; glibc
    // indexes a 384-entry table with the value, so `4` or `echo {` before
    // U+4A28A read about 600 KB past it into unmapped memory, and the
    // release editor died with SIGSEGV on opening a 6-byte markdown file
    // (two runs of three) and a 12-byte bash file (three of three). The
    // copies now compare against '0'..'9'. A smaller codepoint reads mapped
    // memory and returns garbage, which no sanitizer arm reports, so the
    // fuzz gate sees this class only by luck of the address; this row sees
    // the call. The wide functions (`iswspace`, `towlower`) take any
    // `wint_t` and are what a scanner should call on `lookahead`.
    //
    // It covers every shipped grammar crate (`grammar_crate_dirs`, the
    // crates `fuzz/corpora.tsv` holds to the table), not only the two that
    // were caught, and, since E7h's fix round 3, the tree-sitter runtime's
    // C, which reads the same buffer. A call whose line first bounds its
    // argument below 128 is defined and allowed: the runtime's one narrow
    // call, `isprint(chr)` after `0 < chr && chr < 128` in `subtree.c`, is
    // that.
    const NARROW: &[&str] = &[
        "isalnum", "isalpha", "isblank", "iscntrl", "isdigit", "isgraph", "islower", "isprint",
        "ispunct", "isspace", "isupper", "isxdigit", "tolower", "toupper",
    ];
    let mut calls = Vec::new();
    let mut scanned: Vec<(String, PathBuf, Vec<PathBuf>)> = grammar_crate_dirs()
        .into_iter()
        .map(|(krate, dir)| {
            let mut files = Vec::new();
            grammar_c_under(&dir, &mut files);
            (krate, dir, files)
        })
        .collect();
    let runtime = runtime_dir();
    let mut files = Vec::new();
    c_under(&runtime.join("src"), &mut files);
    assert!(
        files.iter().any(|f| f.ends_with("src/parser.c")),
        "control: the runtime's own parser.c is scanned"
    );
    scanned.push(("tree-sitter (runtime)".to_owned(), runtime, files));
    for (krate, dir, files) in scanned {
        for file in files {
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            for (n, line) in src.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for name in NARROW {
                    let called = code.match_indices(name).any(|(i, _)| {
                        let before = code[..i].chars().next_back();
                        let after = code[i + name.len()..].trim_start();
                        let bounded = code[..i].contains("< 128 &&");
                        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                            && after.starts_with('(')
                            && !bounded
                    });
                    if called {
                        calls.push(format!(
                            "{krate}: {}:{}: {}",
                            file.strip_prefix(&dir).unwrap().display(),
                            n + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        calls.is_empty(),
        "a shipped grammar or the runtime calls a narrow ctype function, which is undefined for a \
         codepoint past 255 and on glibc reads past its table (E7h fix round 2: \
         markdown's `4` and bash's `echo {{` before U+4A28A killed the editor):\n{}",
        calls.join("\n")
    );
}
