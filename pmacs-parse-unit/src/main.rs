// pmacs-parse-unit --- the parse unit's entry point (E7i).

//! Reads requests on stdin and answers on stdout until stdin closes, then
//! exits at once, abandoning a parse still running: stdin closes when the
//! editor ends, however it ends, so a worker never outlives its editor
//! (`pmacs_parse_unit::serve`).
//!
//! This is the worker process the editor spawns per buffer, started as
//! `pmacs-parse-unit --memory-limit-mb N [--memory-enforcement rlimit|watch]
//! [--report-memory]`; `--version` prints its version and protocol and
//! exits, which is how a release checks the worker it stages. With `rlimit`, the default, it caps its own address
//! space at what it holds now plus `N` MiB before serving (`RLIMIT_AS`, soft
//! and hard, so it cannot raise it again): a parse that grows past the
//! limit gets a failed allocation and the process aborts, taking only
//! itself down. That is preventive, and Linux grants it. macOS refuses it
//! (`EINVAL`), and there, or with `watch`, a thread reads the process's
//! peak resident memory every millisecond while a parse runs and exits it
//! past the same allowance: reactive, overshooting by what a parse grows
//! between two reads. `--report-memory` sends the resident size to the
//! editor while a parse runs, for its total watchdog where no cgroup holds
//! the workers. Core dumps are off: an aborted worker of several gigabytes
//! is evidence of nothing the editor's own report lacks.

#![forbid(unsafe_code)]

use std::io::{self, BufWriter};

use pmacs_parse_unit::ServeOptions;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut limit_mb = None;
    let mut watch = false;
    let mut report_memory = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--memory-limit-mb" => {
                let Some(mb) = args.next().and_then(|v| v.parse::<u64>().ok()) else {
                    eprintln!("pmacs-parse-unit: --memory-limit-mb needs a number");
                    std::process::exit(2);
                };
                limit_mb = Some(mb);
            }
            "--memory-enforcement" => match args.next().as_deref() {
                Some("rlimit") => watch = false,
                Some("watch") => watch = true,
                other => {
                    eprintln!(
                        "pmacs-parse-unit: --memory-enforcement takes rlimit or watch, not {other:?}"
                    );
                    std::process::exit(2);
                }
            },
            "--report-memory" => report_memory = true,
            "--version" => {
                println!(
                    "pmacs-parse-unit {} protocol {}",
                    env!("CARGO_PKG_VERSION"),
                    pmacs_parse_unit::PROTOCOL
                );
                return;
            }
            _ => {
                eprintln!("pmacs-parse-unit: unknown argument {arg}");
                std::process::exit(2);
            }
        }
    }
    no_core_dumps();
    let watch_growth = limit_mb.and_then(|mb| {
        if watch || !limit_memory(mb) {
            Some(mb * 1024 * 1024)
        } else {
            None
        }
    });
    let options = ServeOptions {
        watch_growth,
        report_memory,
    };
    let stdin = io::stdin().lock();
    let stdout = BufWriter::new(io::stdout());
    if let Err(error) = pmacs_parse_unit::serve(stdin, stdout, options) {
        eprintln!("pmacs-parse-unit: {error}");
        std::process::exit(1);
    }
}

/// Cap this process's address space at its current size plus `mb` MiB;
/// `false` where the platform refuses the limit, and the memory watch then
/// stands in for it.
#[cfg(unix)]
fn limit_memory(mb: u64) -> bool {
    use nix::sys::resource::{Resource, setrlimit};
    // The current size from /proc where it exists (Linux); elsewhere the
    // limit is the growth alone, which the editor sizes for.
    let held = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|p| p.parse::<u64>().ok())
        })
        .map_or(0, |pages| pages * 4096);
    let limit = held + mb * 1024 * 1024;
    match setrlimit(Resource::RLIMIT_AS, limit, limit) {
        Ok(()) => true,
        // Linux enforces it, so a refusal there is a fault. macOS refuses
        // to lower it (EINVAL, measured on CI's runner at E7i): there the
        // memory watch holds the unit instead.
        Err(error) if cfg!(target_os = "linux") => {
            eprintln!("pmacs-parse-unit: setrlimit(RLIMIT_AS, {limit}): {error}");
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!(
                "pmacs-parse-unit: setrlimit(RLIMIT_AS, {limit}): {error}; the memory watch holds this unit instead"
            );
            false
        }
    }
}

#[cfg(not(unix))]
fn limit_memory(_mb: u64) -> bool {
    false
}

/// No core file for this process (`RLIMIT_CORE` 0).
#[cfg(unix)]
fn no_core_dumps() {
    use nix::sys::resource::{Resource, setrlimit};
    let _ = setrlimit(Resource::RLIMIT_CORE, 0, 0);
}

#[cfg(not(unix))]
fn no_core_dumps() {}
