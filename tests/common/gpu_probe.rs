//! The headless GPU probe as the acceptance suites drive it: the
//! `pmacs-gpu` binary beside this build's `pmacs`, and a finished
//! probe's report read as facts. Extracted at E10 (D37) from
//! `tests/e9_session_acceptance.rs`, its first user, for E10's suite,
//! its second; the bodies are E9's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The `pmacs-gpu` binary beside this build's `pmacs`, or `None` when
/// it is not built, which panics under `PMACS_REQUIRE_GPU` so the gate's
/// GPU leg never passes vacuously.
pub fn gpu_probe_binary() -> Option<PathBuf> {
    let binary = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("test binary directory")
        .join("pmacs-gpu");
    if binary.exists() {
        return Some(binary);
    }
    assert!(
        std::env::var_os("PMACS_REQUIRE_GPU").is_none(),
        "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
        binary.display()
    );
    eprintln!("skipping the GPU probe: {} is not built", binary.display());
    None
}

/// A finished headless probe's report as `key=value` facts, or `None`
/// when it had no wgpu adapter (status 3), which panics under
/// `PMACS_REQUIRE_GPU`; any other failure panics with the probe's
/// stderr and `context`.
pub fn probe_facts(
    output: &std::process::Output,
    report: &Path,
    context: &str,
) -> Option<HashMap<String, String>> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && std::env::var_os("PMACS_REQUIRE_GPU").is_none(),
            "headless GPU probe failed (status {:?}, {context}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU probe: no wgpu adapter available");
        return None;
    }
    // The probe's own log, shown by libtest when the row fails.
    eprintln!("probe stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let text = std::fs::read_to_string(report).expect("read the probe report");
    Some(
        text.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    )
}

/// One fact of a probe's report, which must be there.
pub fn fact<'a>(facts: &'a HashMap<String, String>, key: &str) -> &'a str {
    facts.get(key).map_or_else(
        || panic!("the report has no {key}; facts: {facts:?}"),
        String::as_str,
    )
}
