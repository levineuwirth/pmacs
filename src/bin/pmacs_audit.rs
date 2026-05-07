// pmacs_audit.rs --- Audit-lint CLI binary (T M7.9).

//! `pmacs-audit` --- run the v1.0 audit-lint rule set against a
//! file, directory, or list of files. Prints a JSON report to stdout
//! and exits non-zero on the presence of any
//! [`pmacs::audit::Severity::Error`] findings.
//!
//! ## Invocation
//!
//! ```text
//! pmacs-audit <path> [<path>...]    # audit each path
//! pmacs-audit                        # audit '.' (cwd)
//! pmacs-audit --pretty <path>        # pretty-print the JSON
//! pmacs-audit --quiet <path>         # exit-code only, no stdout
//! ```
//!
//! Each `<path>` may be a file or a directory. Directories are
//! scanned recursively for `*.lua` files.
//!
//! ## Exit codes
//!
//! * `0` --- no findings, or only Warning/Info findings.
//! * `1` --- one or more Error-severity findings.
//! * `2` --- I/O or rule-set compile failure.
//!
//! The exit code makes the tool a drop-in CI gate: a failing
//! `pmacs-audit` halts the pipeline before the package ships. CI
//! that wants to allow Warnings as a transitional posture leaves
//! the exit-code semantics alone; CI that wants to block Warnings
//! too can post-process the JSON.

use std::path::PathBuf;
use std::process::ExitCode;

use pmacs::audit::{AuditEngine, AuditError, AuditReport};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut pretty = false;
    let mut quiet = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--pretty" => pretty = true,
            "--quiet" => quiet = true,
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--" => {
                paths.extend(args.by_ref().map(PathBuf::from));
            }
            s if s.starts_with('-') => {
                eprintln!("pmacs-audit: unknown flag: {s}");
                return ExitCode::from(2);
            }
            s => paths.push(PathBuf::from(s)),
        }
    }
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    let engine = match AuditEngine::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("pmacs-audit: rule-set compile failed: {e}");
            return ExitCode::from(2);
        }
    };

    let mut findings = Vec::new();
    for p in &paths {
        let res = if p.is_dir() {
            engine.audit_dir(p)
        } else if p.is_file() {
            engine.audit_file(p)
        } else {
            Err(AuditError::Io {
                path: p.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "path does not exist or is not a file/directory",
                ),
            })
        };
        match res {
            Ok(mut f) => findings.append(&mut f),
            Err(e) => {
                eprintln!("pmacs-audit: {e}");
                return ExitCode::from(2);
            }
        }
    }

    let report = AuditReport::new(findings);
    let exit = if report.summary.errors > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    };

    if !quiet {
        let json = if pretty {
            serde_json::to_string_pretty(&report)
        } else {
            serde_json::to_string(&report)
        };
        match json {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("pmacs-audit: JSON encode failed: {e}");
                return ExitCode::from(2);
            }
        }
    }

    exit
}

fn print_help() {
    println!(
        "pmacs-audit --- audit-lint for pmacs Lua packages (T M7.9)\n\
         \n\
         USAGE:\n  \
             pmacs-audit [--pretty] [--quiet] [<path>...]\n\
         \n\
         FLAGS:\n  \
             --pretty   pretty-print the JSON report\n  \
             --quiet    suppress stdout; exit-code only\n  \
             -h, --help show this help and exit\n\
         \n\
         EXIT CODES:\n  \
             0  no findings, or warnings/info only\n  \
             1  one or more `error`-severity findings\n  \
             2  I/O or rule-set compile failure\n"
    );
}
