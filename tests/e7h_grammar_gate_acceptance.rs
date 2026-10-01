// tests/e7h_grammar_gate_acceptance.rs --- E7h, the grammar gate made real.

//! E7g removed four grammars that took the editor down and left a gate
//! its review found could not fail on them. These rows hold what E7h
//! puts in their place.
//!
//! E7h.1: every C source the build compiles carries
//! `-fno-strict-aliasing`, because the vendored `tree_sitter/array.h` of
//! grammars generated before tree-sitter 0.24 pushes through an
//! `(Array *)` cast, undefined behavior GCC 16 at -O2 compiled into
//! tree-sitter-haskell 0.23.1's heap overflow. The flag is set in
//! `.cargo/config.toml` and read by the `cc` crate from `HOST_CFLAGS`
//! (native) or `TARGET_CFLAGS` (cross); each grammar's build script
//! records what it read, and the second row reads that record for every
//! grammar crate that ships, so the flag is shown to reach each build and
//! not only the ones someone tested. Review 1 found the build that does not
//! start at the root: `cargo install --git` reads no `.cargo/config.toml`,
//! and its editor aborted on the two-pragma Haskell file. Since E7h's fix
//! round 1 no shipped scanner carries that header (bash, haskell and html
//! are vendored with the fixed one), and `build.rs` refuses to build when
//! the C compiler would not receive the flag; the next two rows hold the
//! refusal's verdict and, ignored for its two cold builds, the refusal
//! itself from outside the checkout.
//!
//! E7h.2: a parse is bounded in time where tree-sitter calls its progress
//! callback. `run_parse` cancels through it once `ParseRequest::deadline`
//! passes, and the editor fills the deadline from
//! `syntax.parse-deadline-ms`. Work between two calls is not bounded:
//! `ts_parser__accept` never calls it, and a markdown paragraph of
//! underscore runs spends 29.5 s and 9.8 GB there at 32 KB (#296). The
//! witness is the input E7g unshipped the JavaScript family for (24 bytes
//! on which the parser cycles through 34 states while its memory grows),
//! parsed with tree-sitter-javascript, which stays a dev-dependency; the
//! editor-level row drives the same cancellation through the key path with
//! a deadline shorter than an ordinary large file's parse.
//!
//! E7h.3: the fuzz harness can fail. Review 1 planted defects in a grammar
//! and found two it filed and passed: a parse that never returns but grows
//! past the memory limit inside the hang limit (filed as an allocation), and
//! a crash that only a sequence of inputs brings back (filed as "not
//! reproduced alone"). The rows below plant each class through the worker's
//! `PMACS_FUZZ_SELFTEST` hook, run the harness binary as CI does, and read
//! its report and exit status: crash, hang, growth and sequence fail the run,
//! and a slow parse that returns is filed, not failed (D36). Review 1's
//! big-return plant, a parse past the memory cap that returns, is a memory
//! cut since E7h's fix round 1, and fails the run too.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::{BufferIdLua, StateDir};
use pmacs::protocol::FrontendId;
use pmacs::syntax::{
    BUILTIN_LANGUAGES, ParseError, ParseRequest, ParseTreeBundle, default_injection_aliases,
    run_parse,
};

#[path = "common/iso.rs"]
mod iso;

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The crates behind the grammars that ship, from `fuzz/corpora.tsv`
/// (held to `BUILTIN_LANGUAGES` by E7g's row), and the runtime's.
fn grammar_crates() -> BTreeSet<String> {
    let mut crates: BTreeSet<String> = read("fuzz/corpora.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty() && !l.starts_with("grammar\t"))
        .map(|l| l.split('\t').nth(1).expect("a crate column").to_owned())
        .collect();
    crates.insert("tree-sitter".to_owned());
    crates
}

#[test]
fn e7h_every_c_source_is_compiled_without_strict_aliasing() {
    let config = read(".cargo/config.toml");
    let env = config
        .split("\n[env]\n")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("an [env] table in .cargo/config.toml");
    for var in ["HOST_CFLAGS", "TARGET_CFLAGS"] {
        let line = env
            .lines()
            .find(|l| l.starts_with(&format!("{var} ")))
            .unwrap_or_else(|| panic!("{var} is set in [env]"));
        assert!(
            line.contains("-fno-strict-aliasing"),
            "{var} carries -fno-strict-aliasing: {line}"
        );
        assert!(
            line.contains("force = true"),
            "{var} is forced, so an inherited value cannot drop the flag: {line}"
        );
    }
}

/// `<target>/<profile>/build`, from this test binary's own path
/// (`<target>/<profile>/deps/<name>-<hash>`).
fn build_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary's path");
    exe.parent()
        .and_then(Path::parent)
        .map(|profile| profile.join("build"))
        .expect("<target>/<profile>/build")
}

/// The newest `output` of `krate`'s build-script runs: a directory named
/// `<krate>-<16 hex digits>` holding the script's recorded stdout.
fn newest_build_output(build: &Path, krate: &str) -> Option<String> {
    let prefix = format!("{krate}-");
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(build).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(hash) = name.strip_prefix(&prefix) else {
            continue;
        };
        if hash.len() != 16 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let output = entry.path().join("output");
        let Ok(meta) = std::fs::metadata(&output) else {
            continue;
        };
        let modified = meta.modified().ok()?;
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, output));
        }
    }
    newest.and_then(|(_, p)| std::fs::read_to_string(p).ok())
}

