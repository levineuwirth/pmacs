// main.rs --- Pmacs editor entry point.

//! Pmacs binary entry point.
//!
//! Parses command-line arguments and dispatches local TUI, daemon, attach,
//! remote bridge, and managed GPU modes.
//!
//! # Command-line surface
//!
//! ```text
//! pmacs [-nw|--no-window] [--help] [--version] [FILE]
//! pmacs --gpu [--socket NAME|PATH] [--] [FILE]
//! pmacs --daemon [--socket NAME|PATH]
//! pmacs --attach [--socket NAME|PATH]
//! pmacs --attach <target>
//! pmacs --daemon-attach [--socket NAME|PATH]
//! ```
//!
//! `--gpu` is additive: bare `pmacs [FILE]` remains the local TUI. The root
//! broker resolves the socket, requires a CRDT-capable build, discovers the
//! separate `pmacs-gpu` executable, and waits for that frontend's outcome.
//! The GPU child owns connect-or-start orchestration for the supplied daemon
//! executable. Direct TUI and GPU attach modes remain available for debugging.
//!
//! Anything else is a usage error and exits 2.

use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use pmacs::protocol::{AttachTarget, AttachTargetError};

const USAGE: &str = "\
usage: pmacs [-nw|--no-window] [--help] [--version] [FILE]
       pmacs --gpu [--socket NAME|PATH] [--] [FILE]
       pmacs --daemon [--socket NAME|PATH]
       pmacs --attach [--socket NAME|PATH]
       pmacs --attach <target>
       pmacs --daemon-attach [--socket NAME|PATH]

  -nw, --no-window   select the TUI frontend explicitly. This is the
                     default; `pmacs FILE` already opens the TUI, so
                     the flag exists to say so unambiguously in
                     scripts and wrappers.
  --gpu              start or reuse a CRDT daemon, then launch the
                     separate pmacs-gpu frontend. Requires a build
                     with the `crdt` feature, and the `pmacs-gpu`
                     binary either beside this one or on PATH.
                     When FILE is present, open it before the GPU window appears.
  --daemon           run as a foreground daemon listening on a Unix
                     socket; supervised by the user (systemd, tmux,
                     `nohup &`, etc.)
  --attach           connect to a running daemon as a frontend; F12
                     detaches without killing the daemon. With a
                     positional <target>, attach over the named
                     transport instead of the local socket.
  --daemon-attach    far-side bridge: connect to the local daemon
                     and forward bytes between stdin/stdout and the
                     socket. Used by remote transports (SSH, etc.);
                     does not take over the terminal.
  --socket NAME|PATH bare name → <runtime>/pmacs/NAME.sock; absolute
                     or relative path → used as-is. Default: `default`.
  -h, --help         print this message and exit
  -V, --version      print the version and exit

attach <target> shorthand:
  pmacs --attach mac-studio              ssh to host mac-studio
  pmacs --attach user@host               ssh as user
  pmacs --attach ssh:user@host/research  ssh, target instance `research`
  pmacs --attach local:/tmp/foo.sock     explicit local socket path
  pmacs --attach tls:host:port#cert.pem  TLS (parsed, not yet implemented)

A bare hostname is interpreted as `ssh:<host>`. Use `local:` or
`--socket` for local-socket attaches.
";

/// Frontend the user asked for by IN-PROCESS dispatch. Both variants
/// run the TUI: the GPU frontend is a separate binary reached through
/// `--gpu`, not a value of this enum. `Auto` records "the user did not
/// force TUI" so a future display-detecting default can dispatch
/// without touching the parsing layer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum FrontendChoice {
    /// Explicit `-nw` / `--no-window`. Always TUI.
    Tui,
    /// Default. Runs the TUI today. Reserved for a future
    /// display-detecting default; `--gpu` is the explicit GPU path and
    /// does not route through here.
    Auto,
}

#[derive(Debug, PartialEq, Eq)]
struct CliArgs {
    mode: Mode,
}

