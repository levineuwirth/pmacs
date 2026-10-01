// tests/e7h_review1_probes.rs --- E7h review 1, the states the phase's own
// witnesses did not choose.

//! E7h restored tree-sitter-haskell 0.23.1 on one condition: that its C is
//! compiled with `-fno-strict-aliasing`, which `.cargo/config.toml` forces
//! into every `cc` compile. Cargo reads that file only from the directory it
//! is started in and that directory's ancestors (and `cargo install --git`
//! never reads a package's own config at all), so the flag is a property of
//! where a build is started, not of the source that needs it. Review 1 built
//! the editor the way `docs/package-author-guide.md` installs `pmacs-audit`,
//! `cargo install --git <repo> --rev 0a85d18 --bin pmacs pmacs`, on this
//! laptop's GCC 16.2.1: every grammar's build record read
//! `HOST_CFLAGS = None`, and the editor aborted (`corrupted size vs.
//! prev_size`, SIGABRT) on the first keystroke into the owner's two-pragma
//! file in seven of nine runs, where the release pair built from the root
//! never did. The row below is that probe as a witness at the seam that
//! decides it: a build of pmacs started outside the repository.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
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

/// Every grammar crate `fuzz/corpora.tsv` names, with its source directory
/// as this workspace resolves it (a vendored copy where one is patched in).
fn grammar_crate_dirs() -> Vec<(String, PathBuf)> {
    let krates: BTreeSet<String> = read("fuzz/corpora.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty() && !l.starts_with("grammar\t"))
        .map(|l| l.split('\t').nth(1).unwrap().to_owned())
        .collect();
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--offline", "--locked"])
        .args(["--filter-platform", &host()])
        .current_dir(root())
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
    assert_eq!(dirs.len(), krates.len(), "every corpora crate resolves");
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

/// The crates whose scanner pushes through the pre-0.24 `tree_sitter/array.h`
/// (the aliasing UB): what the flag, and only the flag, protects.
fn crates_relying_on_the_flag() -> BTreeSet<String> {
    let mut exposed = BTreeSet::new();
    for (krate, dir) in grammar_crate_dirs() {
        let mut scanners = Vec::new();
        scanners_under(&dir, &mut scanners);
        for scanner in scanners {
            let src = std::fs::read_to_string(&scanner).unwrap();
            let pushes = [
                "array_push(",
                "array_grow_by(",
                "array_extend(",
                "array_insert(",
            ]
            .iter()
            .any(|m| src.contains(m));
            let header = scanner.parent().unwrap().join("tree_sitter/array.h");
            let aliasing = std::fs::read_to_string(&header)
                .is_ok_and(|h| h.contains("_array__grow((Array *)(self)"));
            if pushes && aliasing {
                exposed.insert(krate.clone());
            }
        }
    }
    exposed
}

#[test]
#[ignore = "failed at 0a85d18: a build started outside the repository compiled \
            tree-sitter-bash, -haskell and -html without -fno-strict-aliasing; \
            passes since E7h's fix round 1 vendored them on the conforming header \
            (the build's refusal is e7h_a_build_started_outside_the_checkout_refuses_\
            without_the_flag); while no scanner relies on the flag it returns at once, \
            and otherwise runs a cold `cargo check` of pmacs (minutes)"]
fn e7h_review1_pmacs_built_outside_its_root_compiles_no_aliasing_scanner_without_the_flag() {
    // Passes when the protection travels with the build rather than with the
    // directory it starts in, by any of: no shipped scanner left on the
    // aliasing header (the upstream fix, as python's and yaml's copies carry
    // it); the flag in the compile command of every scanner still on it (a
    // vendored build script's own `.flag`); or the build refusing to run
    // without the flag and saying so (a guard in pmacs's build script, which
    // sees the config's `[env]` exactly when the grammar crates do).
    let exposed = crates_relying_on_the_flag();
    if exposed.is_empty() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("pmacs-e7h-r1-outside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("target");
    let out = Command::new(env!("CARGO"))
        .args([
            "check",
            "--offline",
            "-p",
            "pmacs",
            "--lib",
            "--manifest-path",
        ])
        .arg(root().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .current_dir(&dir)
        // an empty wrapper, not an absent one: a user config's sccache
        // would otherwise wrap the C too, and can fail on its own
        .env("RUSTC_WRAPPER", "")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("HOST_CFLAGS")
        .env_remove("TARGET_CFLAGS")
        .env("CC_ENABLE_DEBUG_OUTPUT", "1")
        .output()
        .expect("cargo check from outside the repository");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        assert!(
            stderr.contains("-fno-strict-aliasing"),
            "the build outside the root failed, but not by naming the flag it lacks:\n{stderr}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let build = target.join("debug").join("build");
    let mut failures = Vec::new();
    for krate in &exposed {
        let mut compiled = 0;
        for e in std::fs::read_dir(&build).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(hash) = name.strip_prefix(&format!("{krate}-")) else {
                continue;
            };
            if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(record) = std::fs::read_to_string(e.path().join("output")) else {
                continue;
            };
            for line in record.lines().filter(|l| {
                l.starts_with("running:") && l.contains("scanner.c\"") && l.contains("\"-c\"")
            }) {
                compiled += 1;
                if !line.contains("\"-fno-strict-aliasing\"") {
                    failures.push(format!("{krate}: {line}"));
                }
            }
        }
        if compiled == 0 {
            failures.push(format!(
                "{krate}: no scanner compile recorded under {}",
                build.display()
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "a build of pmacs started outside its root compiles these scanners, which push \
         through the aliasing `array.h`, without -fno-strict-aliasing; GCC 16 at -O2 turns \
         tree-sitter-haskell's into the abort E7g unshipped it for:\n{failures:#?}"
    );
}

#[test]
fn e7h_review1_the_crates_relying_on_the_flag_are_the_recorded_residual() {
    // Control for the row above: its subject is exactly the scanners E7h
    // recorded as relying on the flag, so it fails for them and no others.
    // At 0a85d18 those were bash, haskell and html; E7h's fix round 1
    // vendored each with the conforming header, so none relies on it now.
    let exposed: Vec<String> = crates_relying_on_the_flag().into_iter().collect();
    assert_eq!(
        exposed,
        Vec::<String>::new(),
        "no shipped scanner is on the aliasing header"
    );
}
