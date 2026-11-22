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

/// How long a scripted probe may run before it is killed from here: a
/// probe's own limit was not always read (E10t.3).
const SCRIPT_OUTSIDE: std::time::Duration = std::time::Duration::from_mins(2);

/// `pmacs-gpu --headless-probe` under its scripted action (`popup`)
/// against the daemon at `socket`, attaching with no target and running
/// `steps` through the production `App`; its report as facts. `None`
/// when skipped (no `pmacs-gpu` built, or no wgpu adapter), which panics
/// under `PMACS_REQUIRE_GPU`. Extracted at E10a (D37) for the rows that
/// read where a window is after a jump, a rename or a closed panel
/// (`place:<label>`); `tests/e10t_gpu_window_settles.rs` carries the
/// form this follows.
pub fn run_script_probe(
    socket: &Path,
    label: &str,
    steps: &str,
) -> Option<HashMap<String, String>> {
    let binary = gpu_probe_binary()?;
    let dir = socket.parent().expect("socket parent");
    let report = dir.join(format!("{label}.report"));
    let stderr_path = dir.join(format!("{label}.stderr"));
    let started = std::time::Instant::now();
    let mut child = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(socket)
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", steps)
        .env("PMACS_GPU_PROBE_ACTION", "popup")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "30000")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&stderr_path).expect("probe stderr"))
        .spawn()
        .expect("spawn the headless GPU probe");
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the probe") {
            break status;
        }
        if started.elapsed() > SCRIPT_OUTSIDE {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "{label}: the probe was still running at the outside bound, {SCRIPT_OUTSIDE:?}, \
                 and was killed; its stderr: {:?}",
                std::fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let output = std::process::Output {
        status,
        stdout: Vec::new(),
        stderr: std::fs::read(&stderr_path).unwrap_or_default(),
    };
    probe_facts(&output, &report, label)
}