/// Top-level dispatch mode chosen by the CLI flags.
#[derive(Debug, PartialEq, Eq)]
enum Mode {
    /// Plain `pmacs` (or with `-nw` / a file): run a fresh in-process
    /// TUI. The default — no daemon-attach magic.
    Local {
        file: Option<PathBuf>,
        frontend: FrontendChoice,
    },
    /// `pmacs --gpu [--socket ...] [FILE]`: launch the separate GPU
    /// frontend, starting a CRDT daemon on the resolved socket when absent.
    Gpu {
        socket: Option<String>,
        file: Option<PathBuf>,
    },
    /// `pmacs --daemon [--socket ...]`: run a foreground daemon on a
    /// Unix socket, supervised by the user.
    Daemon { socket: Option<String> },
    /// `pmacs --attach [...]`: connect to a daemon and pump the
    /// local terminal. The form depends on whether the user gave a
    /// positional `<target>` or relied on `--socket NAME` / the
    /// default local socket.
    Attach(AttachMode),
    /// `pmacs --daemon-attach [--socket ...]`: byte-bridge stdin/stdout
    /// to the local daemon. The far-side mode used by SSH and other
    /// remote transports — does not take over the terminal.
    DaemonAttach { socket: Option<String> },
}

/// Sub-form of [`Mode::Attach`].
///
/// `LocalSocket` defers path resolution to dispatch time so
/// `parse_args` stays pure of `$XDG_RUNTIME_DIR`. `Target` carries
/// an already-parsed [`AttachTarget`] for the positional form.
#[derive(Debug, PartialEq, Eq)]
enum AttachMode {
    /// `--attach [--socket NAME|PATH]` — local-socket form. The
    /// `Option<String>` matches the previous `Mode::Attach { socket }`
    /// shape; resolution to a `PathBuf` happens at dispatch.
    LocalSocket(Option<String>),
    /// `--attach <target>` — positional form. The bare-hostname
    /// shorthand has already been applied (a positional without a
    /// kind prefix becomes an `ssh:` target before parse).
    Target(AttachTarget),
}

#[derive(Debug)]
enum CliResult {
    Run(CliArgs),
    Help,
    Version,
    Error(String),
}

/// Build the [`AttachMode`] for a given `(positional, socket)` pair.
///
/// Returns `Err(msg)` if the combination is invalid (positional +
/// `--socket`, non-UTF-8 positional, or unparseable target). The
/// caller wraps `Ok(mode)` in `CliResult::Run` and `Err(msg)` in
/// `CliResult::Error`.
fn build_attach_mode(file: Option<PathBuf>, socket: Option<String>) -> Result<AttachMode, String> {
    let Some(positional) = file else {
        return Ok(AttachMode::LocalSocket(socket));
    };
    if socket.is_some() {
        return Err("--attach <target> and --socket cannot be combined".into());
    }
    let target_str = positional
        .to_str()
        .ok_or_else(|| "attach <target> must be valid UTF-8".to_string())?
        .to_string();
    parse_attach_target_with_shorthand(&target_str)
        .map(AttachMode::Target)
        .map_err(|e| format!("invalid attach target {target_str:?}: {e}"))
}

