//! E2 --- GUI desktop basics, the rows that need a real daemon.
//!
//! One path exercises a real daemon and real wgpu rendering together:
//! `pmacs-gpu --headless-probe`, driven here in its Ctrl+wheel mode.

use std::path::{Path, PathBuf};

mod common;

fn gpu_binary() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("test binary directory")
        .join("pmacs-gpu")
}

/// E2.7 (D22) --- one Ctrl+wheel notch toward the user, banked through
/// the production `zoom_wheel_chords` and sent to a real daemon, makes
/// the frontend's applied font size larger: the chord that fell out is
/// `C-=`, the keymap runs `gpu.zoom-in`, `FontFacts` comes back, and
/// the probe reports the size it applied. `crdt`-gated like the E1.7
/// zoom probe: a daemon built without it refuses the attach for a
/// reason that has nothing to do with the wheel.
#[cfg(feature = "crdt")]
#[test]
fn ctrl_wheel_in_a_headless_gpu_changes_the_font_size() {
    let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
    let binary = gpu_binary();
    if !binary.exists() {
        assert!(
            !required,
            "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
            binary.display()
        );
        eprintln!(
            "skipping the GPU wheel probe: {} is not built",
            binary.display()
        );
        return;
    }

    let daemon = common::daemon::TestDaemon::spawn_with_env(&[
        ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
        ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
    ]);
    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join("gpu-wheel-probe.txt");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_ZOOM_WHEEL", "in")
        .output()
        .expect("run the headless GPU wheel probe");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU wheel probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU wheel probe: no wgpu adapter available");
        return;
    }

    let text = std::fs::read_to_string(&report).expect("probe report");
    let facts: std::collections::HashMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let fact = |key: &str| facts.get(key).copied().unwrap_or_default();

    assert_eq!(
        fact("completion_observed"),
        "true",
        "a deadline-driven pass must not read as success:\n{text}"
    );
    assert_eq!(
        fact("zoom_chords_sent"),
        "=",
        "one notch toward the user is exactly one C-=:\n{text}"
    );
    assert_eq!(
        fact("font_facts_observed"),
        "true",
        "the daemon must have relayed a font preference:\n{text}"
    );
    let before: u32 = fact("code_font_centi_before").parse().unwrap_or_default();
    let after: u32 = fact("code_font_centi_after").parse().unwrap_or_default();
    assert!(
        after > before,
        "Ctrl+wheel up must make the frontend's applied font size LARGER; \
         before {before}, after {after}:\n{text}"
    );
}
