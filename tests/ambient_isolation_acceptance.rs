// tests/ambient_isolation_acceptance.rs --- integration tests must not
// read or write the developer's real ambient roots.

//! Acceptance for the archived test-ambient-config-isolation framing.
//!
//! # The defect
//!
//! `src/editor.rs` guards user-config loading with `#[cfg(not(test))]`.
//! `cfg(test)` is set only while compiling the crate's *own* unit tests;
//! an integration test in `tests/` links `pmacs` as an ordinary
//! dependency, so the guard is inactive for all of them. `cargo test
//! --lib` is protected, `cargo test --test <name>` is not. And config
//! loading is only the read half: `EditorState::new` materializes
//! bundled packages into `$XDG_DATA_HOME/pmacs` (else
//! `$HOME/.local/share`) **unconditionally**, outside every `cfg` guard,
//! creating directories and writing files.
//!
//! # The census (framing acceptance 1), read at `54a092e`
//!
//! Method: every occurrence was listed with its enclosing context and
//! read. A grep for the bare name over-counts — the framing's revision 1
//! reported 18 by grepping `Editor::new`, which does not even match the
//! real constructor `EditorState::new`.
//!
//! **In-process construction — 342 sites in 66 of 97 files.**
//!
//! * `EditorState::new()` — 334 textual occurrences, of which **330 are
//!   calls**. The other 4 are prose: `persistence_acceptance.rs:6` and
//!   `:76`, `m7_11_acceptance.rs:92`, `m8_2_acceptance.rs:75`. A fifth
//!   file, `m5_6_acceptance.rs:94`, names the constructor only to say it
//!   deliberately does **not** use it — the third place in the tree
//!   documenting this same `cfg(test)` gap.
//! * `EditorState::open(` — 14 textual occurrences in 3 files, of which
//!   **12 are calls** (`journey_acceptance` 7, `m4_acceptance` 4,
//!   `theme_faces_acceptance` 1). The other 2 are assertion-message
//!   strings in `journey_acceptance.rs:2030,2038`.
//! * Only 329 of the 330 `new()` calls are `let` bindings; the odd one
//!   is `m8_1_acceptance.rs:39`, a bare tail expression in a
//!   `fresh_editor()` helper. Sites, not files, are the unit.
//!
//! **Spawned `pmacs` — 14 sites in 8 files.** `grep CARGO_BIN_EXE_pmacs`
//! reports 36 sites in 26 files, but 18 of those are the *sibling*
//! binaries `CARGO_BIN_EXE_pmacs_fake_lsp` / `_fake_mcp`, which are LSP
//! and MCP stubs and not pmacs at all. Reading each occurrence:
//!
//! * `tests/common/daemon.rs:158` — the shared `--daemon` harness.
//! * `tests/common/pty.rs:111` — the shared real-PTY spawner.
//! * `m5_7_acceptance.rs` ×5, `m5_8_acceptance.rs` ×3,
//!   `m5_5_acceptance.rs` ×1, `m5_perf_acceptance.rs` ×1,
//!   `gpu_invocation_acceptance.rs` ×2 — direct `Command::new`.
//! * 4 further occurrences are **path derivations, not spawns**:
//!   `gpu_invocation_acceptance.rs:115`, `vterm_stage3_acceptance.rs:644`
//!   and `:1180`, `bottom_panel_stage2b_gpu_acceptance.rs:513` all take
//!   `CARGO_BIN_EXE_pmacs`'s *parent* to locate the `pmacs-gpu` sibling.
//!
//! **Mixed files are real: 5 files are both in-process and spawned** —
//! `vterm_stage3_acceptance` constructs an editor at `:159` and reaches
//! a daemon at `:665`. A file-level partition cannot represent them,
//! which is why the ratchet below keys on sites.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use pmacs::bootstrap::BootstrapRoots;
use pmacs::editor::EditorState;

#[path = "common/iso.rs"]
mod iso;

// ---------------------------------------------------------------------------
// Isolated construction still finishes initialization (framing §1.8)
// ---------------------------------------------------------------------------