/// Parse an `--attach <target>` positional, applying the
/// bare-hostname → `ssh:` shorthand.
///
/// The general [`AttachTarget::parse`] grammar requires a kind
/// prefix (`ssh:`, `local:`, `tls:`, `custom:`), but the spec calls
/// for `pmacs --attach mac-studio` to default to SSH. This helper
/// inserts the prefix when the input has no `:` at all. Inputs that
/// already contain a colon are passed through to the strict parser
/// unchanged, so `local:/tmp/x.sock`, `ssh:user@host/name`, etc.
/// continue to work.
///
/// IPv6 literals like `[::1]` are not supported as bare hostnames
/// — the strict parser would reject them anyway. SSH config aliases
/// and IPv4 / DNS hostnames are the supported shapes.
fn parse_attach_target_with_shorthand(s: &str) -> Result<AttachTarget, AttachTargetError> {
    if s.contains(':') {
        AttachTarget::parse(s)
    } else {
        AttachTarget::parse(&format!("ssh:{s}"))
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "single-pass parser keeps mutually exclusive CLI modes explicit"
)]
fn parse_args(args: &[OsString]) -> CliResult {
    let mut file: Option<PathBuf> = None;
    let mut frontend = FrontendChoice::Auto;
    let mut daemon = false;
    let mut gpu = false;
    let mut attach = false;
    let mut daemon_attach = false;
    let mut socket: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg.as_os_str().as_bytes().starts_with(b"-") && arg.to_str().is_none() {
            return CliResult::Error("option names must be valid UTF-8".into());
        }
        match arg.to_str() {
            Some("-nw" | "--no-window") => frontend = FrontendChoice::Tui,
            Some("--gpu") => gpu = true,
            Some("--daemon") => daemon = true,
            Some("--attach") => attach = true,
            Some("--daemon-attach") => daemon_attach = true,
            Some("--socket") => match iter.next() {
                Some(value) => match value.to_str() {
                    Some(value) => socket = Some(value.to_owned()),
                    None => {
                        return CliResult::Error("--socket value must be valid UTF-8".into());
                    }
                },
                None => return CliResult::Error("--socket requires a value".into()),
            },
            Some("-h" | "--help") => return CliResult::Help,
            Some("-V" | "--version") => return CliResult::Version,
            Some("--") => {
                if let Some(path) = iter.next() {
                    if file.is_some() {
                        return CliResult::Error("multiple files not yet supported".into());
                    }
                    file = Some(PathBuf::from(path));
                }
                if iter.next().is_some() {
                    return CliResult::Error("multiple files not yet supported".into());
                }
            }
            Some(flag) if flag.starts_with('-') => {
                return CliResult::Error(format!("unknown option: {flag}"));
            }
            Some(_) | None => {
                if file.is_some() {
                    return CliResult::Error("multiple files not yet supported".into());
                }
                file = Some(PathBuf::from(arg));
            }
        }
    }
    let mode_flags = u8::from(gpu) + u8::from(daemon) + u8::from(attach) + u8::from(daemon_attach);
    if mode_flags > 1 {
        return CliResult::Error(
            "--gpu, --daemon, --attach, and --daemon-attach are mutually exclusive".into(),
        );
    }
    if gpu {
        if frontend == FrontendChoice::Tui {
            return CliResult::Error("--gpu and --no-window are mutually exclusive".into());
        }
        return CliResult::Run(CliArgs {
            mode: Mode::Gpu { socket, file },
        });
    }
    if daemon {
        if file.is_some() {
            return CliResult::Error("--daemon does not take a file argument".into());
        }
        if frontend == FrontendChoice::Tui {
            return CliResult::Error("--daemon and --no-window are mutually exclusive".into());
        }
        return CliResult::Run(CliArgs {
            mode: Mode::Daemon { socket },
        });
    }
    if attach {
        return match build_attach_mode(file, socket) {
            Ok(mode) => CliResult::Run(CliArgs {
                mode: Mode::Attach(mode),
            }),
            Err(msg) => CliResult::Error(msg),
        };
    }
    if daemon_attach {
        if file.is_some() {
            return CliResult::Error("--daemon-attach does not take a file argument".into());
        }
        if frontend == FrontendChoice::Tui {
            return CliResult::Error(
                "--daemon-attach and --no-window are mutually exclusive".into(),
            );
        }
        return CliResult::Run(CliArgs {
            mode: Mode::DaemonAttach { socket },
        });
    }
    if socket.is_some() {
        return CliResult::Error(
            "--socket requires --gpu, --daemon, --attach, or --daemon-attach".into(),
        );
    }
    CliResult::Run(CliArgs {
        mode: Mode::Local { file, frontend },
    })
}

const PMACS_TEST_GPU_BIN: &str = "PMACS_TEST_GPU_BIN";