#[test]
fn e7h_every_grammar_build_recorded_the_flag() {
    // The `cc` crate prints each variable it consults, as `NAME = Some(v)`
    // or `NAME = None`, to the build script's recorded output. A grammar
    // whose record shows neither HOST_CFLAGS nor TARGET_CFLAGS carrying the
    // flag compiled its C without it, whatever the config says.
    let build = build_dir();
    let mut failures = Vec::new();
    for krate in grammar_crates() {
        match newest_build_output(&build, &krate) {
            None => failures.push(format!(
                "{krate}: no build-script record under {}",
                build.display()
            )),
            Some(record) => {
                let carried = record.lines().any(|l| {
                    (l.starts_with("HOST_CFLAGS = Some(") || l.starts_with("TARGET_CFLAGS = Some("))
                        && l.contains("-fno-strict-aliasing")
                });
                if !carried {
                    failures.push(format!(
                        "{krate}: its C was compiled without -fno-strict-aliasing"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[path = "../build/strict_aliasing.rs"]
mod strict_aliasing;

#[test]
fn e7h_the_build_refuses_a_c_compiler_the_aliasing_flag_does_not_reach() {
    // `build.rs` hands `strict_aliasing::verdict` the arguments `cc` would
    // give the C compiler, environment flags last; the last aliasing flag
    // among them decides. E7h review 1's `cargo install --git` build is the
    // first case: `.cargo/config.toml` unread, so no flag at all.
    let v = |args: &[&str]| {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        strict_aliasing::verdict("cc", &args)
    };
    let refused = v(&["-O3", "-ffunction-sections", "-fPIC", "-m64"]).unwrap_err();
    assert!(
        refused.contains("refusing to build")
            && refused.contains("-fno-strict-aliasing")
            && refused.contains("CFLAGS=-fno-strict-aliasing cargo install"),
        "no flag: refused, naming the flag and the remedy: {refused}"
    );
    assert!(
        v(&["-O3", "-fno-strict-aliasing"]).is_ok(),
        "the config's flag"
    );
    let overridden = v(&["-O3", "-fno-strict-aliasing", "-fstrict-aliasing"]).unwrap_err();
    assert!(
        overridden.contains("a later -fstrict-aliasing overrides it"),
        "a CFLAGS_<target> -fstrict-aliasing appended after the config's flag wins, and is \
         refused: {overridden}"
    );
    assert!(
        v(&["-fstrict-aliasing", "-O2", "-fno-strict-aliasing"]).is_ok(),
        "the flag after a user's -fstrict-aliasing wins"
    );
    assert!(
        v(&["-Wno-strict-aliasing", "-fno-strict-aliasing=x"]).is_err(),
        "only the flag itself counts"
    );
}

#[test]
#[ignore = "two cold `cargo check`s of pmacs from outside the checkout (minutes); \
            E7h fix round 1's witness that build.rs refuses what review 1's \
            `cargo install --git` built"]
fn e7h_a_build_started_outside_the_checkout_refuses_without_the_flag() {
    // cargo reads `.cargo/config.toml` only from the directory a build
    // starts in and its ancestors, so a build of this manifest started from
    // a temporary directory sees no `-fno-strict-aliasing` at all, as E7h
    // review 1's `cargo install --git` did. It must fail in pmacs's build
    // script, naming the flag; and with the remedy the message names,
    // `CFLAGS=-fno-strict-aliasing`, the same build must pass.
    let dir = temp_dir("outside");
    let target = dir.join("target");
    let check = |cflags: Option<&str>| {
        let mut cmd = std::process::Command::new(env!("CARGO"));
        cmd.args([
            "check",
            "--offline",
            "-p",
            "pmacs",
            "--lib",
            "--manifest-path",
        ])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .current_dir(&dir)
        // a user config's sccache would wrap the C too, and can fail on
        // its own
        .env("RUSTC_WRAPPER", "")
        .env_remove("CARGO_TARGET_DIR")
        // this test runs under the checkout's `[env]`; the build it
        // starts must not inherit it
        .env_remove("HOST_CFLAGS")
        .env_remove("TARGET_CFLAGS")
        .env_remove("CFLAGS");
        if let Some(flags) = cflags {
            cmd.env("CFLAGS", flags);
        }
        cmd.output().expect("cargo check from outside the checkout")
    };
    let refused = check(None);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success()
            && stderr.contains("failed to run custom build command for `pmacs")
            && stderr.contains(
                "refusing to build: the C compiler would not receive \
                                -fno-strict-aliasing"
            ),
        "a build started outside the checkout is refused by pmacs's build script, \
         naming the flag:\n{stderr}"
    );
    let remedied = check(Some("-fno-strict-aliasing"));
    let stderr = String::from_utf8_lossy(&remedied.stderr);
    assert!(
        remedied.status.success(),
        "with CFLAGS=-fno-strict-aliasing, as the refusal says, the same build passes:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The 24 bytes E7g reduced tree-sitter-javascript 0.25.0's hang to: the
/// parser pins its offset at byte 23 and cycles through 34 (state, row,
/// col) points while its memory grows about 16 MB a second (E7g review 1).
const JS_HANG: &str = "[t,t[t\n[at\n[ ,at\n[ ,5 ];";

fn js_request(text: &str, deadline: Option<Duration>) -> ParseRequest {
    ParseRequest {
        source: Arc::from(text.as_bytes()),
        language: tree_sitter_javascript::LANGUAGE.into(),
        language_name: "javascript".to_owned(),
        prior_tree: None,
        edits: Vec::new(),
        injection_aliases: Arc::new(default_injection_aliases()),
        deadline,
    }
}

#[test]
fn e7h_a_parse_that_never_returns_is_cancelled_at_its_deadline() {
    // Control: an ordinary line of JavaScript parses under the same bound.
    let deadline = Duration::from_millis(300);
    assert!(
        run_parse(js_request("let a = [1, 2];\n", Some(deadline))).is_ok(),
        "control: the grammar parses ordinary JavaScript inside the deadline"
    );
    let started = Instant::now();
    let err = run_parse(js_request(JS_HANG, Some(deadline))).expect_err("the hang is cancelled");
    let returned = started.elapsed();
    let ParseError::DeadlineExceeded { deadline: d, after } = err else {
        panic!("cancelled at the deadline, not {err:?}");
    };
    assert_eq!(d, deadline);
    assert!(after >= deadline, "not before the deadline: {after:?}");
    // A generous bound for the default run (D12); the under-a-millisecond
    // budget is `e7h_budget_cancellation_lands_within_a_millisecond`.
    assert!(
        returned < deadline + Duration::from_millis(250),
        "returned {returned:?} after a {deadline:?} deadline"
    );
}

#[test]
#[ignore = "wall-clock budget (D12): run by scripts/perf-budgets and CI's perf jobs"]
fn e7h_budget_cancellation_lands_within_a_millisecond() {
    // The brief's line: the 24-byte hang cancels in under a millisecond
    // once its deadline passes. Twenty runs; the worst overshoot counts.
    let deadline = Duration::from_millis(50);
    let mut worst = Duration::ZERO;
    for _ in 0..20 {
        let err = run_parse(js_request(JS_HANG, Some(deadline))).expect_err("cancelled");
        let ParseError::DeadlineExceeded { after, .. } = err else {
            panic!("{err:?}");
        };
        worst = worst.max(after.saturating_sub(deadline));
    }
    eprintln!("e7h cancellation: worst overshoot {worst:?} over 20 runs");
    assert!(
        worst < Duration::from_millis(1),
        "worst overshoot {worst:?}"
    );
}

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

fn type_char(s: &mut EditorState, ch: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

fn editor_in(dir: &Path) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7h-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn e7h_ordinary_large_files_parse_inside_the_default_deadline() {
    // The largest ordinary files the tree holds, parsed cold with their
    // injection layers under the deadline the editor ships with: none may
    // trip it, and none may lose a layer to it.
    let dir = temp_dir("default");
    let s = editor_in(&dir);
    let default_ms: u64 = eval(&s, "return pmacs.config.get('syntax.parse-deadline-ms')");
    assert_eq!(default_ms, 5000, "the shipped default");
    let deadline = Some(Duration::from_millis(default_ms));
    for (lang, rel) in [
        ("rust", "pmacs-gpu/src/main.rs"),
        ("rust", "src/lua_bindings/mod.rs"),
        ("markdown", "docs/ci-red-signatures.md"),
    ] {
        let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap();
        let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == lang).unwrap();
        let req = ParseRequest {
            source: Arc::from(bytes.as_slice()),
            language: (entry.loader)(),
            language_name: lang.to_owned(),
            prior_tree: None,
            edits: Vec::new(),
            injection_aliases: Arc::new(default_injection_aliases()),
            deadline,
        };
        let started = Instant::now();
        let bundle = run_parse(req).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(
            !bundle.layers_cut_by_deadline,
            "{rel}: no layer lost to the deadline"
        );
        eprintln!(
            "{rel}: {} layers in {:?}",
            bundle.layers.len(),
            started.elapsed()
        );
    }
}

/// Every node's kind and byte range, in walk order: two trees agree on
/// this only if they are the same parse of the same bytes.
fn node_signature(tree: &tree_sitter::Tree) -> Vec<(u16, usize, usize)> {
    let mut out = Vec::new();
    let mut c = tree.walk();
    loop {
        let n = c.node();
        out.push((n.kind_id(), n.start_byte(), n.end_byte()));
        if c.goto_first_child() {
            continue;
        }
        loop {
            if c.goto_next_sibling() {
                break;
            }
            if !c.goto_parent() {
                return out;
            }
        }
    }
}

fn current_bundle(s: &EditorState, buf: BufferIdLua) -> Option<Arc<ParseTreeBundle>> {
    s.syntax_registry.view(buf.0).and_then(|h| h.current())
}

fn deadline_notices(s: &EditorState) -> usize {
    s.lua_host
        .errors_buffer_text()
        .matches("ran past syntax.parse-deadline-ms")
        .count()
}

#[test]
fn e7h_a_parse_past_its_deadline_keeps_the_tree_and_says_so_once() {
    // A 595 KB Rust file parses in hundreds of milliseconds; with the
    // deadline at 1 ms every parse of it is cancelled. Typed through the
    // key path, as a user types: the buffer keeps the tree it had, the user
    // is told once however many keystrokes follow, nothing aborts, and once
    // parses finish again the tree is the buffer's (the cancelled request
    // drained the edits, so the next parse must be cold, not incremental).
    let dir = temp_dir("keep");
    let file = dir.join("big.rs");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor.rs"),
        &file,
    )
    .unwrap();
    let mut s = editor_in(&dir);
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let buf: BufferIdLua = eval(&s, "return pmacs.window.buffer()");
    assert!(
        wait(&mut s, 60, |s| current_bundle(s, buf).is_some()),
        "the first parse installs under the default deadline"
    );
    let before = current_bundle(&s, buf).unwrap();

    exec(&s, "pmacs.config.set('syntax.parse-deadline-ms', 1)");
    type_char(&mut s, 'x');
    assert!(
        wait(&mut s, 30, |s| deadline_notices(s) == 1),
        "a cancelled parse is reported: {}",
        s.lua_host.errors_buffer_text()
    );
    assert!(
        Arc::ptr_eq(&before, &current_bundle(&s, buf).unwrap()),
        "the buffer keeps the tree it had"
    );
    for ch in "yzw".chars() {
        type_char(&mut s, ch);
        wait(&mut s, 1, |_| false);
    }
    wait(&mut s, 3, |_| false);
    assert_eq!(deadline_notices(&s), 1, "told once, not per keystroke");
    assert!(
        Arc::ptr_eq(&before, &current_bundle(&s, buf).unwrap()),
        "still the tree it had"
    );

    // Parses finish again: the next is cold, and its tree is the buffer's.
    exec(&s, "pmacs.config.set('syntax.parse-deadline-ms', 0)");
    type_char(&mut s, 'v');
    assert!(
        wait(&mut s, 60, |s| !Arc::ptr_eq(
            &before,
            &current_bundle(s, buf).unwrap()
        )),
        "a parse installs once the deadline allows it"
    );
    assert!(
        wait(&mut s, 60, |s| {
            let text: String = eval(
                s,
                "local b = pmacs.window.buffer() return b:slice(0, b:len())",
            );
            current_bundle(s, buf).is_some_and(|b| b.source.as_ref() == text.as_bytes())
        }),
        "the installed tree was parsed from the buffer's text"
    );
    let installed = current_bundle(&s, buf).unwrap();
    let cold = run_parse(ParseRequest {
        source: installed.source.clone(),
        language: tree_sitter_rust::LANGUAGE.into(),
        language_name: "rust".to_owned(),
        prior_tree: None,
        edits: Vec::new(),
        injection_aliases: Arc::new(default_injection_aliases()),
        deadline: None,
    })
    .unwrap();
    assert_eq!(
        node_signature(installed.root_tree()),
        node_signature(cold.root_tree()),
        "the tree after the cancellations is a cold parse of the same text, node for node"
    );

    // The notice re-arms once a parse has installed.
    exec(&s, "pmacs.config.set('syntax.parse-deadline-ms', 1)");
    type_char(&mut s, 'u');
    assert!(
        wait(&mut s, 30, |s| deadline_notices(s) == 2),
        "a later cancellation is reported again: {}",
        s.lua_host.errors_buffer_text()
    );
}

#[test]
fn e7h_the_fuzz_script_builds_an_arm_that_can_see_aliasing_ub_by_default() {
    // E7h review 1, High 2: UBSan has no aliasing check, and its
    // instrumentation keeps GCC 16 from exploiting aliasing UB, so the ASan
    // and UBSan build E7h.4 made passed tree-sitter-haskell 0.23.1 without
    // -fno-strict-aliasing where ASan alone found six crashes. The script's
    // default run is three builds: that one, ASan alone with strict
    // aliasing restored after the config's flag, and clang's TypeSanitizer.
    // CI ran the first alone until E7h's fix round 2; review 2 found GCC
    // 13.3, CI's, forwards a stored length through a narrower lvalue under
    // ASan, so CI now runs the first two as a matrix whose `ubsan` leg keeps
    // the job's name, on a pinned image that prints its compiler.
    let script = read("scripts/fuzz-grammars");
    assert!(
        script.contains("[ -n \"$arms\" ] || arms=\"ubsan asan-strict tysan\""),
        "the default run builds all three arms"
    );
    let line = |needle: &str| {
        script
            .lines()
            .find(|l| l.trim_start().starts_with(needle))
            .unwrap_or_else(|| panic!("a line starting {needle}"))
            .to_owned()
    };
    let strict = line("sanitize=\"-fsanitize=address -fno-omit-frame-pointer");
    assert!(
        strict.ends_with("-fstrict-aliasing\"") && !strict.contains("undefined"),
        "asan-strict is ASan alone, strict aliasing appended last: {strict}"
    );
    assert!(
        line("sanitize=\"-fsanitize=type").ends_with("-fsanitize=type\""),
        "tysan is clang's TypeSanitizer"
    );
    assert!(
        script.contains("[ \"$arm\" = tysan ] && fail_on=crashes"),
        "only the tysan arm's crashes fail it"
    );
    let wf = read(".github/workflows/grammar-fuzz.yml");
    assert!(
        wf.contains("scripts/fuzz-grammars\n          --arm \"${{ matrix.arm }}\"\n")
            && wf.contains("        arm: [ubsan, asan-strict]\n"),
        "CI builds the ubsan and asan-strict arms, one leg each"
    );
    assert!(
        wf.contains(
            "    name: ${{ matrix.arm == 'ubsan' && 'Grammar fuzz' || \
             format('Grammar fuzz ({0})', matrix.arm) }}\n"
        ),
        "the ubsan leg keeps the check's name, `Grammar fuzz`"
    );
    assert!(
        wf.contains("    runs-on: ubuntu-24.04\n") && wf.contains("cc --version"),
        "the image is pinned and the job prints its compiler"
    );
}

/// One harness run over `lua` with `seeds` as its whole corpus (no
/// mutation), a planted `mode`, a 1 s hang limit and a 64 MB memory limit:
/// its exit code, its report.tsv row for lua, and its report.md.
fn planted_run(tag: &str, mode: &str, seeds: &[(&str, &str)]) -> (i32, Vec<String>, String) {
    let (code, row, md, _) = planted_run_with_notes(tag, mode, seeds);
    (code, row, md)
}

/// `planted_run`, and each finding's note (`lua/<kind>-<n>.txt`), joined.
fn planted_run_with_notes(
    tag: &str,
    mode: &str,
    seeds: &[(&str, &str)],
) -> (i32, Vec<String>, String, String) {
    fuzz_run(&Fuzz {
        tag,
        grammar: "lua",
        mode,
        seeds,
        hang_ms: 1000,
        rss_mb: 64,
        accepted: false,
        minimize_secs: None,
    })
}

/// One harness run with `seeds` as `grammar`'s whole corpus (no mutation,
/// no edits): its exit code, its report.tsv row for the grammar as
/// `column=value` cells, its report.md, and its findings' notes, joined.
struct Fuzz<'a> {
    tag: &'a str,
    grammar: &'a str,
    /// `PMACS_FUZZ_SELFTEST`, or "" for none.
    mode: &'a str,
    seeds: &'a [(&'a str, &'a str)],
    hang_ms: u64,
    rss_mb: u64,
    /// Pass this tree's `fuzz/accepted.tsv`.
    accepted: bool,
    /// The minimizer's budget, if not its default five minutes.
    minimize_secs: Option<u64>,
}

fn fuzz_run(f: &Fuzz) -> (i32, Vec<String>, String, String) {
    let Fuzz {
        tag,
        grammar,
        mode,
        seeds,
        hang_ms,
        rss_mb,
        accepted,
        minimize_secs,
    } = *f;
    let dir = temp_dir(&format!("plant-{tag}"));
    let corpus = dir.join("corpus").join(grammar);
    std::fs::create_dir_all(&corpus).unwrap();
    for (name, text) in seeds {
        std::fs::write(corpus.join(name), text).unwrap();
    }
    let out = dir.join("out");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_pmacs_grammar_fuzz"));
    cmd.args(["run", "--corpus"])
        .arg(dir.join("corpus"))
        .arg("--out")
        .arg(&out)
        .args(["--grammar", grammar, "--seconds", "0", "--jobs", "1"])
        .args([
            "--hang-ms",
            &hang_ms.to_string(),
            "--rss-mb",
            &rss_mb.to_string(),
        ])
        .args(["--edits", "0"])
        .env("PMACS_FUZZ_SELFTEST", mode)
        .env("PMACS_FUZZ_SELFTEST_MS", "2500");
    if let Some(secs) = minimize_secs {
        cmd.env("PMACS_FUZZ_MINIMIZE_SECONDS", secs.to_string());
    }
    if accepted {
        cmd.arg("--accepted")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/accepted.tsv"));
    }
    let run = cmd.output().expect("the harness runs");
    let tsv = std::fs::read_to_string(out.join("report.tsv")).unwrap_or_default();
    let header: Vec<&str> = tsv.lines().next().unwrap_or("").split('\t').collect();
    let row: Vec<String> = tsv
        .lines()
        .find(|l| l.starts_with(&format!("{grammar}\t")))
        .unwrap_or("")
        .split('\t')
        .map(str::to_owned)
        .collect();
    let named: Vec<String> = header
        .iter()
        .zip(&row)
        .map(|(h, v)| format!("{h}={v}"))
        .collect();
    let md = std::fs::read_to_string(out.join("report.md")).unwrap_or_default();
    let mut notes = String::new();
    if let Ok(entries) = std::fs::read_dir(out.join(grammar)) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths
            .iter()
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        {
            notes.push_str(&std::fs::read_to_string(p).unwrap_or_default());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    (run.status.code().unwrap_or(-1), named, md, notes)
}

fn has(row: &[String], cell: &str) -> bool {
    row.iter().any(|c| c == cell)
}

const TRIGGER: &str = "@FUZZSELFTEST@";

#[test]
fn e7h_a_planted_crash_is_found_and_fails_the_run() {
    let (code, row, md) = planted_run("crash", "crash", &[("a", "local y = 2\n"), ("b", TRIGGER)]);
    assert!(has(&row, "crashes=1"), "{row:?}\n{md}");
    assert_eq!(code, 1, "a crash fails the run: {row:?}");
}

#[test]
fn e7h_a_planted_hang_is_found_and_fails_the_run() {
    let (code, row, md) = planted_run("hang", "hang", &[("a", TRIGGER)]);
    assert!(has(&row, "hangs=1"), "{row:?}\n{md}");
    assert_eq!(code, 1, "a hang fails the run: {row:?}");
}

#[test]
fn e7h_a_parse_that_grows_without_returning_fails_the_run_and_is_no_allocation() {
    // Review 1's `grow` plant: never returns, and passes the memory limit
    // well inside the hang limit, so it is first seen as an allocation.
    // Confirmed alone under four times the memory and twelve times the
    // time, it reaches the memory cap long before the time runs out
    // (1.3 GB a second against 256 MB): since E7h's fix round 1 that is its
    // own outcome, "exceeded memory cap", and it fails the run. E7h.3 had
    // filed it a hang that "never returned", which is the time limit's to
    // say; whether this one would ever return is not known at the cut.
    let (code, row, md) = planted_run("grow", "grow", &[("a", TRIGGER)]);
    assert!(has(&row, "allocs=0"), "{row:?}\n{md}");
    // The harness reads a worker's memory from `/proc`, so only on Linux
    // does it see the growth before the hang limit does; on macOS, which has
    // no `/proc`, the hang limit catches it (PR #297's first CI run). The
    // run fails on both.
    if cfg!(target_os = "linux") {
        assert!(
            has(&row, "memory=1") && has(&row, "hangs=0"),
            "{row:?}\n{md}"
        );
        assert!(md.contains("exceeded memory cap at 256 MB (cut at"), "{md}");
    } else {
        assert!(has(&row, "hangs=1"), "{row:?}\n{md}");
    }
    assert_eq!(code, 1, "a parse that never returns fails the run: {row:?}");
}

#[test]
fn e7h_a_parse_past_the_memory_cap_that_returns_is_a_memory_cut_and_fails_the_run() {
    // Review 1's Medium 2: its big-return plant touched 6.4 GB and returned
    // after 10 s, and the harness filed it a hang that "never returned".
    // The owner's ruling at E7h's fix round 1: a parse that exceeds the cap
    // is reported as "exceeded memory cap at N" with its peak, and fails the
    // run; it is not a hang and not slow. The plant here touches 400 MB in
    // 64 MB steps and returns: past the 64 MB limit (an allocation), then
    // past the confirmation's 256 MB cap, where it is cut.
    let (code, row, md) = planted_run("bigreturn", "bigreturn", &[("a", TRIGGER)]);
    if cfg!(target_os = "linux") {
        assert!(
            has(&row, "memory=1")
                && has(&row, "hangs=0")
                && has(&row, "slow=0")
                && has(&row, "allocs=0"),
            "{row:?}\n{md}"
        );
        assert!(md.contains("exceeded memory cap at 256 MB (cut at"), "{md}");
        assert!(
            !md.contains("never returned"),
            "a memory cut is not said to have never returned:\n{md}"
        );
        assert_eq!(code, 1, "a memory cut fails the run: {row:?}");
    } else {
        // No `/proc`: the harness cannot read a worker's memory, the plant
        // returns in well under the hang limit, and nothing is found.
        assert_eq!(code, 0, "{row:?}\n{md}");
    }
}

#[test]
fn e7h_a_crash_only_a_sequence_brings_back_is_found_and_fails_the_run() {
    // The worker dies on the third trigger it sees: no input alone brings
    // it back. Replayed after the inputs its worker ran before it, it does.
    let (code, row, md) = planted_run(
        "sequence",
        "sequence",
        &[("a", TRIGGER), ("b", TRIGGER), ("c", TRIGGER)],
    );
    assert!(
        has(&row, "crashes=1") && has(&row, "in_sequence=1"),
        "{row:?}\n{md}"
    );
    assert!(
        md.contains("reproduced: after the 2 inputs before it"),
        "{md}"
    );
    assert_eq!(
        code, 1,
        "a crash that needs a sequence fails the run: {row:?}"
    );
}

#[test]
fn e7h_a_slow_parse_that_returns_is_filed_not_failed() {
    // 2.5 s against a 1 s hang limit: seen as a hang, it returns alone
    // under the twelve-times limit, so it is slow, filed, and the run
    // passes (D36).
    let (code, row, md, notes) = planted_run_with_notes("slow", "slow", &[("a", TRIGGER)]);
    assert!(has(&row, "slow=1") && has(&row, "hangs=0"), "{row:?}\n{md}");
    // E7h review 1, Low 7: the twelve-times limit is per parse and an input
    // is several parses, so the note says no parse passed it, never that the
    // input "returned in" more than the limit "under" it.
    assert!(
        notes.contains("slow, not hung: alone, no parse ran past the 12 s limit")
            && !notes.contains("returned alone in"),
        "{notes}"
    );
    assert_eq!(code, 0, "a slow parse does not fail the run: {row:?}");
}

#[test]
fn e7h_a_large_slow_input_whose_minimum_returns_is_filed_not_failed() {
    // 2.5 s for each of six triggers: the input alone runs 15 s, past the
    // twelve-times limit (12 s), while its minimal input, one trigger,
    // returns in 2.5 s. That is CMake's case (247 KB of whitespace parses
    // in 210 s natively, 34 KB in 4 s): slow in its size, not hung, so it
    // is filed and the run passes.
    let six = format!("{TRIGGER}\n").repeat(6);
    let (code, row, md) = planted_run("sized", "sized", &[("a", six.as_str())]);
    assert!(has(&row, "slow=1") && has(&row, "hangs=0"), "{row:?}\n{md}");
    assert!(
        md.contains("minimal 14 bytes"),
        "whole, it ran past the limit and was minimized to one trigger, whose \
         timing files it slow:\n{md}"
    );
    assert!(
        row.iter()
            .find_map(|c| c.strip_prefix("seconds="))
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|secs| secs >= 12),
        "the grammar's wall time counts its triage, which timed the whole \
         input for the full twelve-times limit: {row:?}"
    );
    assert_eq!(code, 0, "a large slow parse does not fail the run: {row:?}");
}

// -------------------------------------------------------------------
// E7h fix round 2: review 2's High 2 and High 1, and its Low 4.
// -------------------------------------------------------------------

#[test]
fn e7h2_a_parse_whose_memory_grows_with_its_input_is_judged_on_the_input_as_found() {
    // Review 2's High 2: an allocation was minimized against the first
    // limit before it was confirmed, so a parse whose memory grows with its
    // input was confirmed at the size just past that limit, where it
    // returns, and passed (#296's 16 KB input: 5.29 GB as found, filed
    // "large, and it returns" at a 4 KB minimum). The `scaled` plant touches
    // 100 MB a trigger: four pass the 256 MB cap, one returns under it.
    // Confirmed on the input as found it is a memory cut and fails the run;
    // its minimum, one trigger, is confirmed too and reported beside it.
    let four = format!("{TRIGGER}\n").repeat(4);
    let (code, row, md, notes) = fuzz_run(&Fuzz {
        tag: "scaled",
        grammar: "lua",
        mode: "scaled",
        seeds: &[("a", four.as_str())],
        hang_ms: 10_000,
        rss_mb: 64,
        accepted: false,
        minimize_secs: None,
    });
    if cfg!(target_os = "linux") {
        assert!(
            has(&row, "memory=1") && has(&row, "allocs=0"),
            "{row:?}\n{md}"
        );
        // The minimum's size is the minimizer's, whose oracle is a 50 ms
        // RSS poll: here one trigger, 14 bytes; CI's runners stopped at 20
        // (`c1fd1ed`, three ubuntu legs). What the row holds is that it
        // returned, and is reported beside the input as found.
        assert!(
            notes.contains("the input as found, alone: passed the 256 MB cap and was cut there")
                && notes
                    .lines()
                    .any(|l| l.starts_with("its minimum (")
                        && l.contains(" bytes), alone: returned in")),
            "both confirmations reported:\n{notes}"
        );
        assert_eq!(code, 1, "a memory cut fails the run: {row:?}");
    } else {
        // No `/proc`: no memory is read, and nothing is found.
        assert_eq!(code, 0, "{row:?}\n{md}");
    }
}

#[test]
fn e7h2_a_crash_while_an_allocation_is_confirmed_is_filed_as_a_crash() {
    // Review 2's Low 4: the confirmation mapped a crash to "did not
    // return", and an allocation whose confirmation crashed was filed as a
    // hang that "never returned, growing past 1024 MB". The `growcrash`
    // plant touches 160 MB, past the 64 MB limit, then aborts.
    let (code, row, md, notes) = fuzz_run(&Fuzz {
        tag: "growcrash",
        grammar: "lua",
        mode: "growcrash",
        seeds: &[("a", TRIGGER)],
        hang_ms: 10_000,
        rss_mb: 64,
        accepted: false,
        minimize_secs: None,
    });
    if cfg!(target_os = "linux") {
        assert!(
            has(&row, "crashes=1") && has(&row, "hangs=0"),
            "{row:?}\n{md}"
        );
        assert!(
            notes.contains("crashed under the confirmation's limits")
                && !notes.contains("never returned"),
            "{notes}"
        );
    } else {
        assert!(has(&row, "crashes=1"), "{row:?}\n{md}");
    }
    assert_eq!(code, 1, "a crash fails the run: {row:?}");
}

#[test]
fn e7h2_a_large_slow_input_that_first_meets_the_memory_limit_is_filed_slow() {
    // The time boundary whatever limit the first parse met. CI's
    // `asan-strict` leg at `38c9287` failed bash as two hangs: 235 KB inputs
    // whose first parse passed 1 GB, which as found did not return in 120 s
    // under 4 GB, and whose 28 KB minima returned in 2.6 s. The same
    // quadratic entering by the time limit is filed slow (`sized`, above).
    // `sizedalloc` holds 96 MB, past the 64 MB limit, while it sleeps 2.5 s
    // for each of six triggers: 15 s whole, past the twelve-times limit
    // (12 s), under the 256 MB cap, and 2.5 s minimized.
    let six = format!("{TRIGGER}\n").repeat(6);
    let (code, row, md, notes) =
        planted_run_with_notes("sizedalloc", "sizedalloc", &[("a", six.as_str())]);
    assert!(
        has(&row, "slow=1") && has(&row, "hangs=0") && has(&row, "allocs=0"),
        "{row:?}\n{md}\n{notes}"
    );
    if cfg!(target_os = "linux") {
        assert!(
            notes.contains("\nRSS ")
                && notes.contains("signature: one parse over 1000 ms")
                && notes.contains("the input as found, alone: did not return")
                && notes.contains("slow, not hung"),
            "its first parse met the memory limit, the input as found ran past the \
             time limit, and its minimum returned:\n{notes}"
        );
    }
    assert_eq!(code, 0, "a large slow parse does not fail the run: {row:?}");
}

/// #296's paragraph at `lines` lines of 588 bytes.
fn underscores(lines: usize) -> String {
    vec![format!("{}a `_`_", "_".repeat(582)); lines].join("\n")
}

#[test]
fn e7h2_296s_paragraph_is_known_and_accepted_and_does_not_fail_the_run() {
    // The owner's ruling at fix round 2 on review 2's High 1: a finding
    // `fuzz/accepted.tsv` names is reported "known, accepted (#N)" and does
    // not fail the run. #296's paragraph, scaled to the limits here (8 MB,
    // a 32 MB cap): confirmed on the input as found it is a memory cut, and
    // the list's #296 row, keyed to its underscores, names it.
    let paragraph = underscores(3);
    let (code, row, md, notes) = fuzz_run(&Fuzz {
        tag: "accept-296",
        grammar: "markdown_inline",
        mode: "",
        seeds: &[("a", paragraph.as_str())],
        hang_ms: 30_000,
        rss_mb: 8,
        accepted: true,
        minimize_secs: Some(30),
    });
    if cfg!(target_os = "linux") {
        assert!(
            has(&row, "memory=1") && has(&row, "accepted=1"),
            "{row:?}\n{md}\n{notes}"
        );
        assert!(md.contains("known, accepted (#296)"), "{md}");
        // The note says so once and nowhere the opposite: fix round 2's
        // first run of #296's input wrote "It fails the run" two lines below
        // "it does not fail the run".
        assert!(
            notes.contains("accepted: known, accepted (#296); it does not fail the run")
                && !notes.contains("It fails the run"),
            "{notes}"
        );
        assert_eq!(
            code, 0,
            "an accepted finding does not fail the run: {row:?}"
        );
    } else {
        assert_eq!(code, 0, "{row:?}\n{md}");
    }
}

#[test]
fn e7h2_a_different_defect_in_markdown_inline_fails_though_it_is_found_inside_an_accepted_class() {
    // The narrowness the ruling asks for: planted defects in markdown_inline,
    // each found inside the text of an accepted class, still fail. A hang
    // planted in #301's nested openers minimizes to its trigger, which no
    // accepted unit covers.
    let openers = format!("*bar**\n{}{TRIGGER}{}", "f![".repeat(12), "f*bark]");
    let (code, row, md, _) = fuzz_run(&Fuzz {
        tag: "planted-301",
        grammar: "markdown_inline",
        mode: "hang",
        seeds: &[("a", openers.as_str())],
        hang_ms: 1000,
        rss_mb: 1024,
        accepted: true,
        minimize_secs: Some(30),
    });
    assert!(
        has(&row, "hangs=1") && has(&row, "accepted=0"),
        "{row:?}\n{md}"
    );
    assert!(
        !md.contains(", known, accepted"),
        "no finding is marked accepted:\n{md}"
    );
    assert_eq!(code, 1, "a planted hang is not #301: {row:?}");
    if cfg!(target_os = "linux") {
        // And a memory cut planted in #296's underscores is not #296.
        let paragraph = format!("{}{TRIGGER}", underscores(1));
        let (code, row, md, _) = fuzz_run(&Fuzz {
            tag: "planted-296",
            grammar: "markdown_inline",
            mode: "bigreturn",
            seeds: &[("a", paragraph.as_str())],
            hang_ms: 10_000,
            rss_mb: 64,
            accepted: true,
            minimize_secs: Some(30),
        });
        assert!(
            has(&row, "memory=1") && has(&row, "accepted=0"),
            "{row:?}\n{md}"
        );
        assert_eq!(code, 1, "a planted memory cut is not #296: {row:?}");
    }
}