/// **N** — isolated construction skips the ambient *reads* and still
/// flips the init-complete gate.
///
/// Config loading and `set_init_complete()` share one conditional block,
/// so the tempting fix — skip the block when roots are redirected —
/// would leave every isolated test permanently in the init phase.
/// `tests/m8_2_acceptance.rs:75` documents its dependence on
/// integration-test construction being init-complete, so that is not a
/// hypothetical.
///
/// Falsified by wrapping the block in `if roots.is_ambient()`.
#[test]
fn isolated_construction_is_init_complete() {
    let state = EditorState::new_with_roots(&iso::roots());
    assert!(
        state.lua_host.is_init_complete(),
        "isolated construction must still leave the init phase, or every \
         suite that reopens it (m8_2) breaks"
    );
    // The paired half — that the *ambient* constructor is unchanged —
    // deliberately does NOT live here. Asserting it needs an ambient
    // `EditorState::new()`, and an ambient construction in an ordinary
    // parent test reads the developer's real `init.lua` and materializes
    // packages into their real data root: the exact exposure this suite
    // exists to remove, committed by the suite itself. It lives in the
    // re-exec'd positive control below, where the roots are controlled
    // by construction.
}

/// **N** — the init phase is genuinely closed, not merely reported
/// closed.
///
/// A flag read is one bool; this asserts the *behaviour* the flag gates,
/// so a fix that sets the flag without the surrounding block having run
/// cannot pass. `pmacs.attach` is init-only and must now refuse.
#[test]
fn isolated_construction_closes_the_init_only_lua_surface() {
    let state = EditorState::new_with_roots(&iso::roots());
    let err = state
        .lua_host
        .lua()
        .load(r#"pmacs.attach { target = "local:/run/pmacs/x.sock" }"#)
        .exec()
        .expect_err("pmacs.attach must refuse after init");
    let text = err.to_string();
    assert!(
        text.contains("init"),
        "the refusal must name the init phase; got {text}"
    );
}

// ---------------------------------------------------------------------------
// Isolated construction redirects the writes (framing §1.6)
// ---------------------------------------------------------------------------

/// **N** — the bundled-package materialization lands in the redirected
/// data root.
///
/// Asserts content produced, not an invariant preserved: the package
/// tree has to actually exist under the isolated root. A test that only
/// checked "the real root was not modified" would pass on a machine
/// where the real root already held identical bytes, because
/// `write_if_changed` is content-gated.
#[test]
fn isolated_construction_materializes_into_the_redirected_data_root() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("materialize-probe");
    let _ = std::fs::remove_dir_all(&base);
    let roots = BootstrapRoots::isolated_under(&base);
    let dir = roots.bundled_runtime_dir().expect("redirected data root");
    assert!(!dir.exists(), "the probe root must start absent");

    let _state = EditorState::new_with_roots(&roots);

    let manifest = dir.join("repl").join("pmacs.toml");
    assert!(
        manifest.is_file(),
        "bundled packages must materialize under the redirected data \
         root; {} is missing",
        manifest.display()
    );
    let text = std::fs::read_to_string(&manifest).expect("read materialized manifest");
    assert!(
        text.contains("repl"),
        "the materialized manifest must be the bundled package's own; got {text:?}"
    );
}

/// **N** — `install_state_dirs` honours the redirected state root.
///
/// It runs *after* the constructor, so a constructor-only parameter
/// would leave it resolving `PMACS_STATE_HOME` / `XDG_STATE_HOME` from
/// the environment and hand an isolated session the developer's real
/// state dir.
#[test]
fn install_state_dirs_honours_the_redirected_state_root() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("state-probe");
    let roots = BootstrapRoots::isolated_under(&base);
    let state = EditorState::new_with_roots(&roots);
    state.install_state_dirs();

    let dir = state
        .lua_host
        .lua()
        .app_data_ref::<pmacs::lua_bindings::StateDir>()
        .expect("install_state_dirs must configure a state dir")
        .0
        .clone();
    assert_eq!(
        dir,
        roots.state_dir().unwrap(),
        "state dir must be redirected"
    );
    let history = state.core.borrow().minibuffer.history_dir.clone();
    assert_eq!(
        history,
        roots.history_dir(),
        "minibuffer history must be redirected too"
    );
}

