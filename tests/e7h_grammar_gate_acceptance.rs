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
//! not only the ones someone tested.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
