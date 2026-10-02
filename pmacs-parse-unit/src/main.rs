// pmacs-parse-unit --- the parse unit's entry point (E7i).

//! Reads requests on stdin and answers on stdout until stdin closes.
//!
//! Natively this is the worker process the editor spawns per buffer,
//! started as `pmacs-parse-unit --memory-limit-mb N`: before serving it
//! caps its own address space at what it holds now plus `N` MiB
//! (`RLIMIT_AS`, soft and hard, so it cannot raise it again), so a parse
//! that grows past the limit gets a failed allocation and the process
//! aborts, taking only itself down. Under wasmtime the editor bounds the
//! module's memory instead, and the flag is absent.

#![forbid(unsafe_code)]

use std::io::{self, BufWriter};

fn main() {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--memory-limit-mb" {
            let Some(mb) = args.next().and_then(|v| v.parse::<u64>().ok()) else {
                eprintln!("pmacs-parse-unit: --memory-limit-mb needs a number");
                std::process::exit(2);
            };
            limit_memory(mb);
        } else {
            eprintln!("pmacs-parse-unit: unknown argument {arg}");
            std::process::exit(2);
        }
    }
    let stdin = io::stdin().lock();
    let stdout = BufWriter::new(io::stdout().lock());
    if let Err(error) = pmacs_parse_unit::serve(stdin, stdout) {
        eprintln!("pmacs-parse-unit: {error}");
        std::process::exit(1);
    }
}

/// Cap this process's address space at its current size plus `mb` MiB.
#[cfg(unix)]
fn limit_memory(mb: u64) {
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
    if let Err(error) = setrlimit(Resource::RLIMIT_AS, limit, limit) {
        eprintln!("pmacs-parse-unit: setrlimit(RLIMIT_AS, {limit}): {error}");
        std::process::exit(2);
    }
}

#[cfg(not(unix))]
fn limit_memory(_mb: u64) {
    eprintln!("pmacs-parse-unit: --memory-limit-mb has no effect on this platform");
}