// ---------------------------------------------------------------------------
// Hostile ambient environment (framing Bet 3, acceptance 7)
// ---------------------------------------------------------------------------

/// Names the isolated base for the isolated child; its presence is the
/// signal that this process *is* that child.
const ISOLATED_CHILD_BASE: &str = "PMACS_AMBIENT_ISOLATION_CHILD_BASE";
/// Presence marks the ambient positive-control child.
const AMBIENT_CONTROL_CHILD: &str = "PMACS_AMBIENT_ISOLATION_CONTROL";

/// An `init.lua` that leaves a mark an assertion can see. The real
/// developer `init.lua` that produced this lane broke
/// `compile_mode_acceptance` by redefining a command pmacs already
/// defines; a marker global is the same exposure with a cheaper failure
/// mode, and it discriminates read-vs-not-read directly.
const HOSTILE_INIT_LUA: &str = "_G.HOSTILE_INIT_RAN = true\n";

fn hostile_root(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    let config = root.join("pmacs");
    std::fs::create_dir_all(&config).expect("create hostile config dir");
    std::fs::write(config.join("init.lua"), HOSTILE_INIT_LUA).expect("write hostile init.lua");
    // Pre-seed the data root the ambient resolver would use, so a write
    // into it is visible as a *change*, not just as a new tree.
    let seeded = root
        .join("pmacs")
        .join("builtin-packages")
        .join(format!("v{}", env!("CARGO_PKG_VERSION")));
    std::fs::create_dir_all(&seeded).expect("seed hostile data root");
    std::fs::write(seeded.join("SEED"), b"pre-seeded\n").expect("write seed marker");
    root
}