fn gpu_binary(current_exe: &Path, override_bin: Option<PathBuf>) -> (PathBuf, PathBuf) {
    let sibling = current_exe
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("pmacs-gpu");
    if let Some(override_bin) = override_bin {
        return (override_bin, sibling);
    }
    if sibling.is_file() {
        return (sibling.clone(), sibling);
    }
    (PathBuf::from("pmacs-gpu"), sibling)
}

fn run_gpu(socket: Option<&str>, file: Option<&Path>) -> ExitCode {
    if !cfg!(feature = "crdt") {
        eprintln!("pmacs: --gpu requires pmacs built with --features crdt");
        return ExitCode::FAILURE;
    }

    let socket_path = pmacs::socket_path::resolve_socket_path(socket);
    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("pmacs: cannot locate the running pmacs executable: {error}");
            return ExitCode::FAILURE;
        }
    };
    let initial_target = match file {
        Some(path) => {
            let cwd = match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(error) => {
                    eprintln!("pmacs: cannot determine launcher working directory: {error}");
                    return ExitCode::FAILURE;
                }
            };
            Some((cwd, pmacs::editor_core::expand_tilde(path.to_owned())))
        }
        None => None,
    };
    let (gpu, sibling) = gpu_binary(
        &current_exe,
        std::env::var_os(PMACS_TEST_GPU_BIN).map(PathBuf::from),
    );
    let mut command = Command::new(&gpu);
    command
        .arg("--managed-attach")
        .arg(&socket_path)
        .arg(&current_exe);
    if let Some((cwd, path)) = initial_target {
        command.arg("--initial-target").arg(cwd).arg(path);
    }
    let status = command.status();
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("pmacs: GPU frontend {} exited with {status}", gpu.display());
            status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .map_or(ExitCode::FAILURE, ExitCode::from)
        }
        Err(error) => {
            if gpu == Path::new("pmacs-gpu") {
                eprintln!(
                    "pmacs: could not launch GPU frontend: sibling {} is absent and PATH lookup \
                     for pmacs-gpu failed: {error}",
                    sibling.display()
                );
            } else {
                eprintln!(
                    "pmacs: could not launch GPU frontend {}: {error}",
                    gpu.display()
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match parse_args(&args) {
        CliResult::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        CliResult::Version => {
            println!("pmacs {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        CliResult::Error(msg) => {
            eprintln!("pmacs: {msg}");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
        CliResult::Run(parsed) => match parsed.mode {
            // FrontendChoice::Auto and ::Tui both run the TUI. The
            // match is structured this way deliberately so a future
            // display-detecting default can make the second arm
            // dispatch elsewhere without touching parsing. Note this
            // is NOT how the GPU frontend is reached — `--gpu` spawns
            // the separate pmacs-gpu binary via run_gpu().
            Mode::Local {
                file,
                frontend: FrontendChoice::Tui | FrontendChoice::Auto,
            } => match pmacs::editor::run(file) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("pmacs: {e}");
                    ExitCode::FAILURE
                }
            },
            Mode::Gpu { socket, file } => run_gpu(socket.as_deref(), file.as_deref()),
            Mode::Daemon { socket } => {
                let socket_path = pmacs::socket_path::resolve_socket_path(socket.as_deref());
                // The user-provided NAME (no slashes) becomes the
                // instance's `instance_name` for display in
                // `Hello.instance_identity`. Absolute / relative paths
                // don't have a natural name; treat as None.
                let instance_name = match socket {
                    Some(s) if !s.contains('/') => Some(s),
                    _ => None,
                };
                match pmacs::daemon::run_daemon(socket_path, instance_name) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("pmacs: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Mode::Attach(AttachMode::LocalSocket(socket)) => {
                let socket_path = pmacs::socket_path::resolve_socket_path(socket.as_deref());
                match pmacs::attach::run_attach(socket_path) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("pmacs: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Mode::Attach(AttachMode::Target(target)) => {
                use pmacs::attach_dispatch::{AttachDispatch, dispatch_attach};
                match dispatch_attach(Some(target)) {
                    AttachDispatch::RunAttachLocalSocket(path) => {
                        match pmacs::attach::run_attach(path) {
                            Ok(()) => ExitCode::SUCCESS,
                            Err(e) => {
                                eprintln!("pmacs: {e}");
                                ExitCode::FAILURE
                            }
                        }
                    }
                    AttachDispatch::RunAttachSsh(ssh_target) => {
                        match pmacs::attach::run_attach_ssh(ssh_target) {
                            Ok(()) => ExitCode::SUCCESS,
                            Err(e) => {
                                eprintln!("pmacs: {e}");
                                ExitCode::FAILURE
                            }
                        }
                    }
                    d @ AttachDispatch::DeferredInV01 { .. } => {
                        eprintln!(
                            "pmacs: {}",
                            d.deferred_message()
                                .expect("DeferredInV01 always has a message"),
                        );
                        ExitCode::FAILURE
                    }
                    AttachDispatch::RunLocal => {
                        // RunLocal is the None-target case from
                        // post-init dispatcher; CLI always supplies
                        // Some(target) here.
                        unreachable!("CLI dispatch always provides Some(target)")
                    }
                }
            }
            Mode::DaemonAttach { socket } => {
                let socket_path = pmacs::socket_path::resolve_socket_path(socket.as_deref());
                match pmacs::daemon_attach::run_daemon_attach(socket_path) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("pmacs: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn args(slice: &[&str]) -> Vec<OsString> {
        slice.iter().map(OsString::from).collect()
    }

    fn local_mode(parsed: CliArgs) -> (Option<PathBuf>, FrontendChoice) {
        match parsed.mode {
            Mode::Local { file, frontend } => (file, frontend),
            other => panic!("expected Local mode, got {other:?}"),
        }
    }

    #[test]
    fn no_args_runs_with_no_file_in_auto_frontend() {
        match parse_args(&args(&[])) {
            CliResult::Run(p) => {
                let (file, frontend) = local_mode(p);
                assert!(file.is_none());
                assert_eq!(frontend, FrontendChoice::Auto);
            }
            other => panic!("expected Run; got {other:?}"),
        }
    }

    #[test]
    fn nw_flag_selects_tui() {
        for flag in &["-nw", "--no-window"] {
            match parse_args(&args(&[flag])) {
                CliResult::Run(p) => {
                    let (_, frontend) = local_mode(p);
                    assert_eq!(frontend, FrontendChoice::Tui);
                }
                other => panic!("expected Run; got {other:?}"),
            }
        }
    }

    #[test]
    fn nw_flag_with_file_works_in_either_order() {
        for line in &[vec!["-nw", "README.md"], vec!["README.md", "-nw"]] {
            match parse_args(&args(line)) {
                CliResult::Run(p) => {
                    let (file, frontend) = local_mode(p);
                    assert_eq!(frontend, FrontendChoice::Tui);
                    assert_eq!(file, Some(PathBuf::from("README.md")));
                }
                other => panic!("expected Run for {line:?}; got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_flag_errors() {
        match parse_args(&args(&["--bogus"])) {
            CliResult::Error(m) => assert!(m.contains("--bogus")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn help_short_and_long() {
        for f in &["-h", "--help"] {
            assert!(matches!(parse_args(&args(&[f])), CliResult::Help));
        }
    }

    #[test]
    fn version_short_and_long() {
        for f in &["-V", "--version"] {
            assert!(matches!(parse_args(&args(&[f])), CliResult::Version));
        }
    }

    #[test]
    fn double_dash_treats_following_arg_as_path() {
        match parse_args(&args(&["--", "-nw"])) {
            CliResult::Run(p) => {
                let (file, frontend) = local_mode(p);
                // Without `--`, `-nw` would have been the flag; with
                // `--` it's a literal filename.
                assert_eq!(file, Some(PathBuf::from("-nw")));
                assert_eq!(frontend, FrontendChoice::Auto);
            }
            other => panic!("expected Run; got {other:?}"),
        }
    }

    #[test]
    fn multiple_files_rejected() {
        match parse_args(&args(&["a.txt", "b.txt"])) {
            CliResult::Error(m) => assert!(m.contains("multiple")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn daemon_flag_selects_daemon_mode_with_no_socket() {
        match parse_args(&args(&["--daemon"])) {
            CliResult::Run(CliArgs {
                mode: Mode::Daemon { socket },
            }) => assert!(socket.is_none()),
            other => panic!("expected Daemon; got {other:?}"),
        }
    }

    #[test]
    fn daemon_with_socket_name() {
        match parse_args(&args(&["--daemon", "--socket", "research"])) {
            CliResult::Run(CliArgs {
                mode: Mode::Daemon { socket },
            }) => assert_eq!(socket, Some("research".into())),
            other => panic!("expected Daemon with socket; got {other:?}"),
        }
    }

    #[test]
    fn daemon_with_absolute_socket_path() {
        match parse_args(&args(&["--daemon", "--socket", "/tmp/foo.sock"])) {
            CliResult::Run(CliArgs {
                mode: Mode::Daemon { socket },
            }) => assert_eq!(socket, Some("/tmp/foo.sock".into())),
            other => panic!("expected Daemon; got {other:?}"),
        }
    }

    #[test]
    fn daemon_rejects_file_argument() {
        match parse_args(&args(&["--daemon", "README.md"])) {
            CliResult::Error(m) => assert!(m.contains("file")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn daemon_rejects_nw_flag() {
        // `--daemon` and `-nw` make no sense together: daemon has no
        // controlling terminal to render into.
        match parse_args(&args(&["--daemon", "-nw"])) {
            CliResult::Error(m) => assert!(m.contains("mutually")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn socket_without_value_errors() {
        match parse_args(&args(&["--socket"])) {
            CliResult::Error(m) => assert!(m.contains("--socket")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn attach_flag_selects_attach_mode_with_no_socket() {
        match parse_args(&args(&["--attach"])) {
            CliResult::Run(CliArgs {
                mode: Mode::Attach(AttachMode::LocalSocket(socket)),
            }) => assert!(socket.is_none()),
            other => panic!("expected Attach(LocalSocket); got {other:?}"),
        }
    }

    #[test]
    fn attach_with_socket_name() {
        match parse_args(&args(&["--attach", "--socket", "research"])) {
            CliResult::Run(CliArgs {
                mode: Mode::Attach(AttachMode::LocalSocket(socket)),
            }) => assert_eq!(socket, Some("research".into())),
            other => panic!("expected Attach(LocalSocket); got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // M5.7d — --attach <target> positional form
    // -----------------------------------------------------------------

    #[test]
    fn attach_with_bare_hostname_is_ssh_shorthand() {
        match parse_args(&args(&["--attach", "mac-studio"])) {
            CliResult::Run(CliArgs {
                mode:
                    Mode::Attach(AttachMode::Target(AttachTarget::Ssh {
                        host,
                        user,
                        instance_name,
                    })),
            }) => {
                assert_eq!(host, "mac-studio");
                assert!(user.is_none());
                assert!(instance_name.is_none());
            }
            other => panic!("expected Attach(Target(Ssh)); got {other:?}"),
        }
    }

    #[test]
    fn attach_with_user_at_host_is_ssh_with_user() {
        match parse_args(&args(&["--attach", "alice@workstation"])) {
            CliResult::Run(CliArgs {
                mode:
                    Mode::Attach(AttachMode::Target(AttachTarget::Ssh {
                        host,
                        user,
                        instance_name,
                    })),
            }) => {
                assert_eq!(host, "workstation");
                assert_eq!(user, Some("alice".into()));
                assert!(instance_name.is_none());
            }
            other => panic!("expected Attach(Target(Ssh)); got {other:?}"),
        }
    }

    #[test]
    fn attach_with_explicit_ssh_kind_carries_instance_name() {
        match parse_args(&args(&["--attach", "ssh:bob@workstation/research"])) {
            CliResult::Run(CliArgs {
                mode:
                    Mode::Attach(AttachMode::Target(AttachTarget::Ssh {
                        host,
                        user,
                        instance_name,
                    })),
            }) => {
                assert_eq!(host, "workstation");
                assert_eq!(user, Some("bob".into()));
                assert_eq!(instance_name, Some("research".into()));
            }
            other => panic!("expected Attach(Target(Ssh)); got {other:?}"),
        }
    }

    #[test]
    fn attach_with_local_kind_is_local_socket_path() {
        match parse_args(&args(&["--attach", "local:/tmp/foo.sock"])) {
            CliResult::Run(CliArgs {
                mode: Mode::Attach(AttachMode::Target(AttachTarget::LocalSocket(p))),
            }) => assert_eq!(p, PathBuf::from("/tmp/foo.sock")),
            other => panic!("expected Attach(Target(LocalSocket)); got {other:?}"),
        }
    }

    #[test]
    fn attach_target_and_socket_flag_are_mutually_exclusive() {
        match parse_args(&args(&["--attach", "mac-studio", "--socket", "research"])) {
            CliResult::Error(m) => assert!(
                m.contains("--attach <target>") && m.contains("--socket"),
                "expected mutual-exclusion error, got: {m}",
            ),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn attach_with_unknown_kind_yields_parse_error() {
        match parse_args(&args(&["--attach", "telnet:host:23"])) {
            CliResult::Error(m) => assert!(
                m.contains("invalid attach target") && m.contains("telnet"),
                "expected target-parse error, got: {m}",
            ),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn attach_with_malformed_tls_yields_parse_error() {
        // tls: requires `endpoint#cert.pem`; missing `#` is a parse
        // error from AttachTarget::parse.
        match parse_args(&args(&["--attach", "tls:host:9999"])) {
            CliResult::Error(m) => assert!(
                m.contains("invalid attach target"),
                "expected target-parse error, got: {m}",
            ),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn daemon_and_attach_mutually_exclusive() {
        match parse_args(&args(&["--daemon", "--attach"])) {
            CliResult::Error(m) => assert!(m.contains("mutually")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn daemon_attach_flag_selects_daemon_attach_mode_with_no_socket() {
        match parse_args(&args(&["--daemon-attach"])) {
            CliResult::Run(CliArgs {
                mode: Mode::DaemonAttach { socket },
            }) => assert!(socket.is_none()),
            other => panic!("expected DaemonAttach; got {other:?}"),
        }
    }

    #[test]
    fn daemon_attach_with_socket_name() {
        match parse_args(&args(&["--daemon-attach", "--socket", "research"])) {
            CliResult::Run(CliArgs {
                mode: Mode::DaemonAttach { socket },
            }) => assert_eq!(socket, Some("research".into())),
            other => panic!("expected DaemonAttach; got {other:?}"),
        }
    }

    #[test]
    fn daemon_attach_rejects_file_argument() {
        match parse_args(&args(&["--daemon-attach", "README.md"])) {
            CliResult::Error(m) => assert!(m.contains("file")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn daemon_attach_and_no_window_mutually_exclusive() {
        match parse_args(&args(&["--daemon-attach", "-nw"])) {
            CliResult::Error(m) => assert!(m.contains("mutually")),
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn daemon_attach_excludes_daemon_and_attach() {
        for combo in &[
            vec!["--daemon", "--daemon-attach"],
            vec!["--attach", "--daemon-attach"],
            vec!["--daemon", "--attach", "--daemon-attach"],
        ] {
            let v: Vec<OsString> = combo.iter().map(OsString::from).collect();
            match parse_args(&v) {
                CliResult::Error(m) => assert!(
                    m.contains("mutually exclusive"),
                    "combo {combo:?} expected mutually-exclusive error, got: {m}",
                ),
                other => panic!("combo {combo:?}: expected Error; got {other:?}"),
            }
        }
    }
    #[test]
    fn gpu_flag_accepts_one_optional_file_and_socket() {
        for (argv, expected_socket, expected_file) in [
            (vec!["--gpu"], None, None),
            (
                vec!["--gpu", "--socket", "research"],
                Some("research"),
                None,
            ),
            (vec!["--gpu", "README.md"], None, Some("README.md")),
            (
                vec!["--gpu", "--socket", "research", "README.md"],
                Some("research"),
                Some("README.md"),
            ),
            (vec!["--gpu", "--", "-notes"], None, Some("-notes")),
        ] {
            match parse_args(&args(&argv)) {
                CliResult::Run(CliArgs {
                    mode: Mode::Gpu { socket, file },
                }) => {
                    assert_eq!(socket.as_deref(), expected_socket);
                    assert_eq!(file.as_deref(), expected_file.map(Path::new));
                }
                other => panic!("expected GPU mode; got {other:?}"),
            }
        }
    }

    #[test]
    fn gpu_flag_rejects_tui_other_modes_and_multiple_files() {
        for argv in [
            vec!["--gpu", "-nw"],
            vec!["--gpu", "--daemon"],
            vec!["--gpu", "--attach"],
            vec!["--gpu", "--daemon-attach"],
            vec!["--gpu", "one", "two"],
        ] {
            assert!(
                matches!(parse_args(&args(&argv)), CliResult::Error(_)),
                "accepted conflicting argv: {argv:?}"
            );
        }
    }

    #[test]
    fn gpu_file_keeps_non_utf8_bytes_and_launcher_tilde_expansion_is_exact() {
        let raw = OsString::from_vec(vec![b'n', b'o', b't', b'e', 0xff]);
        let parsed = parse_args(&[OsString::from("--gpu"), raw.clone()]);
        match parsed {
            CliResult::Run(CliArgs {
                mode: Mode::Gpu {
                    file: Some(file), ..
                },
            }) => assert_eq!(file.as_os_str().as_bytes(), raw.as_bytes()),
            other => panic!("expected raw GPU file; got {other:?}"),
        }

        let home = std::env::var_os("HOME").expect("test HOME");
        assert_eq!(
            pmacs::editor_core::expand_tilde(PathBuf::from("~")),
            PathBuf::from(&home)
        );
        assert_eq!(
            pmacs::editor_core::expand_tilde(PathBuf::from("~/notes")),
            PathBuf::from(home).join("notes")
        );
        assert_eq!(
            pmacs::editor_core::expand_tilde(PathBuf::from("~other/notes")),
            PathBuf::from("~other/notes")
        );
    }

    #[test]
    fn bare_socket_is_never_silently_ignored() {
        match parse_args(&args(&["--socket", "research"])) {
            CliResult::Error(message) => assert!(message.contains("--socket requires")),
            other => panic!("expected bare --socket error; got {other:?}"),
        }
    }
    #[test]
    fn gpu_binary_discovery_prefers_override_then_sibling_then_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("pmacs");
        let sibling = temp.path().join("pmacs-gpu");
        let override_bin = temp.path().join("override-gpu");

        let (selected, reported_sibling) = gpu_binary(&root, Some(override_bin.clone()));
        assert_eq!(selected, override_bin);
        assert_eq!(reported_sibling, sibling);

        std::fs::create_dir(&sibling).expect("create sibling directory");
        let (selected, _) = gpu_binary(&root, None);
        assert_eq!(selected, PathBuf::from("pmacs-gpu"));
        std::fs::remove_dir(&sibling).expect("remove sibling directory");

        std::fs::write(&sibling, b"gpu").expect("create sibling");
        let (selected, _) = gpu_binary(&root, None);
        assert_eq!(selected, sibling);

        std::fs::remove_file(&sibling).expect("remove sibling");
        let (selected, reported_sibling) = gpu_binary(&root, None);
        assert_eq!(selected, PathBuf::from("pmacs-gpu"));
        assert_eq!(reported_sibling, sibling);
    }
}