/// Flat snapshot of a tree: relative path → contents (directories map to
/// the empty vector). Sorted, so comparison is order-independent.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .expect("entry is under root")
                .to_path_buf();
            if path.is_dir() {
                out.insert(rel, Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// The five storage variables plus `HOME`, all aimed at `root`. `HOME`
/// is included here — and only here — because this is the adversary: a
/// machine that leaves an XDG variable unset falls back to it, and the
/// point of a hostile environment is to leave no path out.
fn hostile_env(root: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("HOME", root.to_path_buf()),
        ("XDG_CONFIG_HOME", root.to_path_buf()),
        ("XDG_DATA_HOME", root.to_path_buf()),
        ("XDG_STATE_HOME", root.to_path_buf()),
        ("PMACS_STATE_HOME", root.to_path_buf()),
        ("XDG_CACHE_HOME", root.to_path_buf()),
    ]
}

fn run_child(test_name: &str, env: Vec<(&'static str, PathBuf)>) -> (bool, String) {
    let exe = std::env::current_exe().expect("current test binary");
    let output = Command::new(exe)
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .envs(env)
        .output()
        .unwrap_or_else(|e| panic!("re-exec `{test_name}`: {e}"));
    let mut log = String::new();
    log.push_str("--- stdout ---\n");
    log.push_str(&String::from_utf8_lossy(&output.stdout));
    log.push_str("--- stderr ---\n");
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        log.contains("1 passed") || !output.status.success(),
        "child `{test_name}` ran no test — the `--exact` filter went stale\n{log}"
    );
    (output.status.success(), log)
}

/// **Positive control.** Under the hostile environment, the *ambient*
/// constructor really is captured by it.
///
/// Without this the isolation assertion below is unfalsifiable: an
/// `init.lua` that never loads under any circumstances would satisfy
/// "the isolated editor did not load it" while proving nothing.
///
/// **This is the suite's only ambient construction, and it runs only as
/// a re-exec'd child** (the marker gates it), where the roots are
/// controlled by construction. An ambient `EditorState::new()` in an
/// ordinary parent test would read the developer's real `init.lua` and
/// write their real data root — so the one place that legitimately needs
/// the ambient constructor is also the one place where the environment
/// has already been redirected. Every ambient claim this suite makes
/// belongs here for that reason.
#[test]
fn ambient_construction_under_a_hostile_environment_is_captured_by_it() {
    if std::env::var_os(AMBIENT_CONTROL_CHILD).is_none() {
        return;
    }
    let state = EditorState::new();
    // Relocated from `isolated_construction_is_init_complete`: the
    // ambient constructor is unchanged by this lane and still finishes
    // initialization. Asserting it needs an ambient construction, which
    // is only safe here.
    assert!(
        state.lua_host.is_init_complete(),
        "the ambient constructor must still leave the init phase — the \
         roots parameter changes which directory is read, never whether \
         the block runs"
    );
    let ran: bool = state
        .lua_host
        .lua()
        .load("return _G.HOSTILE_INIT_RAN == true")
        .eval()
        .expect("read the hostile marker");
    assert!(
        ran,
        "the ambient constructor must load the hostile init.lua — if it \
         does not, the isolation assertion proves nothing"
    );
    // And the write half: the ambient constructor materializes into the
    // hostile data root.
    let dir = pmacs::builtin_packages::bundled_runtime_dir();
    assert!(
        dir.join("repl").join("pmacs.toml").is_file(),
        "the ambient constructor must write into the hostile data root; \
         {} is empty",
        dir.display()
    );
}

/// The isolated child: same hostile environment, redirected roots.
#[test]
fn isolated_construction_under_a_hostile_environment_ignores_it() {
    let Some(base) = std::env::var_os(ISOLATED_CHILD_BASE) else {
        return;
    };
    let base = PathBuf::from(base);
    let roots = BootstrapRoots::isolated_under(&base);
    let state = EditorState::new_with_roots(&roots);
    let ran: bool = state
        .lua_host
        .lua()
        .load("return _G.HOSTILE_INIT_RAN == true")
        .eval()
        .expect("read the hostile marker");
    assert!(!ran, "the hostile init.lua must not have been loaded");
    assert!(
        state.lua_host.is_init_complete(),
        "and initialization must still have finished"
    );
    // Content produced, in the right place.
    let dir = roots.bundled_runtime_dir().expect("redirected data root");
    assert!(
        dir.join("repl").join("pmacs.toml").is_file(),
        "bundled packages must land under the isolated root; {} is empty",
        dir.display()
    );
}

/// **N** — the whole of Bet 3: green under a hostile environment, and
/// the hostile root byte-identical afterwards.
///
/// Two children, because the two halves need opposite environments to be
/// meaningful: the positive control must be *captured* by its hostile
/// root (and so modifies it), while the isolated child must leave its
/// own hostile root untouched.
#[test]
fn a_hostile_ambient_environment_is_neither_read_nor_written() {
    // Half 1 — the control. Its hostile root is expected to change.
    let control = hostile_root("hostile-control");
    let before_control = snapshot(&control);
    let (ok, log) = run_child(
        "ambient_construction_under_a_hostile_environment_is_captured_by_it",
        {
            let mut env = hostile_env(&control);
            env.push((AMBIENT_CONTROL_CHILD, PathBuf::from("1")));
            env
        },
    );
    assert!(ok, "the ambient positive control must be captured\n{log}");
    let after_control = snapshot(&control);
    assert_ne!(
        before_control, after_control,
        "the control's hostile root must have been written into — if it \
         was not, this environment is not hostile and the isolated half \
         below asserts nothing"
    );

    // Half 2 — the isolated child. Its hostile root must be untouched.
    let hostile = hostile_root("hostile-isolated");
    let isolated_base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hostile-isolated-roots");
    let _ = std::fs::remove_dir_all(&isolated_base);
    let before = snapshot(&hostile);
    assert!(!before.is_empty(), "the hostile root must not be empty");
    let (ok, log) = run_child(
        "isolated_construction_under_a_hostile_environment_ignores_it",
        {
            let mut env = hostile_env(&hostile);
            env.push((ISOLATED_CHILD_BASE, isolated_base.clone()));
            env
        },
    );
    assert!(ok, "the isolated child must stay green\n{log}");
    let after = snapshot(&hostile);
    assert_eq!(
        before, after,
        "the hostile root must be byte-identical afterwards — a green \
         suite that still wrote into it has not demonstrated isolation"
    );
    // And the writes went somewhere: the isolated tree exists.
    assert!(
        isolated_base.join("data").join("pmacs").is_dir(),
        "the isolated data root must have been written instead"
    );
}

// ---------------------------------------------------------------------------
// Adoption ratchet (framing acceptance 12)
// ---------------------------------------------------------------------------

/// Files permitted to construct an editor through the **ambient** entry
/// points, **with the exact number of sites each is permitted**.
///
/// `journey_acceptance` is ambient on purpose: it is the golden-journey
/// ratchet, and its whole claim is that the production entry point
/// `pmacs FILE` calls has a caller. It is isolated by re-execing itself
/// with controlled roots instead (framing §1.10). This file is ambient
/// only inside the positive control above, which never runs except as a
/// deliberately re-exec'd child.
///
/// **The count is the point, not decoration.** A bare file-level
/// exemption is the weakest form of this ratchet: it licenses the named
/// file to grow *new* ambient sites forever. That is not hypothetical —
/// review round 1 of this PR found an ambient `EditorState::new()` in an
/// ordinary parent test of **this very file**, and the file-level
/// exemption is precisely what let it through a green ratchet. Every
/// exemption is now a census entry, so an added site fails even inside
/// an allowlisted file, and a removed one has to be recorded.
const AMBIENT_ALLOWLIST: &[(&str, usize)] = &[
    // 19 `new()` + 7 `open(` — the golden journey, every one of them
    // reached only from inside a re-exec'd child. (29 textual
    // occurrences; the scanner drops 2 assertion-message mentions and
    // the assembled `concat!` needle in the self-source check.)
    ("journey_acceptance.rs", 26),
    // Exactly one: the re-exec'd positive control.
    ("ambient_isolation_acceptance.rs", 1),
];

/// Strip comments and string-literal *contents* from Rust source, so a
/// scan counts calls rather than mentions.
///
/// Both matter here. `m5_6_acceptance.rs:94` names `EditorState::new` in
/// a comment only to say it deliberately does not call it, and
/// `journey_acceptance.rs:2030` carries it inside an assertion message.
/// Raw strings (`r#"..."#`) are pervasive in this suite, so they are
/// handled rather than hoped about.
fn strip_comments_and_strings(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        // Line comment.
        if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment (Rust's nest).
        if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && i + 1 < b.len() && b[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }
        // Raw string: r, then any number of #, then ".
        if b[i] == 'r' {
            let mut j = i + 1;
            let mut hashes = 0;
            while j < b.len() && b[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < b.len() && b[j] == '"' {
                j += 1;
                loop {
                    if j >= b.len() {
                        break;
                    }
                    if b[j] == '"' {
                        let mut k = j + 1;
                        let mut seen = 0;
                        while k < b.len() && b[k] == '#' && seen < hashes {
                            seen += 1;
                            k += 1;
                        }
                        if seen == hashes {
                            j = k;
                            break;
                        }
                    }
                    j += 1;
                }
                out.push(' ');
                i = j;
                continue;
            }
        }
        // Ordinary string.
        if b[i] == '"' {
            i += 1;
            while i < b.len() {
                if b[i] == '\\' {
                    i += 2;
                    continue;
                }
                if b[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(' ');
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Every `.rs` file under `tests/`, including `tests/common/`.
fn test_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = tests_dir();
    let push_dir = |d: &Path, out: &mut Vec<(String, String)>| {
        for entry in std::fs::read_dir(d).expect("read tests dir").flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "rs") {
                let name = path
                    .file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .into_owned();
                out.push((name, std::fs::read_to_string(&path).expect("read source")));
            }
        }
    };
    push_dir(&dir, &mut out);
    push_dir(&dir.join("common"), &mut out);
    out
}

/// **N** — a *durable* adoption ratchet, not a one-time census.
///
/// One self-spawning hostile-environment test proves the seam works; it
/// cannot notice a raw `EditorState::new()` added to a different binary
/// next month. This can. Falsified by adding an ambient constructor to
/// any non-allowlisted suite.
#[test]
fn no_test_outside_the_allowlist_constructs_an_ambient_editor() {
    let sources = test_sources();
    // A broken glob must not read as a clean tree.
    assert!(
        sources.len() > 90,
        "expected the whole tests/ corpus; found only {} files",
        sources.len()
    );
    let needles = [
        concat!("EditorState::", "new()"),
        concat!("EditorState::", "open("),
    ];
    let mut offenders: Vec<String> = Vec::new();
    let mut miscounted: Vec<String> = Vec::new();
    let mut seen_allowlisted: Vec<&str> = Vec::new();
    for (name, src) in &sources {
        let code = strip_comments_and_strings(src);
        let hits: usize = needles.iter().map(|n| code.matches(n).count()).sum();
        if hits == 0 {
            continue;
        }
        match AMBIENT_ALLOWLIST.iter().find(|(f, _)| *f == name.as_str()) {
            Some((file, allowed)) => {
                seen_allowlisted.push(file);
                // An allowlisted file is exempted for the sites it was
                // reviewed with, not for any it grows later.
                if hits != *allowed {
                    miscounted.push(format!("{name}: {hits} site(s), allowlist says {allowed}"));
                }
            }
            None => offenders.push(format!("{name} ({hits} site(s))")),
        }
    }
    offenders.sort();
    miscounted.sort();
    assert!(
        offenders.is_empty(),
        "these suites construct an editor through the ambient entry \
         points, so they read the developer's real init.lua and write \
         into their real data root: {offenders:?}\n\
         Use `EditorState::new_with_roots(&crate::iso::roots())` (see \
         tests/common/iso.rs), or add the file to AMBIENT_ALLOWLIST with \
         a reason.",
    );
    assert!(
        miscounted.is_empty(),
        "allowlisted files whose ambient site count moved: {miscounted:?}\n\
         MORE than allowed means a new ambient construction slipped into \
         an exempted file — the failure mode a bare file-level exemption \
         cannot see. FEWER means the census is stale; update the count.",
    );
    // Dead allowlist entries are how a ratchet rots: an entry that no
    // longer needs to be there silently licenses a future regression.
    let mut missing: Vec<&str> = AMBIENT_ALLOWLIST
        .iter()
        .map(|(f, _)| *f)
        .filter(|f| !seen_allowlisted.contains(f))
        .collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "allowlisted files that no longer construct an ambient editor — \
         remove them: {missing:?}"
    );
}

/// **N** — the ratchet's scanner is not fooled by prose.
///
/// A grep-shaped answer is what cost this lane a review round; the scan
/// above only ratchets if it distinguishes a call from a mention. Pinned
/// with the exact shapes the corpus actually contains.
#[test]
fn the_ratchet_scanner_counts_calls_not_mentions() {
    let sample = r##"
//! `EditorState::new()` in a doc comment.
// `EditorState::new()` in a line comment.
/* `EditorState::new()` in a block comment. */
fn f() {
    let msg = "EditorState::open(file) must not greet";
    let raw = r#"EditorState::new() inside a raw string"#;
    let _ = EditorState::new();
}
"##;
    let code = strip_comments_and_strings(sample);
    assert_eq!(
        code.matches(concat!("EditorState::", "new()")).count(),
        1,
        "only the call survives; got {code:?}"
    );
    assert_eq!(
        code.matches(concat!("EditorState::", "open(")).count(),
        0,
        "the assertion-message mention must not count; got {code:?}"
    );
}

/// **N** — the seam actually reached the corpus.
///
/// The ratchet above is an absence check, and an absence check passes on
/// a tree where nobody constructs an editor at all. This asserts the
/// positive: the isolated constructor has broad adoption.
#[test]
fn the_isolated_constructor_has_been_adopted_across_the_corpus() {
    let sources = test_sources();
    let adopters: Vec<&String> = sources
        .iter()
        .filter(|(_, src)| {
            let code = strip_comments_and_strings(src);
            code.contains(concat!("EditorState::new", "_with_roots("))
                || code.contains(concat!("EditorState::open", "_with_roots("))
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        adopters.len() >= 60,
        "expected the whole in-process population to have migrated; only \
         {} files did",
        adopters.len()
    );
}
