//! E10r --- what a frontend does with a session it lost (#390).
//!
//! Since E10r the daemon drops a frontend that does not take a frame within
//! `pmacs::frontend_stream::FRAME_WRITE_BOUND`. These rows are what the attached
//! TUI's two routes do about it, the owner's rulings of 2026-11-05 and 2026-11-06:
//! each comes back on a fresh view and says why, the local one ending, saying why,
//! if the daemon is gone, and the remote one stopping if its sessions keep being
//! cut before their first frame. They need one frontend and no CRDT, so unlike
//! `tests/e10r_frontend_write_bound.rs`, which holds the rows about the other
//! frontends, a build without CRDT runs them. Most stand in for the daemon or for
//! `ssh`, since what a frontend is told is the whole of the input here; the remote
//! route's run the real `--daemon-attach` bridge where it decides ([`BridgedSsh`]).

mod common;

use std::fmt::Write as _;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pmacs::attach::PMACS_TEST_SSH_BIN;
use pmacs::attach_reconnect::{DAEMON_AGE_SLACK, HANDSHAKE_RETRY_CAP, PMACS_TEST_BACKOFF_SCALE_MS};
use pmacs::frontend_stream::FRAME_WRITE_BOUND;
use pmacs_protocol::message::{
    AttachRequest, GoodbyeReason, Hello, InstanceCapabilities, InstanceIdentity, InstanceMessage,
    PROTOCOL_VERSION,
};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{FRAME_CUT_SHORT_SENTENCE, FrontendId};

use common::daemon::TestDaemon;
use common::pty::{ScreenTui, attached_tui, spawn_pmacs_in_pty_tapped};

/// G1's sentences, the owner's words of 2026-11-06, written out here so that a word
/// moved in the product fails a row. The first is said for certain, the second when
/// a `Hello` cannot tell.
const NEW_DAEMON_NOTICE: &str = "[pmacs: new daemon; work not autosaved is gone; M-x recover-file]";
const MAYBE_NEW_DAEMON_NOTICE: &str =
    "[pmacs reconnected: this may be a new daemon; C-x b shows what it holds]";
/// The plain notices, which say nothing of a new daemon.
const PLAIN_DAEMON_NOTICE: &str =
    "[pmacs reconnected on a new view: the daemon closed the last one]";
const PLAIN_LINK_NOTICE: &str = "[pmacs reconnected on a new view: the connection was lost]";
/// How long a notice stands whatever is typed under it, by G2's ruling.
const NOTICE_STANDS: Duration = Duration::from_secs(5);

/// A daemon too old for its age to fall inside the slack, for a row about what a real
/// session on a healthy daemon reads: under it, a `Hello` reads `may be a new daemon`,
/// which [`p6_a_hello_held_past_the_slack_on_a_real_daemon_reads_may_be`]'s neighbours
/// state.
fn past_the_age_slack() {
    std::thread::sleep(DAEMON_AGE_SLACK + Duration::from_secs(1));
}

fn daemon_log(daemon: &TestDaemon) -> String {
    std::fs::read_to_string(format!("{}.stderr.log", daemon.socket_path().display()))
        .unwrap_or_default()
}

/// A lone attached TUI, on a real daemon, in a build with or without CRDT: stopped
/// past the bound its socket fills and the write fails at the bound, and resumed
/// it comes back and a key typed after that is the daemon's to answer. A build
/// without CRDT admits one frontend, so this is the bound's one row there.
///
/// Nothing here writes but the idle tick, which takes under a second to fill a
/// stopped TUI's socket on Linux and about thirty on macOS (CI's two legs: one
/// write failure inside thirty seconds and none inside thirty), so the wait is
/// long and ends when the drop does.
#[test]
fn a_lone_attached_tui_stopped_past_the_bound_comes_back() {
    let daemon = TestDaemon::spawn();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(daemon.socket_path(), dir.path(), &[]);
    let pid =
        nix::unistd::Pid::from_raw(i32::try_from(tui.process_id().expect("pid")).expect("pid"));
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    past_the_age_slack();
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGSTOP).expect("SIGSTOP");
    let dropped = || daemon_log(&daemon).contains("sent no EOF within");
    let deadline = Instant::now() + Duration::from_mins(2);
    while !dropped() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let said = daemon_log(&daemon);
    assert!(dropped(), "the daemon never dropped it: {said}");
    assert!(
        said.contains(&format!(
            "was not written within {} ms",
            FRAME_WRITE_BOUND.as_millis()
        )),
        "{said}"
    );
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT).expect("SIGCONT");
    tui.wait_for_output("reconnected on a new view", Duration::from_secs(20));
    assert!(
        tui.wait_for_exit(Duration::from_millis(100)).is_none(),
        "the TUI came back rather than exiting"
    );
    tui.write_input(b"\x1bx").expect("M-x");
    tui.wait_for_output("M-x", Duration::from_secs(10));
    tui.write_input(b"\x07").expect("C-g");
    tui.write_input(b"\x1b[24~").expect("F12");
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .expect("F12 ends the session");
    assert!(status.success(), "{status:?}");
}

/// A daemon killed under an attached TUI ends it, saying why where the user can
/// read it, with no try after the one the connect refused. Until E10r's fix
/// round it exited with status 0, its one line on the alternate screen it was
/// leaving.
#[test]
fn e10r_3_a_daemon_killed_under_the_attached_tui_ends_it_saying_why() {
    let mut daemon = TestDaemon::spawn();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(daemon.socket_path(), dir.path(), &[]);
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(daemon.pid()).expect("pid")),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL");
    daemon.wait_for_exit().expect("the daemon is gone");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the TUI does not go on trying");
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(
        said.contains("is gone") && said.contains("refused"),
        "the reason is on the terminal the user is left with: {said:?}"
    );
}

/// pmacs with `args` under a real job-control `bash`, which runs `fg` once `go`
/// exists: `go` under `dir`, and `envs` beside the home.
#[cfg(target_os = "linux")]
fn under_a_shell(
    args: &[&str],
    dir: &Path,
    go: &Path,
    envs: &[(&str, &Path)],
) -> common::pty::PmacsPty {
    let mut all: Vec<(&str, &Path)> =
        vec![("HOME", dir), ("XDG_CONFIG_HOME", dir), ("E10R_GO", go)];
    all.extend_from_slice(envs);
    common::pty::spawn_pmacs_in_pty_under(
        &[
            "bash",
            "--norc",
            "--noprofile",
            "-i",
            "-c",
            "\"$0\" \"$@\"; until [ -e \"$E10R_GO\" ]; do sleep 0.02; done; fg",
        ],
        args,
        &all,
        24,
        80,
    )
}

/// Stop the shell's child with an external `SIGSTOP`, which gives the terminal back
/// to the shell, and wait for the daemon to have torn its session down.
#[cfg(target_os = "linux")]
fn stop_the_shells_child_until_dropped(shell: &common::pty::PmacsPty, daemon: &TestDaemon) {
    let bash = shell.process_id().expect("the shell's pid").to_string();
    let children = std::process::Command::new("pgrep")
        .args(["-P", &bash])
        .output()
        .expect("pgrep");
    let tui: i32 = String::from_utf8_lossy(&children.stdout)
        .split_whitespace()
        .next()
        .expect("the shell's child")
        .parse()
        .expect("a pid");
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(tui),
        nix::sys::signal::Signal::SIGSTOP,
    )
    .expect("SIGSTOP");
    let deadline = Instant::now() + Duration::from_mins(2);
    while !daemon_log(daemon).contains("sent no EOF within") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        daemon_log(daemon).contains("sent no EOF within"),
        "{}",
        daemon_log(daemon)
    );
}

/// The real thing, which the rows above with no shell cannot see: a client stopped
/// under a job-control shell and brought back by `fg`. `bash` hands it a canonical
/// terminal, so a TUI that came back with nothing done about that reconnects and
/// then hears no key. Linux, for the `pgrep` and for a `bash` that behaves as this
/// one; the modes are set again on every platform.
#[cfg(target_os = "linux")]
#[test]
fn e10r_3_a_tui_brought_back_by_a_shells_fg_hears_its_keys() {
    let daemon = TestDaemon::spawn();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let go = dir.path().join("go");
    let socket = daemon.socket_path().to_str().expect("utf-8").to_owned();
    let mut shell = under_a_shell(&["--attach", "--socket", &socket], dir.path(), &go, &[]);
    shell.wait_for_output("*scratch*", Duration::from_secs(30));
    past_the_age_slack();
    stop_the_shells_child_until_dropped(&shell, &daemon);
    std::fs::write(&go, b"").expect("say go");
    shell.wait_for_output("reconnected on a new view", Duration::from_secs(20));
    shell.write_input(b"\x1bx").expect("M-x");
    shell.wait_for_output("M-x", Duration::from_secs(10));
    shell.write_input(b"\x07").expect("C-g");
    shell.write_input(b"\x1b[24~").expect("F12");
    assert!(
        shell.wait_for_exit(Duration::from_secs(10)).is_some(),
        "F12 ended the TUI, and the shell's script with it"
    );
}

/// A daemon of the row's own on `socket`, in a thread: each connection is
/// handed to `session` with its number from 1, after the `Hello` and the
/// `AttachRequest`, until `session` says to stop by returning `false`. Returns
/// how many connections came.
struct StandInDaemon {
    connections: Arc<AtomicUsize>,
    dir: tempfile::TempDir,
    socket: std::path::PathBuf,
}

impl StandInDaemon {
    /// One daemon's `Hello`s: numbers going up, on a daemon too old for its age to fall
    /// inside the slack, so what a session reads of it is what the row's bytes say.
    fn serve(session: impl Fn(usize, &mut UnixStream) + Send + 'static) -> Self {
        Self::serve_saying(|n| (u64::try_from(n).expect("n"), 1_000), session)
    }

    /// [`Self::serve`], its `Hello` saying `identity(n)`, a frontend number and an
    /// `uptime_secs`, to connection `n`.
    fn serve_saying(
        identity: impl Fn(usize) -> (u64, u64) + Send + 'static,
        session: impl Fn(usize, &mut UnixStream) + Send + 'static,
    ) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let socket = dir.path().join("d.sock");
        Self::serve_on(dir, socket, identity, session)
    }

    /// [`Self::serve`] on the default socket of a far side whose `XDG_RUNTIME_DIR` is
    /// [`Self::runtime_dir`], where a bridge given no `--socket` looks.
    fn serve_as_the_far_sides_default(
        session: impl Fn(usize, &mut UnixStream) + Send + 'static,
    ) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let holder = dir.path().join("pmacs");
        std::fs::create_dir(&holder).expect("the runtime's directory");
        std::fs::set_permissions(&holder, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("chmod");
        let socket = holder.join("default.sock");
        Self::serve_on(
            dir,
            socket,
            |n| (u64::try_from(n).expect("n"), 1_000),
            session,
        )
    }

    fn serve_on(
        dir: tempfile::TempDir,
        socket: std::path::PathBuf,
        identity: impl Fn(usize) -> (u64, u64) + Send + 'static,
        session: impl Fn(usize, &mut UnixStream) + Send + 'static,
    ) -> Self {
        let listener =
            std::os::unix::net::UnixListener::bind(&socket).expect("bind the stand-in's socket");
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let n = counted.fetch_add(1, Ordering::SeqCst) + 1;
                let (id, uptime_secs) = identity(n);
                let hello = Hello {
                    protocol_version: PROTOCOL_VERSION,
                    assigned_frontend_id: FrontendId(id),
                    instance_identity: InstanceIdentity {
                        pmacs_version: "0.0.0-test".to_owned(),
                        build_hash: None,
                        instance_name: Some("stand-in".to_owned()),
                        uptime_secs,
                        working_directory: "/tmp".to_owned(),
                    },
                    instance_capabilities: InstanceCapabilities::default(),
                };
                if write_message(&mut stream, &hello).is_err()
                    || read_message::<AttachRequest>(&mut stream).is_err()
                {
                    continue;
                }
                session(n, &mut stream);
                // The stream closes here, with no `Goodbye` unless `session`
                // wrote one.
            }
        });
        Self {
            connections,
            dir,
            socket,
        }
    }

    fn socket(&self) -> std::path::PathBuf {
        self.socket.clone()
    }

    fn runtime_dir(&self) -> &Path {
        self.dir.path()
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

fn first_frame(stream: &mut UnixStream) {
    write_message(
        stream,
        &InstanceMessage::CellDelta {
            spans: Vec::new(),
            full_grid: true,
        },
    )
    .expect("the first frame");
}

/// A daemon that closes every new session before its first frame stops the
/// local route at the cap, so it cannot spin, saying why where the user can
/// read it.
#[test]
fn e10r_3_sessions_dropped_before_their_first_frame_stop_the_local_route() {
    let daemon = StandInDaemon::serve(|_, _| {});
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &daemon.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the route stops");
    assert_eq!(
        daemon.connections(),
        usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap"),
        "as many sessions as the cap, and no more"
    );
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains("before their first frame"), "{said:?}");
}

/// A session that reached its first frame starts the count again, so a route
/// that keeps getting a working session is not stopped by the cap, and each
/// new one says why the view is fresh. The stand-in's `Goodbye` ends it, with
/// status 0, as a daemon's `Goodbye` always has.
#[test]
fn e10r_3_a_first_frame_starts_the_local_routes_count_again() {
    let sessions = usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap") + 3;
    let daemon = StandInDaemon::serve(move |n, stream| {
        first_frame(stream);
        if n == sessions {
            write_message(
                stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .expect("Goodbye");
        }
    });
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &daemon.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    assert_eq!(daemon.connections(), sessions);
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(said.contains("reconnected on a new view"), "{said:?}");
}

/// A daemon restarted on its socket is how the local route meets a new one: with
/// nobody there it ends instead. A number not past the last says so for certain, in
/// the first of the owner's sentences. The stand-in's `Goodbye` ends it.
#[test]
fn e10r_3_a_new_daemon_on_the_local_routes_socket_says_so_for_certain() {
    let daemon = StandInDaemon::serve_saying(
        |n| if n == 1 { (2, 1_000) } else { (2, 0) },
        |n, stream| {
            first_frame(stream);
            if n == 2 {
                std::thread::sleep(Duration::from_millis(500));
                write_message(
                    stream,
                    &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
                )
                .expect("Goodbye");
            }
        },
    );
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &daemon.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(said.contains(NEW_DAEMON_NOTICE), "{said:?}");
}

/// Part of a frame and then the end of the socket, as a write past the bound
/// leaves it: a length prefix promising more than the bytes that follow it.
fn part_of_a_frame(stream: &mut UnixStream) {
    stream
        .write_all(&1000u32.to_be_bytes())
        .and_then(|()| stream.write_all(&[0u8; 10]))
        .expect("part of a frame");
}

/// A frame cut short cannot be followed by a `Goodbye`, so the frontend says
/// what happened and what the floor is, in the sentence a user can act on,
/// where it can be read. This is an attach past the floor, on the local route.
#[test]
fn e10r_floor_a_session_cut_short_says_each_frame_must_arrive_within_the_bound() {
    let daemon = StandInDaemon::serve(|_, stream| part_of_a_frame(stream));
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &daemon.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the route stops");
    assert!(!status.success(), "{status:?}");
    assert_eq!(
        daemon.connections(),
        usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap")
    );
    let said = tui.after_the_alternate_screen();
    assert!(said.contains(FRAME_CUT_SHORT_SENTENCE), "{said:?}");
    assert!(said.contains("within 2 s"), "{said:?}");
}

/// A session that had its first frame and is then cut short comes back saying
/// the bound is why, on the new session's bottom row.
#[test]
fn e10r_floor_a_session_that_comes_back_after_a_cut_says_the_bound_is_why() {
    let daemon = StandInDaemon::serve(|n, stream| {
        first_frame(stream);
        if n == 1 {
            part_of_a_frame(stream);
        } else {
            std::thread::sleep(Duration::from_millis(500));
            write_message(
                stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .expect("Goodbye");
        }
    });
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &daemon.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(
        said.contains("reconnected on a new view: a frame did not arrive within 2 s"),
        "{said:?}"
    );
}

/// A stand-in for `ssh` on the attach's own channel (stderr, the default): what
/// `sessions[n - 1]` says on its `n`th invocation, the last repeating. The stand-in
/// plays each session's bytes, keeps its pipes open long enough for the attach to
/// answer, and exits with the code given, as `ssh` exits with its remote command's.
struct StandInSsh {
    dir: tempfile::TempDir,
}

/// One invocation of [`StandInSsh`]: the bytes it plays and the shell that ends it.
struct RemoteSession {
    bytes: Vec<u8>,
    then: String,
}

impl StandInSsh {
    fn new(sessions: &[RemoteSession]) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let mut cases = String::new();
        for (i, session) in sessions.iter().enumerate() {
            let n = i + 1;
            let bytes = dir.path().join(format!("session-{n}.bin"));
            std::fs::write(&bytes, &session.bytes).expect("write the session's bytes");
            let arm = if n == sessions.len() {
                "*".to_owned()
            } else {
                n.to_string()
            };
            writeln!(
                cases,
                "{arm}) cat '{}' >&2; sleep 0.1; {} ;;",
                bytes.display(),
                session.then
            )
            .expect("write to a String");
        }
        let counter = dir.path().join("count");
        std::fs::write(&counter, "0").expect("seed the counter");
        let script = dir.path().join("ssh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nn=$(cat '{counter}')\nn=$((n+1))\necho $n > '{counter}'\ncase $n in\n{cases}esac\n",
                counter = counter.display(),
            ),
        )
        .expect("write the stand-in");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        Self { dir }
    }

    fn invocations(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("count"))
            .expect("the counter")
            .trim()
            .parse()
            .expect("a count")
    }

    fn attach(&self) -> common::pty::PmacsPty {
        self.attach_scaled("1")
    }

    fn attach_scaled(&self, scale_ms: &str) -> common::pty::PmacsPty {
        self.attach_with(scale_ms, &[])
    }

    fn attach_with(&self, scale_ms: &str, envs: &[(&str, &Path)]) -> common::pty::PmacsPty {
        let home = self.dir.path();
        let script = home.join("ssh");
        let mut all: Vec<(&str, &Path)> = vec![
            ("HOME", home),
            ("XDG_CONFIG_HOME", home),
            (PMACS_TEST_SSH_BIN, &script),
            (PMACS_TEST_BACKOFF_SCALE_MS, Path::new(scale_ms)),
        ];
        all.extend_from_slice(envs);
        common::pty::spawn_pmacs_in_pty(&["--attach", "farhost"], &all, 24, 80)
    }
}

/// A stand-in for `ssh` that runs the real `--daemon-attach` bridge over the pipes
/// the attach gave it, on the attach's own channel (stderr, the default). What it
/// leaves out is `ssh` itself and the network; every byte goes through the bridge.
struct BridgedSsh {
    dir: tempfile::TempDir,
}

impl BridgedSsh {
    fn new(socket: &Path) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let script = dir.path().join("ssh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho $$ >> '{count}'\nPMACS_ATTACH_PROTOCOL_FD=2 exec '{pmacs}' --daemon-attach --socket '{socket}'\n",
                count = dir.path().join("count").display(),
                pmacs = env!("CARGO_BIN_EXE_pmacs"),
                socket = socket.display(),
            ),
        )
        .expect("write the stand-in");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        Self { dir }
    }

    fn invocations(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("count"))
            .map_or(0, |count| count.lines().count())
    }

    /// A stand-in whose bridge is given no `--socket`, on a far side whose
    /// `XDG_RUNTIME_DIR` is `runtime`, as `ssh <host> pmacs --daemon-attach` is.
    fn on_the_default_socket_of(runtime: &Path) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let script = dir.path().join("ssh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nXDG_RUNTIME_DIR='{runtime}' PMACS_ATTACH_PROTOCOL_FD=2 exec '{pmacs}' --daemon-attach\n",
                runtime = runtime.display(),
                pmacs = env!("CARGO_BIN_EXE_pmacs"),
            ),
        )
        .expect("write the stand-in");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        Self { dir }
    }

    /// A real TUI through the bridge, the backoff scaled to `scale_ms`.
    fn attach(&self, scale_ms: &str) -> common::pty::PmacsPty {
        self.spawn(scale_ms, 80)
    }

    /// [`Self::attach`], `cols` wide.
    fn spawn(&self, scale_ms: &str, cols: u16) -> common::pty::PmacsPty {
        let home = self.dir.path();
        let script = home.join("ssh");
        common::pty::spawn_pmacs_in_pty(
            &["--attach", "farhost"],
            &[
                ("HOME", home),
                ("XDG_CONFIG_HOME", home),
                ("PMACS_STATE_HOME", home),
                (PMACS_TEST_SSH_BIN, &script),
                (PMACS_TEST_BACKOFF_SCALE_MS, Path::new(scale_ms)),
            ],
            24,
            cols,
        )
    }

    /// [`Self::attach`], its screen modelled from what it writes.
    fn attach_screen(&self, scale_ms: &str) -> ScreenTui {
        let home = self.dir.path();
        let script = home.join("ssh");
        let (pty, tap) = spawn_pmacs_in_pty_tapped(
            &["--attach", "farhost"],
            &[
                ("HOME", home),
                ("XDG_CONFIG_HOME", home),
                ("PMACS_STATE_HOME", home),
                (PMACS_TEST_SSH_BIN, &script),
                (PMACS_TEST_BACKOFF_SCALE_MS, Path::new(scale_ms)),
            ],
            24,
            80,
        );
        ScreenTui::new(pty, tap, 24, 80)
    }

    /// The real route with no daemon of the row's own: the socket is in the stand-in's
    /// directory, 0700, the bridge starts every daemon on it, and each reads `init_lua`
    /// from that directory and keeps its state there, as a host's daemons do.
    fn starting_its_own_daemons(init_lua: &str) -> (Self, StartedByTheBridge) {
        let holder = tempfile::TempDir::new().expect("tempdir");
        std::fs::set_permissions(
            holder.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("chmod");
        let socket = holder.path().join("d.sock");
        let ssh = Self::new(&socket);
        let config = ssh.dir.path().join("pmacs");
        std::fs::create_dir_all(&config).expect("the config directory");
        std::fs::write(config.join("init.lua"), init_lua).expect("init.lua");
        let mut lockfile = socket.as_os_str().to_os_string();
        lockfile.push(".lock");
        (
            ssh,
            StartedByTheBridge {
                lockfile: lockfile.into(),
                _holder: Some(holder),
            },
        )
    }

    /// A stand-in like [`Self::new`] whose second invocation on runs `first` before it
    /// starts its bridge.
    fn first_running(socket: &Path, first: &str) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let count = dir.path().join("count");
        let script = dir.path().join("ssh");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
echo $$ >> '{count}'
if [ "$(wc -l < '{count}')" -ge 2 ]; then
  {first}
fi
PMACS_ATTACH_PROTOCOL_FD=2 exec '{pmacs}' --daemon-attach --socket '{socket}'
"#,
                count = count.display(),
                pmacs = env!("CARGO_BIN_EXE_pmacs"),
                socket = socket.display(),
            ),
        )
        .expect("write the stand-in");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        Self { dir }
    }

    /// [`Self::first_running`] asking on the terminal, as `ssh` asks for a passphrase:
    /// `asking` beside it before a line read from `/dev/tty` lands in `got`, with
    /// `prologue` before that.
    fn asking_on_the_terminal(socket: &Path, prologue: &str) -> Self {
        Self::first_running(
            socket,
            &format!(
                "{prologue}\n  here=\"$(dirname \"$0\")\"\n  : > \"$here/asking\"\n  IFS= read -r line < /dev/tty\n  printf %s \"$line\" > \"$here/got\""
            ),
        )
    }

    /// Wait for the invocation to be asking, and a moment for it to be in its read.
    fn wait_until_asking(&self) {
        let asking = self.dir.path().join("asking");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !asking.exists() {
            assert!(Instant::now() < deadline, "the second ssh never asked");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    /// Whatever pid each invocation so far wrote, in order.
    fn pids(&self) -> Vec<i32> {
        std::fs::read_to_string(self.dir.path().join("count"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }

    /// A stand-in like [`Self::new`] whose second invocation on passes on what its
    /// bridge says only after `hold` seconds, as a stalled link does: what the daemon
    /// says first is written at once and read that much later.
    fn holding_later_sessions(socket: &Path, hold: u32) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let count = dir.path().join("count");
        let script = dir.path().join("ssh");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
echo $$ >> '{count}'
export PMACS_ATTACH_PROTOCOL_FD=2
if [ "$(wc -l < '{count}')" -ge 2 ]; then
  {{ '{pmacs}' --daemon-attach --socket '{socket}' 2>&1 >/dev/null; }} | {{ sleep {hold}; cat; }} >&2
else
  exec '{pmacs}' --daemon-attach --socket '{socket}'
fi
"#,
                count = count.display(),
                pmacs = env!("CARGO_BIN_EXE_pmacs"),
                socket = socket.display(),
            ),
        )
        .expect("write the stand-in");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        Self { dir }
    }
}

/// A `Hello` with the next frontend number and an age past the slack, as one old
/// daemon's are, so that sessions built in order are the same daemon's.
fn hello_frame() -> Vec<u8> {
    static NEXT: AtomicU64 = AtomicU64::new(2);
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &Hello {
            protocol_version: PROTOCOL_VERSION,
            assigned_frontend_id: FrontendId(NEXT.fetch_add(1, Ordering::SeqCst)),
            instance_identity: InstanceIdentity {
                pmacs_version: "0.0.0-test".to_owned(),
                build_hash: None,
                instance_name: Some("stand-in".to_owned()),
                uptime_secs: 1_000,
                working_directory: "/tmp".to_owned(),
            },
            instance_capabilities: InstanceCapabilities::default(),
        },
    )
    .expect("encode Hello");
    bytes
}

/// A session that gets as far as `Hello`, and then `after`.
fn session_with(after: &[u8], exit: i32) -> RemoteSession {
    let mut bytes = hello_frame();
    bytes.extend_from_slice(after);
    RemoteSession {
        bytes,
        then: format!("exit {exit}"),
    }
}

fn frame_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &InstanceMessage::CellDelta {
            spans: Vec::new(),
            full_grid: true,
        },
    )
    .expect("encode a frame");
    bytes
}

fn goodbye_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
    )
    .expect("encode Goodbye");
    bytes
}

fn cut_bytes() -> Vec<u8> {
    let mut bytes = 1000u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(&[0u8; 10]);
    bytes
}

/// An `ssh` that exits with a failure after a frame cut short in the bootstrap
/// is a reconnect, which at `8c46d9f` went on for ever. It stops at the cap with
/// the floor's sentence, so a route past the floor holds the others up only so
/// many times. [`e10r_remote_sessions_cut_before_their_first_frame_stop_at_the_cap_saying_why`]
/// is the same through the real bridge, which exits 0.
#[test]
fn e10r_remote_an_ssh_failing_after_a_cut_stops_at_the_cap_saying_why() {
    let ssh = StandInSsh::new(&[session_with(&cut_bytes(), 1)]);
    let mut tui = ssh.attach();
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .unwrap_or_else(|| panic!("it never stopped: {} attempts", ssh.invocations()));
    assert_eq!(
        ssh.invocations(),
        usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap")
    );
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains(FRAME_CUT_SHORT_SENTENCE), "{said:?}");
}

/// An `init.lua` that keeps a daemon writing to its frontends: every 20 ms a line at
/// the top of `*scratch*` that moves every row a window shows. A stopped frontend's
/// pipe and socket then fill in seconds, where an idle tick's small frames took 53.6 s
/// to fill the pipe a bridge writes to first (read once at `b25e779`).
const KEEPS_WRITING: &str = r#"
local count = 0
pmacs.async(function()
  while true do
    pmacs.workers.sleep(20):await()
    count = count + 1
    for _, b in ipairs(pmacs.buffer.list()) do
      if b:name() == "*scratch*" then
        b:insert(0, string.rep(tostring(count) .. " ", 16) .. "\n")
      end
    end
  end
end)
"#;

/// F9's first witness, the real thing over the remote route: a TUI stopped past the
/// bound through the real bridge and brought back by a real shell's `fg` is on a
/// fresh view, its notice saying why, and a key typed after that arrives. At
/// `b25e779` the bridge's exit 0 ended it, status 0. `bash` hands back a canonical
/// terminal, so with no `retake_terminal` on this route it says it is back and hears
/// nothing, the local row's defect. Linux, as that row is.
#[cfg(target_os = "linux")]
#[test]
fn e10r_remote_a_tui_brought_back_by_a_shells_fg_is_on_a_fresh_view_and_hears_its_keys() {
    let daemon = TestDaemon::spawn_with_config(KEEPS_WRITING);
    let ssh = BridgedSsh::new(daemon.socket_path());
    let dir = tempfile::TempDir::new().expect("tempdir");
    let go = dir.path().join("go");
    let script = ssh.dir.path().join("ssh");
    let mut shell = under_a_shell(
        &["--attach", "farhost"],
        dir.path(),
        &go,
        &[(PMACS_TEST_SSH_BIN, &script)],
    );
    shell.wait_for_output("*scratch*", Duration::from_secs(30));
    past_the_age_slack();
    stop_the_shells_child_until_dropped(&shell, &daemon);
    std::fs::write(&go, b"").expect("say go");
    shell.wait_for_output("reconnected on a new view", Duration::from_secs(30));
    assert_eq!(ssh.invocations(), 2, "one new session");
    shell.write_input(b"\x1bx").expect("M-x");
    shell.wait_for_output("M-x", Duration::from_secs(10));
    shell.write_input(b"\x07").expect("C-g");
    shell.write_input(b"\x1b[24~").expect("F12");
    assert!(
        shell.wait_for_exit(Duration::from_secs(10)).is_some(),
        "F12 ended the TUI, and the shell's script with it"
    );
    assert_eq!(ssh.invocations(), 2, "and nothing tried again");
}

/// A daemon the bridge started, which nothing here spawned: its lockfile names it,
/// and it is gone when this is.
struct StartedByTheBridge {
    lockfile: std::path::PathBuf,
    /// The directory the socket is in, for a route with no daemon of the row's own.
    _holder: Option<tempfile::TempDir>,
}

impl StartedByTheBridge {
    fn pid(&self) -> Option<i32> {
        std::fs::read_to_string(&self.lockfile)
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// `SIGKILL` the one running now, once its lockfile names one.
    fn kill_the_one_running(&self) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(pid) = self.pid()
                && nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                )
                .is_ok()
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "no daemon to kill");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for StartedByTheBridge {
    fn drop(&mut self) {
        let Some(pid) = self.pid().map(nix::unistd::Pid::from_raw) else {
            return;
        };
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
        let deadline = Instant::now() + Duration::from_secs(10);
        while nix::sys::signal::kill(pid, None).is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    }
}

/// F9's second witness: a daemon killed under the TUI, over the real bridge. The next
/// bridge finds nobody and starts a new daemon, so the notice says that for certain,
/// and a key typed after that is the new daemon's to answer. At `b25e779` the bridge's
/// exit 0 ended it, status 0, saying nothing of the kind. The kill comes as soon as
/// the session is up, which `uptime_secs` alone cannot tell (a real run read the plain
/// notice here): the new daemon's first frontend number is what says so.
#[test]
fn e10r_remote_a_daemon_killed_comes_back_on_a_new_one_saying_so_for_certain() {
    let mut daemon = TestDaemon::spawn();
    let started = StartedByTheBridge {
        lockfile: daemon.lockfile_path(),
        _holder: None,
    };
    let first = started.pid().expect("the daemon's lockfile");
    let ssh = BridgedSsh::new(daemon.socket_path());
    let mut tui = ssh.attach("500");
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(daemon.pid()).expect("pid")),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL");
    daemon.wait_for_exit().expect("the daemon is gone");
    tui.wait_for_output(NEW_DAEMON_NOTICE, Duration::from_mins(1));
    assert_eq!(ssh.invocations(), 2, "one new session");
    assert_ne!(
        started.pid().expect("the new daemon's lockfile"),
        first,
        "a daemon of its own"
    );
    tui.write_input(b"\x1bx").expect("M-x");
    tui.wait_for_output("M-x", Duration::from_secs(10));
    tui.write_input(b"\x07").expect("C-g");
    tui.write_input(b"\x1b[24~").expect("F12");
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .expect("F12 ends the session");
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), 2, "and nothing tried again");
}

/// Where other frontends got to the new daemon first, this one's number is past its
/// last, so only `uptime_secs` says what happened, and it can say only that this may
/// be a new daemon: two real TUIs through the real bridge on a daemon that lived past
/// the slack, one reattached quickly, which reads the first sentence, and the other at
/// a scale that leaves time for connections of the row's own to take the numbers past
/// its last. Two frontends, so a build with CRDT.
#[cfg(feature = "crdt")]
#[test]
fn e10r_remote_a_new_daemon_another_frontend_reached_first_may_be_one_by_its_uptime() {
    let mut daemon = TestDaemon::spawn();
    let started = StartedByTheBridge {
        lockfile: daemon.lockfile_path(),
        _holder: None,
    };
    let socket = daemon.socket_path().to_path_buf();
    let ssh = BridgedSsh::new(&socket);
    let mut late = ssh.attach("3000");
    late.wait_for_output("*scratch*", Duration::from_secs(30));
    let mut quick = ssh.attach("1");
    quick.wait_for_output("*scratch*", Duration::from_secs(30));
    std::thread::sleep(DAEMON_AGE_SLACK + Duration::from_secs(2));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(daemon.pid()).expect("pid")),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL");
    daemon.wait_for_exit().expect("the daemon is gone");
    quick.wait_for_output(NEW_DAEMON_NOTICE, Duration::from_mins(1));
    for _ in 0..4 {
        let mut stream = UnixStream::connect(socket.as_path()).expect("the new daemon");
        let _: Hello = read_message(&mut stream).expect("its Hello");
    }
    late.wait_for_output(MAYBE_NEW_DAEMON_NOTICE, Duration::from_mins(1));
    for tui in [&mut late, &mut quick] {
        tui.write_input(b"\x1b[24~").expect("F12");
        let status = tui
            .wait_for_exit(Duration::from_secs(10))
            .expect("F12 ends the session");
        assert!(status.success(), "{status:?}");
    }
    drop(started);
}

/// A link past the floor, through the real bridge, which exits 0 when the daemon
/// closes its socket: each session reaches the daemon and is cut before its first
/// frame, which costs the others the bound on every try, so they stop at the cap,
/// saying why. Until the ruling of 2026-11-06 the route stopped at the first.
#[test]
fn e10r_remote_sessions_cut_before_their_first_frame_stop_at_the_cap_saying_why() {
    let daemon = StandInDaemon::serve(|_, stream| part_of_a_frame(stream));
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("it never stopped: {} sessions", daemon.connections()));
    assert_eq!(
        daemon.connections(),
        usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap"),
        "as many sessions as the cap, and no more"
    );
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains(FRAME_CUT_SHORT_SENTENCE), "{said:?}");
    assert!(
        !said.contains("stderr"),
        "the daemon's bound, not its death, so G11's clause is not here: {said:?}"
    );
}

/// A session that worked and was then lost comes back through the real bridge, on a
/// fresh view, saying the daemon closed the last one. Until the ruling of 2026-11-06
/// the bridge's exit 0 ended the route. The stand-in's `Goodbye` ends it, status 0.
#[test]
fn e10r_remote_a_lost_session_comes_back_through_the_bridge_saying_why() {
    let daemon = StandInDaemon::serve(|n, stream| {
        first_frame(stream);
        if n == 2 {
            std::thread::sleep(Duration::from_millis(500));
            write_message(
                stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .expect("Goodbye");
        }
    });
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("it went on: {} sessions", daemon.connections()));
    assert!(status.success(), "{status:?}");
    assert_eq!(daemon.connections(), 2);
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(
        said.contains("reconnected on a new view: the daemon closed the last one"),
        "{said:?}"
    );
}

/// The same when the frame in flight was cut short, the bound's own case: its notice
/// says so.
#[test]
fn e10r_remote_a_session_cut_after_its_first_frame_comes_back_saying_the_bound_is_why() {
    let daemon = StandInDaemon::serve(|n, stream| {
        first_frame(stream);
        if n == 1 {
            part_of_a_frame(stream);
        } else {
            std::thread::sleep(Duration::from_millis(500));
            write_message(
                stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .expect("Goodbye");
        }
    });
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    assert_eq!(daemon.connections(), 2);
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(
        said.contains("reconnected on a new view: a frame did not arrive within 2 s"),
        "{said:?}"
    );
}

/// A second loss after a session that reached its first frame waits the schedule's
/// first delay, not its next, as the local route's does: each such loss is an event
/// of its own. The countdown says each wait, so this needs no clock: at a scale of
/// 1 s the first delay reads `in 1s` and the second, were the schedule walking on,
/// `in 2s`, and later ones more. Through the real bridge.
#[test]
fn e10r_remote_a_loss_after_a_first_frame_waits_the_schedules_first_delay() {
    let daemon = StandInDaemon::serve(|n, stream| {
        first_frame(stream);
        if n == 4 {
            std::thread::sleep(Duration::from_millis(200));
            write_message(
                stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .expect("Goodbye");
        }
    });
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1000");
    let status = tui
        .wait_for_exit(Duration::from_mins(1))
        .unwrap_or_else(|| panic!("it went on: {} sessions", daemon.connections()));
    assert!(status.success(), "{status:?}");
    assert_eq!(daemon.connections(), 4);
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(said.contains("reconnecting in 1s"), "{said:?}");
    assert!(
        !said.contains("reconnecting in 2s"),
        "a second loss waited the schedule's next delay: {said:?}"
    );
}

/// A failed attempt does not start the schedule again: a network that stays down
/// walks the curve, as it does at `b25e779`. A guard. Three failed attempts after
/// the loss read `in 1s`, `in 2s` and `in 4s` at a scale of 1 s.
#[test]
fn e10r_remote_a_network_that_stays_down_walks_the_schedule() {
    let down = 3;
    let mut sessions = vec![session_with(&frame_bytes(), 1)];
    sessions.extend((0..down).map(|_| RemoteSession {
        bytes: Vec::new(),
        then: "exit 255".to_owned(),
    }));
    sessions.push(session_with(&goodbye_bytes(), 0));
    let ssh = StandInSsh::new(&sessions);
    let mut tui = ssh.attach_scaled("1000");
    let status = tui
        .wait_for_exit(Duration::from_mins(1))
        .expect("the network comes back and the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), down + 2);
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(said.contains("reconnecting in 4s"), "{said:?}");
}

/// An `ssh` that failed says the connection was lost, which is all it knows; the
/// bridge exits 0 when the daemon is what closed. The stand-in's `Goodbye` ends it.
#[test]
fn e10r_remote_an_ssh_that_failed_comes_back_saying_the_connection_was_lost() {
    let mut last = frame_bytes();
    last.extend_from_slice(&goodbye_bytes());
    let ssh = StandInSsh::new(&[session_with(&frame_bytes(), 255), session_with(&last, 0)]);
    let mut tui = ssh.attach();
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), 2);
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(
        said.contains("reconnected on a new view: the connection was lost"),
        "{said:?}"
    );
}

/// F12 through the real bridge onto a real daemon ends the route, status 0, with no
/// second `ssh`, so a deliberate stop starts nothing. A guard of F9's.
#[test]
fn e10r_remote_f12_through_the_bridge_ends_it_and_starts_nothing() {
    let mut daemon = TestDaemon::spawn();
    let ssh = BridgedSsh::new(daemon.socket_path());
    let mut tui = ssh.attach("1");
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    tui.write_input(b"\x1b[24~").expect("F12");
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .expect("F12 ends the session");
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), 1, "and nothing tried again");
    assert!(daemon.is_alive(), "the daemon goes on");
}

/// A daemon stopped on purpose says `Goodbye`, which through the real bridge ends the
/// route, status 0, with no second `ssh`, so it is not started again. A guard of F9's.
#[test]
fn e10r_remote_a_daemon_stopped_on_purpose_ends_it_and_is_not_started_again() {
    let mut daemon = TestDaemon::spawn();
    let ssh = BridgedSsh::new(daemon.socket_path());
    let mut tui = ssh.attach("1");
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(daemon.pid()).expect("pid")),
        nix::sys::signal::Signal::SIGTERM,
    )
    .expect("SIGTERM");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends the session");
    assert!(status.success(), "{status:?}");
    daemon.wait_for_exit().expect("the daemon is gone");
    assert_eq!(ssh.invocations(), 1, "and nothing tried again");
}

/// A session that reached its first frame starts the count again: an `ssh` that
/// keeps failing after one keeps its reconnects, as it always has. The last
/// session's `Goodbye` ends it.
#[test]
fn e10r_remote_a_first_frame_starts_the_count_again() {
    let failures = usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap") + 2;
    let mut sessions: Vec<RemoteSession> = (0..failures)
        .map(|_| session_with(&frame_bytes(), 1))
        .collect();
    sessions.push(session_with(&goodbye_bytes(), 0));
    let ssh = StandInSsh::new(&sessions);
    let mut tui = ssh.attach();
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), failures + 1);
}

/// A network that is down reaches no `Hello` and counts nothing: past the cap's
/// worth of attempts it is still reconnecting, as it does at `8c46d9f`. A guard.
#[test]
fn e10r_remote_a_network_that_is_down_keeps_reconnecting() {
    let down = usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap") + 2;
    let mut sessions = vec![session_with(&frame_bytes(), 1)];
    sessions.extend((0..down).map(|_| RemoteSession {
        bytes: Vec::new(),
        then: "exit 255".to_owned(),
    }));
    sessions.push(session_with(&goodbye_bytes(), 0));
    let ssh = StandInSsh::new(&sessions);
    let mut tui = ssh.attach();
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the network comes back and the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), down + 2);
}

/// The daemon's `Goodbye` ends the route, status 0, whatever `ssh` exits with
/// after it, so a daemon stopped on purpose is not started again by a reconnect.
/// At `b25e779` a failing `ssh` made it a reconnect. A guard of F9's.
#[test]
fn e10r_remote_a_goodbye_ends_it_whatever_ssh_exits_with() {
    let mut after = frame_bytes();
    after.extend_from_slice(&goodbye_bytes());
    let ssh = StandInSsh::new(&[session_with(&after, 1)]);
    let mut tui = ssh.attach();
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .unwrap_or_else(|| panic!("it went on: {} attempts", ssh.invocations()));
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), 1, "and nothing tried again");
}

/// F12 ends the route, status 0, though `ssh` outlasts the kick and exits on its
/// SIGTERM, as a slow link's may. At `b25e779` that was a reconnect. A guard of
/// F9's.
#[test]
fn e10r_remote_f12_ends_it_though_ssh_outlasts_the_kick() {
    let ssh = StandInSsh::new(&[RemoteSession {
        bytes: [hello_frame(), frame_bytes()].concat(),
        then: "exec sleep 30".to_owned(),
    }]);
    let mut tui = ssh.attach();
    tui.wait_for_output("\x1b[?1049h", Duration::from_secs(10));
    tui.write_input(b"\x1b[24~").expect("F12");
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .unwrap_or_else(|| panic!("it went on: {} attempts", ssh.invocations()));
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), 1, "and nothing tried again");
}

/// `C-c` in the wait ends the local route with status 0, as the SSH route's
/// does; nothing else a user can reach does while a daemon keeps answering.
#[test]
fn e10r_3_ctrl_c_in_the_local_routes_wait_ends_it() {
    let daemon = StandInDaemon::serve(|_, stream| first_frame(stream));
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &daemon.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("2000"))],
    );
    tui.wait_for_output("reconnecting in", Duration::from_secs(20));
    tui.write_input(b"\x03").expect("C-c");
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .expect("C-c ends the wait");
    assert!(status.success(), "{status:?}");
    assert_eq!(daemon.connections(), 1, "and nothing tried again");
}

// ---------------------------------------------------------------------------
// E10r review 1's probes, committed as witnesses, and G1's sentences
// ---------------------------------------------------------------------------

/// `M-x`, then `name`, then a return: a command by name, which no chord can race (#470).
fn run_command(tui: &mut ScreenTui, name: &str) {
    let limit = Duration::from_secs(30);
    tui.write(b"\x1bx");
    tui.wait_for(limit, "the M-x prompt", &|t| t.bottom_row().contains("M-x"));
    tui.write(name.as_bytes());
    tui.write(b"\r");
}

/// **Review 1's Medium 1, its `p1` and `p3_autosave`:** a file typed into and not saved,
/// a recovery copy of it on disk, its daemon killed. The notice does not call that work
/// gone, and the positive control: the new daemon's `M-x recover-file` brings it back.
/// At `5fd9d25` the notice read `the old one's unsaved work is gone`.
#[test]
fn p1_the_notice_does_not_call_what_autosave_holds_gone() {
    const WORK: &str = "UNSAVEDWORK";
    let note = tempfile::TempDir::new().expect("tempdir");
    let file = note.path().join("notes.txt");
    std::fs::write(&file, "one\n").expect("write the file");
    let typed = note.path().join("typed");
    let init = format!(
        "pmacs.autosave.interval_ms(1000)\nlocal b = pmacs.buffer.find_or_open({file:?})\nlocal typed = io.open({typed:?})\nif typed then typed:close() else\n  io.open({typed:?}, 'w'):close()\n  b:insert(0, '{WORK}\\n')\nend\n",
        file = file.display().to_string(),
        typed = typed.display().to_string(),
    );
    let (ssh, started) = BridgedSsh::starting_its_own_daemons(&init);
    let mut tui = ssh.attach_screen("1");
    tui.wait_for(Duration::from_secs(30), "the first frame", &|t| {
        t.screen_contains("*scratch*") || t.screen_contains("notes.txt")
    });
    // The precondition: the work is in a recovery copy, and not in the file.
    let copies = ssh.dir.path().join("pmacs/autosave");
    let held = || {
        std::fs::read_dir(&copies).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                std::fs::read(entry.path())
                    .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(WORK))
            })
        })
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while !held() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(held(), "a recovery copy holds the work");
    assert_eq!(std::fs::read_to_string(&file).expect("the file"), "one\n");
    started.kill_the_one_running();
    tui.wait_for(Duration::from_mins(1), "the notice", &|t| {
        t.bottom_row().contains("reconnected") || t.bottom_row().contains("new daemon")
    });
    let said = tui.bottom_row();
    assert!(
        !said.contains("unsaved work is gone"),
        "it told the user the work is gone with a recovery copy on disk: {said:?}"
    );
    assert_eq!(said, NEW_DAEMON_NOTICE);
    // What that sentence sends the user to, once the notice has stood.
    std::thread::sleep(NOTICE_STANDS + Duration::from_secs(1));
    run_command(&mut tui, "editor.switch-buffer");
    tui.wait_for(Duration::from_secs(30), "the switch prompt", &|t| {
        t.bottom_row().contains("witch")
    });
    tui.write(b"notes.txt\r");
    tui.wait_for(Duration::from_secs(30), "the file, as saved", &|t| {
        t.screen_contains("notes.txt") && t.screen_contains("one")
    });
    assert!(
        !tui.screen_contains(WORK),
        "the new daemon does not hold it yet"
    );
    run_command(&mut tui, "recover-file");
    tui.wait_for(Duration::from_secs(30), "recover-file's question", &|t| {
        t.bottom_row().contains("Recover from autosave")
    });
    tui.write(b"yes\r");
    tui.wait_for(Duration::from_secs(30), "the work, brought back", &|t| {
        t.screen_contains(WORK)
    });
    tui.write(b"\x1b[24~");
    assert!(
        tui.pty.wait_for_exit(Duration::from_secs(10)).is_some(),
        "F12 ends it"
    );
}

/// A `Hello` past the last number and younger than the last daemon would be is only
/// `may be`, through either route, since a stalled link's reads the same. The stand-in's
/// `Goodbye` ends each. At `5fd9d25` this read the categorical sentence.
#[test]
fn g1_a_number_past_the_last_and_an_age_short_reads_may_be_on_either_route() {
    let saying = |n| if n == 1 { (2, 1_000) } else { (9, 0) };
    let session = |n: usize, stream: &mut UnixStream| {
        first_frame(stream);
        if n == 2 {
            std::thread::sleep(Duration::from_millis(500));
            write_message(
                stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .expect("Goodbye");
        }
    };
    let local = StandInDaemon::serve_saying(saying, session);
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &local.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(said.contains(MAYBE_NEW_DAEMON_NOTICE), "{said:?}");

    let remote = StandInDaemon::serve_saying(saying, session);
    let ssh = BridgedSsh::new(&remote.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(said.contains(MAYBE_NEW_DAEMON_NOTICE), "{said:?}");
}

/// **Review 1's Low 1, `p7_silent`'s sequence, deterministically:** a daemon killed young
/// under two TUIs, the second to come back reading a number past its last on a daemon
/// too young for an age to tell. At `5fd9d25` that read the plain notice on a new daemon,
/// 6 of 6 rounds. The real two TUIs are in the round's note.
#[test]
fn p7_a_number_past_the_last_on_a_daemon_too_young_to_tell_reads_may_be() {
    let daemon = StandInDaemon::serve_saying(
        |n| (u64::try_from(n).expect("n") + 1, 0),
        |n, stream| {
            first_frame(stream);
            if n == 2 {
                std::thread::sleep(Duration::from_millis(500));
                write_message(
                    stream,
                    &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
                )
                .expect("Goodbye");
            }
        },
    );
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(said.contains(MAYBE_NEW_DAEMON_NOTICE), "{said:?}");
    assert!(!said.contains(PLAIN_DAEMON_NOTICE), "{said:?}");
}

/// A TUI stopped, its daemon killed at `age`, a new one started by hand on the socket
/// with a connection of the row's own there first, then resumed: `p19_local`'s sequence.
fn p19_local_route_after(age: Duration) -> String {
    let mut daemon = TestDaemon::spawn();
    let socket = daemon.socket_path().to_path_buf();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &socket,
        dir.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let pid =
        nix::unistd::Pid::from_raw(i32::try_from(tui.process_id().expect("pid")).expect("pid"));
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    std::thread::sleep(age);
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGSTOP).expect("SIGSTOP");
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(daemon.pid()).expect("pid")),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL");
    daemon.wait_for_exit().expect("the daemon is gone");
    let mut second = common::daemon::spawn_daemon_process(&socket);
    common::daemon::wait_for_socket_or_exit(&socket, second.child(), Duration::from_secs(30))
        .expect("the new daemon");
    // The readiness wait took a number, and this takes one more: past the TUI's last.
    let mut stream = UnixStream::connect(&socket).expect("the new daemon");
    let _: Hello = read_message(&mut stream).expect("its Hello");
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT).expect("SIGCONT");
    tui.wait_for_output("[pmacs", Duration::from_secs(30));
    tui.wait_for_output("]", Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(300));
    let said = String::from_utf8_lossy(&tui.output()).into_owned();
    tui.write_input(b"\x1b[24~").expect("F12");
    let _ = tui.wait_for_exit(Duration::from_secs(10));
    said
}

/// **Review 1's Low 1, `p19_local`:** an old daemon of 8 s and a new one another
/// connection reached first, on the local route: its age says only `may be`. At
/// `5fd9d25` this was the categorical sentence.
#[test]
fn p19_the_local_route_onto_a_new_daemon_others_reached_first_reads_may_be() {
    let said = p19_local_route_after(Duration::from_secs(8));
    assert!(said.contains(MAYBE_NEW_DAEMON_NOTICE), "{said:?}");
    assert!(!said.contains("reconnected on a new view"), "{said:?}");
}

/// **Review 1's Medium 2, `p6_false_alarm`:** a real daemon never touched, the link
/// killed, the next session's first bytes held 7 s, a stalled link. The `Hello` reads
/// younger than the daemon would be, so the notice says `may be`, and the work is there
/// when the sentence's `C-x b` looks. At `5fd9d25` this called the work gone.
#[test]
fn p6_a_hello_held_past_the_slack_on_a_real_daemon_reads_may_be() {
    const WORK: &str = "UNSAVEDWORK";
    let note = tempfile::TempDir::new().expect("tempdir");
    let file = note.path().join("notes.txt");
    std::fs::write(&file, "one\n").expect("write the file");
    let daemon = TestDaemon::spawn_with_config(&format!(
        "local b = pmacs.buffer.find_or_open({:?})\nb:insert(0, '{WORK}\\n')\n",
        file.display().to_string()
    ));
    let before = std::fs::read_to_string(daemon.lockfile_path()).expect("the lockfile");
    let ssh = BridgedSsh::holding_later_sessions(daemon.socket_path(), 7);
    let mut tui = ssh.attach_screen("1");
    tui.wait_for(Duration::from_secs(30), "the first frame", &|t| {
        t.screen_contains("*scratch*") || t.screen_contains("notes.txt")
    });
    let first = ssh.pids()[0];
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(first),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL the link");
    tui.wait_for(Duration::from_mins(1), "the notice", &|t| {
        t.bottom_row().contains("new daemon") || t.bottom_row().contains("reconnected")
    });
    assert_eq!(tui.bottom_row(), MAYBE_NEW_DAEMON_NOTICE);
    assert_eq!(
        std::fs::read_to_string(daemon.lockfile_path()).expect("the lockfile"),
        before,
        "the same daemon"
    );
    std::thread::sleep(NOTICE_STANDS + Duration::from_secs(1));
    run_command(&mut tui, "editor.switch-buffer");
    tui.wait_for(Duration::from_secs(30), "the switch prompt", &|t| {
        t.bottom_row().contains("witch")
    });
    tui.write(b"notes.txt\r");
    tui.wait_for(Duration::from_secs(30), "the work, still there", &|t| {
        t.screen_contains(WORK)
    });
    tui.write(b"\x1b[24~");
    assert!(
        tui.pty.wait_for_exit(Duration::from_secs(15)).is_some(),
        "F12 ends it"
    );
}

/// The same with a hold of 1 s, on a daemon past the slack: nothing says this is another
/// daemon, and the notice stays plain.
#[test]
fn p6_a_hello_held_a_moment_on_a_real_daemon_stays_plain() {
    let daemon = TestDaemon::spawn();
    let ssh = BridgedSsh::holding_later_sessions(daemon.socket_path(), 1);
    let mut tui = ssh.attach_screen("1");
    tui.wait_for(Duration::from_secs(30), "the first frame", &|t| {
        t.screen_contains("*scratch*")
    });
    past_the_age_slack();
    let first = ssh.pids()[0];
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(first),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL the link");
    tui.wait_for(Duration::from_mins(1), "the notice", &|t| {
        t.bottom_row().contains("reconnected")
    });
    assert_eq!(tui.bottom_row(), PLAIN_LINK_NOTICE);
    tui.write(b"\x1b[24~");
    assert!(
        tui.pty.wait_for_exit(Duration::from_secs(15)).is_some(),
        "F12 ends it"
    );
}

/// **Review 1's Low 7:** on a narrow pane each sentence takes its short form, so the
/// clause that matters is not cut, where at `5fd9d25` a 40-column pane lost it.
#[test]
fn p4_a_narrow_pane_still_reads_what_is_gone() {
    let read = |saying: fn(usize) -> (u64, u64), cols: u16| {
        let daemon = StandInDaemon::serve_saying(saying, |_, stream| {
            first_frame(stream);
            std::thread::sleep(Duration::from_millis(300));
        });
        let ssh = BridgedSsh::new(&daemon.socket());
        let mut tui = ssh.spawn("1", cols);
        tui.wait_for_output("[", Duration::from_secs(20));
        tui.wait_for_output("]", Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(200));
        let said = String::from_utf8_lossy(&tui.output()).into_owned();
        tui.write_input(b"\x1b[24~").expect("F12");
        let _ = tui.wait_for_exit(Duration::from_secs(10));
        said
    };
    let certain = |n: usize| if n == 1 { (2, 1_000) } else { (2, 0) };
    let maybe = |n: usize| if n == 1 { (2, 1_000) } else { (9, 0) };
    let said = read(certain, 40);
    assert!(
        said.contains("[new daemon; work not autosaved is gone]"),
        "cut at 40 columns the clause is lost: {said:?}"
    );
    let said = read(maybe, 30);
    assert!(said.contains("[this may be a new daemon]"), "{said:?}");
}

/// Whether what `up` reads stays false, the row having just seen it so: a read of the
/// terminal can end inside a large update, between its frame and the notice over it.
fn staying_off(tui: &mut ScreenTui, up: &dyn Fn(&ScreenTui) -> bool) -> bool {
    tui.watch_for(Duration::from_millis(300), up).is_none()
}

/// **Review 1's Medium 3, `p4_typing`:** keys every 200 ms across a daemon's death, each
/// reaching the new daemon under the notice, which comes off only at the first key after
/// it has stood its floor. At `5fd9d25` it stood 128 ms, one key's gap.
#[test]
fn p4_the_notice_stands_its_floor_under_typing_and_the_keys_reach_the_daemon() {
    let (ssh, started) = BridgedSsh::starting_its_own_daemons("");
    let mut tui = ssh.attach_screen("1");
    tui.wait_for(Duration::from_secs(30), "the first frame", &|t| {
        t.screen_contains("*scratch*")
    });
    started.kill_the_one_running();
    let up = |t: &ScreenTui| t.bottom_row() == NEW_DAEMON_NOTICE;
    let deadline = Instant::now() + Duration::from_mins(1);
    let shown = loop {
        tui.write(b"q");
        if let Some(at) = tui.watch_for(Duration::from_millis(200), &up) {
            break at;
        }
        assert!(Instant::now() < deadline, "{}", tui.describe());
    };
    let mut typed_under_it = false;
    let off = loop {
        tui.write(b"q");
        if let Some(at) = tui.watch_for(Duration::from_millis(200), &|t| !up(t))
            && staying_off(&mut tui, &up)
        {
            break at;
        }
        typed_under_it = typed_under_it || tui.screen_contains("qq");
        assert!(Instant::now() < deadline, "{}", tui.describe());
    };
    let stood = off.saturating_duration_since(shown);
    assert!(
        stood >= NOTICE_STANDS,
        "the notice stood {} ms under typing",
        stood.as_millis()
    );
    assert!(
        stood < NOTICE_STANDS + Duration::from_secs(5),
        "it came off at the first key after the floor: {} ms; {}",
        stood.as_millis(),
        tui.describe()
    );
    assert!(typed_under_it, "the keys reach the daemon under it");
    tui.write(b"\x1b[24~");
    assert!(
        tui.pty.wait_for_exit(Duration::from_secs(10)).is_some(),
        "F12 ends it"
    );
}

/// A frame that takes the bottom row while the notice stands does not take the notice
/// off, and what that frame put there comes back when the notice does come off: the
/// daemon sends a cell only when it changes, so nothing else would ever repaint it.
#[test]
fn p4_a_prompt_under_the_notice_comes_back_when_it_comes_off() {
    let (ssh, started) = BridgedSsh::starting_its_own_daemons("");
    let mut tui = ssh.attach_screen("1");
    tui.wait_for(Duration::from_secs(30), "the first frame", &|t| {
        t.screen_contains("*scratch*")
    });
    started.kill_the_one_running();
    let up = |t: &ScreenTui| t.bottom_row() == NEW_DAEMON_NOTICE;
    tui.wait_for(Duration::from_mins(1), "the notice", &up);
    tui.write(b"\x1bx");
    let until = Instant::now() + Duration::from_secs(2);
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        if tui.watch_for(left, &|t| !up(t)).is_some() {
            assert!(
                !staying_off(&mut tui, &up),
                "a prompt under the notice took it off: {}",
                tui.describe()
            );
        }
    }
    std::thread::sleep(NOTICE_STANDS);
    tui.write(b"h");
    tui.wait_for(Duration::from_secs(20), "the prompt, come back", &|t| {
        let row = t.bottom_row();
        row.contains("M-x") && row.contains('h')
    });
    tui.write(b"\x07");
    tui.write(b"\x1b[24~");
    assert!(
        tui.pty.wait_for_exit(Duration::from_secs(10)).is_some(),
        "F12 ends it"
    );
}

/// Whether the process `pid` has gone, reaped or a zombie waiting on nobody's account.
fn gone(pid: i32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// A first session that worked and then failed, for a row about the reconnect after it.
fn worked_then_failed() -> RemoteSession {
    session_with(&frame_bytes(), 255)
}

/// **Review 1's Medium 4, its `p2`:** the link lost and the next `ssh` connects and says
/// nothing. `C-c`, which the banner offers, ends the route, status 0, and the `ssh` with
/// it. At `5fd9d25` nothing read a key and the route could be ended only from outside.
#[test]
fn p2_ctrl_c_ends_a_reconnect_whose_hello_never_comes() {
    let ssh = StandInSsh::new(&[
        worked_then_failed(),
        RemoteSession {
            bytes: Vec::new(),
            then: "echo $$ > \"$(dirname \"$0\")/second\"; exec sleep 30".to_owned(),
        },
    ]);
    let mut tui = ssh.attach();
    let second = ssh.dir.path().join("second");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !second.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(second.exists(), "the second ssh started");
    std::thread::sleep(Duration::from_millis(500));
    tui.write_input(b"\x03").expect("C-c");
    let status = tui.wait_for_exit(Duration::from_secs(5)).expect(
        "C-c, which the banner offers, did nothing: the route waits on a Hello with no deadline",
    );
    assert!(status.success(), "{status:?}");
    let pid = std::fs::read_to_string(&second)
        .expect("its pid")
        .trim()
        .parse()
        .expect("a pid");
    assert!(gone(pid), "the ssh went with it");
    assert_eq!(ssh.invocations(), 2, "and nothing tried again");
}

/// **G3's proof:** what `ssh` does on a reconnect, a stand-in reading a line from
/// `/dev/tty` before it passes anything on. It receives every byte typed, `Enter` ending
/// the line as on any terminal, and the route goes on: the session comes back and a
/// key typed after that is the daemon's to answer. At `5fd9d25` the terminal was raw, so
/// the line never ended and the route never went on.
#[test]
fn g3_a_prompt_on_the_terminal_gets_every_byte_and_the_route_goes_on() {
    let daemon = TestDaemon::spawn();
    let ssh = BridgedSsh::asking_on_the_terminal(daemon.socket_path(), "");
    let mut tui = ssh.attach_screen("1");
    tui.wait_for(Duration::from_secs(30), "the first frame", &|t| {
        t.screen_contains("*scratch*")
    });
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(ssh.pids()[0]),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL the link");
    ssh.wait_until_asking();
    tui.write(b"s3cret passphrase\r");
    let got = ssh.dir.path().join("got");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !got.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read_to_string(&got).expect("the prompt never got its line"),
        "s3cret passphrase",
        "every byte typed reaches the prompt, and none the TUI"
    );
    tui.wait_for(Duration::from_secs(30), "the session, back", &|t| {
        t.bottom_row().contains("reconnected")
    });
    std::thread::sleep(NOTICE_STANDS);
    tui.write(b"\x1bx");
    tui.wait_for(Duration::from_secs(10), "the daemon's answer", &|t| {
        t.bottom_row().contains("M-x")
    });
    tui.write(b"\x07");
    tui.write(b"\x1b[24~");
    let status = tui
        .pty
        .wait_for_exit(Duration::from_secs(10))
        .expect("F12 ends it");
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), 2, "one reconnect");
}

/// `C-c` at that prompt ends the route, status 0, its terminal handed back, and kills an
/// `ssh` that does not die of the signal itself. At `5fd9d25` it did nothing.
#[test]
fn g3_ctrl_c_at_a_prompt_ends_the_route_and_the_ssh() {
    let daemon = TestDaemon::spawn();
    let ssh = BridgedSsh::asking_on_the_terminal(daemon.socket_path(), "trap '' INT");
    let mut tui = ssh.attach("1");
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(ssh.pids()[0]),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL the link");
    ssh.wait_until_asking();
    tui.write_input(b"\x03").expect("C-c");
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .expect("C-c at the prompt ends the route");
    assert!(status.success(), "{status:?}");
    assert!(gone(ssh.pids()[1]), "the ssh went with it");
    assert_eq!(ssh.invocations(), 2, "and nothing tried again");
    tui.after_the_alternate_screen();
}

/// A stand-in that never speaks is killed at the deadline, shortened here, and that is a
/// failed attempt on the schedule: more of them than the cap, and the route goes on until
/// a far side answers, whose `Goodbye` ends it. At `5fd9d25` the first of them held it.
#[test]
fn g3_a_far_side_that_never_speaks_is_killed_at_the_deadline_with_no_limit() {
    let silent = usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap") + 1;
    let mut sessions = vec![worked_then_failed()];
    sessions.extend((0..silent).map(|_| RemoteSession {
        bytes: Vec::new(),
        then: "echo $$ >> \"$(dirname \"$0\")/silent\"; exec sleep 300".to_owned(),
    }));
    sessions.push(session_with(&goodbye_bytes(), 0));
    let ssh = StandInSsh::new(&sessions);
    let mut tui = ssh.attach_with("1", &[("PMACS_TEST_RECONNECT_HELLO_MS", Path::new("300"))]);
    let status = tui
        .wait_for_exit(Duration::from_mins(1))
        .unwrap_or_else(|| panic!("it held: {} attempts", ssh.invocations()));
    assert!(status.success(), "{status:?}");
    assert_eq!(ssh.invocations(), silent + 2);
    let pids = std::fs::read_to_string(ssh.dir.path().join("silent")).expect("the pids");
    let pids: Vec<i32> = pids
        .lines()
        .map(|pid| pid.parse().expect("a pid"))
        .collect();
    assert_eq!(pids.len(), silent);
    for pid in pids {
        assert!(gone(pid), "{pid} was killed at the deadline");
    }
}

/// `F12` in that wait does nothing, since nothing reads a key: its bytes go to whatever
/// is reading the terminal, and what nothing read goes with the wait, so it does not land
/// on the view the session comes back to. `C-c` is what ends a wait.
#[test]
fn g3_f12_in_the_wait_ends_nothing_and_what_was_typed_goes_with_the_wait() {
    let daemon = TestDaemon::spawn();
    let ssh = BridgedSsh::first_running(
        daemon.socket_path(),
        ": > \"$(dirname \"$0\")/asking\"; until [ -e \"$(dirname \"$0\")/go\" ]; do sleep 0.02; done",
    );
    let mut tui = ssh.attach_screen("1");
    tui.wait_for(Duration::from_secs(30), "the first frame", &|t| {
        t.screen_contains("*scratch*")
    });
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(ssh.pids()[0]),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL the link");
    ssh.wait_until_asking();
    tui.write(b"\x1b[24~qqq");
    assert!(
        tui.pty.wait_for_exit(Duration::from_secs(1)).is_none(),
        "F12 ended the wait"
    );
    std::fs::write(ssh.dir.path().join("go"), b"").expect("say go");
    tui.wait_for(Duration::from_secs(30), "the session, back", &|t| {
        t.bottom_row().contains("reconnected")
    });
    assert!(
        tui.watch_for(Duration::from_secs(1), &|t| t.screen_contains("qqq"))
            .is_none(),
        "what was typed in the wait landed on the new view: {}",
        tui.describe()
    );
    tui.write(b"\x1b[24~");
    let status = tui
        .pty
        .wait_for_exit(Duration::from_secs(10))
        .expect("F12 ends it");
    assert!(status.success(), "{status:?}");
}

/// A daemon that dies after each first frame, as a stand-in's `Hello`s and sessions read:
/// `identity(n)`, and a first frame, and closed.
fn serving_first_frames(identity: fn(usize) -> (u64, u64)) -> StandInDaemon {
    StandInDaemon::serve_saying(identity, |_, stream| {
        first_frame(stream);
        std::thread::sleep(Duration::from_millis(50));
    })
}

/// **Review 1's Medium 5, its `p3`:** each session gets its first frame and is lost, each
/// `Hello` a new daemon's for certain. The route stops after three, saying so, naming the
/// host and where the stderr is. At `5fd9d25` it never stopped, 50 a minute.
#[test]
fn p3_new_daemons_that_keep_dying_stop_the_route() {
    let daemon = serving_first_frames(|_| (2, 0));
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let ended = tui.wait_for_exit(Duration::from_secs(10));
    let status = ended.unwrap_or_else(|| {
        panic!(
            "it never stopped: {} new daemons in 10 s",
            daemon.connections()
        )
    });
    assert!(!status.success(), "{status:?}");
    assert_eq!(
        daemon.connections(),
        1 + usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap"),
        "the first, and as many new ones as the cap"
    );
    let said = tui.after_the_alternate_screen();
    for needle in [
        "a new daemon on farhost died 3 times in a row",
        "under 30 s",
        "$XDG_RUNTIME_DIR/pmacs/default.sock.stderr",
    ] {
        assert!(said.contains(needle), "{needle:?} in {said:?}");
    }
}

/// **G11, the owner's ruling of 2026-11-07:** a far side whose daemon goes before each
/// first frame stops at the older cap, not at `p3`'s, and a daemon that dies that young has
/// likely left its reason in the file the bridge keeps, so the sentence names it, in `p3`'s
/// words. The row follows the sentence. The real bridge, given no `--socket`, finds the
/// stand-in on the default socket of its own `XDG_RUNTIME_DIR`, which the client cannot
/// know and names by the variable, and the path with the variable filled in holds a
/// planted line, the stand-in having no stderr of its own to keep; `g4_a_daemon_the_bridge`
/// reads a real daemon's, and this round's note a real death's. At `d832d69` the sentence
/// stopped at `stopped trying`.
#[test]
fn g11_sessions_closed_before_their_first_frame_name_the_far_sides_stderr() {
    let daemon = StandInDaemon::serve_as_the_far_sides_default(|_, _| {});
    let runtime = daemon.runtime_dir();
    std::fs::write(
        runtime.join("pmacs/default.sock.stderr"),
        "PLANTED DAEMON WORDS\n",
    )
    .expect("plant the line");
    let ssh = BridgedSsh::on_the_default_socket_of(runtime);
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("it never stopped: {} sessions", daemon.connections()));
    assert!(!status.success(), "{status:?}");
    assert_eq!(
        daemon.connections(),
        usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap"),
        "as many sessions as the cap, and no more"
    );
    let said = tui.after_the_alternate_screen();
    assert!(
        said.contains(
            "the daemon closed 3 sessions in a row before their first frame, so this one has \
             stopped trying; the last one's stderr is on farhost at \
             $XDG_RUNTIME_DIR/pmacs/default.sock.stderr"
        ),
        "{said:?}"
    );
    let named = said
        .split_whitespace()
        .find(|word| word.starts_with("$XDG_RUNTIME_DIR/"))
        .expect("the sentence names a path");
    let path = named.replacen("$XDG_RUNTIME_DIR", &runtime.display().to_string(), 1);
    let kept = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert!(kept.contains("PLANTED DAEMON WORDS"), "{kept:?}");
}

/// The local route starts no daemon, so its sentence names no file: a guard, whole, on what
/// the same stand-in's sessions read there.
#[test]
fn g11_the_local_routes_sentence_stays() {
    let daemon = StandInDaemon::serve(|_, _| {});
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        &daemon.socket(),
        home.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the route stops");
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(
        said.contains(
            "the daemon closed 3 sessions in a row before their first frame, so this one has \
             stopped trying"
        ),
        "{said:?}"
    );
    assert!(!said.contains("stderr"), "{said:?}");
}

/// A session of 30 s or more resets the count, so a daemon that lived that long is not
/// one of three in a row. A guard, at the real floor, with what comes after it stopping
/// at three again: without the reset it stops at the third connection after the first.
#[test]
fn g4_a_session_past_the_floor_starts_the_count_again() {
    let daemon = StandInDaemon::serve_saying(
        |_| (2, 0),
        |n, stream| {
            first_frame(stream);
            std::thread::sleep(if n == 3 {
                Duration::from_secs(31)
            } else {
                Duration::from_millis(50)
            });
        },
    );
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_mins(2))
        .unwrap_or_else(|| panic!("it never stopped: {}", daemon.connections()));
    assert!(!status.success(), "{status:?}");
    assert_eq!(daemon.connections(), 6, "1, 2 and 3 long, then 4, 5 and 6");
}

/// A reading of the second sentence neither counts nor resets: certain, may be, certain,
/// certain stops at the third certain one. A guard; a `may be` that counted would stop a
/// connection sooner and one that reset would go on.
#[test]
fn g4_a_may_be_neither_counts_nor_resets() {
    let daemon = serving_first_frames(|n| if n == 3 { (3, 0) } else { (2, 0) });
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("it never stopped: {}", daemon.connections()));
    assert!(!status.success(), "{status:?}");
    assert_eq!(daemon.connections(), 5);
}

/// So a link that keeps being lost onto a daemon it cannot tell from a new one does not
/// stop the route, however often: a guard, past the cap's worth, its last `Goodbye`
/// ending it with status 0.
#[test]
fn g4_a_slow_link_reading_only_may_be_does_not_stop_the_route() {
    let last = usize::try_from(HANDSHAKE_RETRY_CAP).expect("cap") + 3;
    let daemon = StandInDaemon::serve_saying(
        |n| (u64::try_from(n).expect("n") + 1, 0),
        move |n, stream| {
            first_frame(stream);
            if n == last {
                write_message(
                    stream,
                    &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
                )
                .expect("Goodbye");
            }
        },
    );
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("it went on: {}", daemon.connections()));
    assert!(status.success(), "{status:?}");
    assert_eq!(daemon.connections(), last);
}

/// A daemon the bridge starts keeps its stderr in a file beside the socket, as the
/// window's route does, so what a daemon said before it died can be read. A planted line
/// stands in for a panic in this row; a real one is in the round's note, by a plant. At
/// `5fd9d25` it went to `/dev/null`.
#[test]
fn g4_a_daemon_the_bridge_starts_keeps_its_stderr_beside_its_socket() {
    let (ssh, started) =
        BridgedSsh::starting_its_own_daemons("io.stderr:write('PLANTED DAEMON WORDS\\n')\n");
    let mut tui = ssh.attach("1");
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    let path = started.lockfile.with_extension("stderr");
    let said = std::fs::read_to_string(&path).expect("the stderr file beside the socket");
    assert!(said.contains("PLANTED DAEMON WORDS"), "{said:?}");
    tui.write_input(b"\x1b[24~").expect("F12");
    assert!(
        tui.wait_for_exit(Duration::from_secs(10)).is_some(),
        "F12 ends it"
    );
}

/// `pmacs --daemon --stop`, as a user runs it. A build with no CRDT refuses that second
/// frontend while a TUI is attached, saying so, so there the row stops it by `SIGTERM`,
/// which says the same `Goodbye`.
fn stop_the_daemon(daemon: &TestDaemon, home: &Path) {
    if cfg!(feature = "crdt") {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_pmacs"))
            .args(["--daemon", "--stop", "--socket"])
            .arg(daemon.socket_path())
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("XDG_DATA_HOME", home)
            .env("XDG_STATE_HOME", home)
            .env("XDG_CACHE_HOME", home)
            .env("PMACS_STATE_HOME", home)
            .output()
            .expect("--stop");
        assert!(output.status.success(), "{output:?}");
    } else {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(daemon.pid()).expect("pid")),
            nix::sys::signal::Signal::SIGTERM,
        )
        .expect("SIGTERM");
    }
}

/// **Review 1's Low 5, its `p10_goodbye`:** a daemon's `Goodbye` under a live TUI, on
/// either route: the reason is on the terminal the user is left with, status 0. At
/// `5fd9d25` it was written to the alternate screen it was leaving, a false comment
/// beside it saying it would surface after, and the terminal read nothing.
#[test]
fn p10_a_goodbyes_reason_is_on_the_terminal_on_either_route() {
    let mut daemon = TestDaemon::spawn();
    let socket = daemon.socket_path().to_path_buf();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(&socket, dir.path(), &[]);
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    stop_the_daemon(&daemon, dir.path());
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains("ShuttingDown"), "local: {said:?}");
    daemon.wait_for_exit().expect("the daemon is gone");

    let mut daemon = TestDaemon::spawn();
    let socket = daemon.socket_path().to_path_buf();
    let ssh = BridgedSsh::new(&socket);
    let mut tui = ssh.attach("1");
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    stop_the_daemon(&daemon, dir.path());
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the Goodbye ends it");
    assert!(status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains("ShuttingDown"), "remote: {said:?}");
    assert_eq!(ssh.invocations(), 1, "and nothing tried again");
    daemon.wait_for_exit().expect("the daemon is gone");
}

/// **Review 1's Low 5, its `p18_refusal`, on every leg:** a daemon that refuses the
/// attach ends the route saying so, not status 0 with nothing said, on either route.
#[test]
fn p18_a_refusal_ends_the_route_saying_so_on_either_route() {
    let refusing = || {
        StandInDaemon::serve(|_, stream| {
            write_message(
                stream,
                &InstanceMessage::Goodbye(GoodbyeReason::AlreadyAttached),
            )
            .expect("Goodbye");
        })
    };
    let daemon = refusing();
    let home = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(&daemon.socket(), home.path(), &[]);
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the refusal ends it");
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains("AlreadyAttached"), "local: {said:?}");

    let daemon = refusing();
    let ssh = BridgedSsh::new(&daemon.socket());
    let mut tui = ssh.attach("1");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the refusal ends it");
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains("AlreadyAttached"), "remote: {said:?}");
    assert_eq!(daemon.connections(), 1, "and nothing tried again");
}

/// `p18_refusal` itself: a build with no CRDT admits one frontend, and a second TUI on a
/// real daemon reads the refusal.
#[cfg(not(feature = "crdt"))]
#[test]
fn p18_a_second_tui_on_a_build_with_no_crdt_reads_already_attached() {
    let daemon = TestDaemon::spawn();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut first = attached_tui(daemon.socket_path(), dir.path(), &[]);
    first.wait_for_output("*scratch*", Duration::from_secs(30));
    let mut second = attached_tui(daemon.socket_path(), dir.path(), &[]);
    let status = second
        .wait_for_exit(Duration::from_secs(20))
        .expect("the refusal ends it");
    assert!(!status.success(), "{status:?}");
    let said = second.after_the_alternate_screen();
    assert!(said.contains("AlreadyAttached"), "{said:?}");
    first.write_input(b"\x1b[24~").expect("F12");
    let _ = first.wait_for_exit(Duration::from_secs(10));
}

/// **Review 1's Low 6, the fourth row of its `p9_outlast`:** an `ssh` that outlasts the
/// kick's `SIGTERM`, here a wrapper not `exec`ing its bridge, with its daemon stopped so
/// the bridge never ends. `F12` ends the TUI within the bound the kick and the join state,
/// where at `5fd9d25` it sat in `join`, raw and painted, with nothing reading a key.
#[test]
fn p9_f12_ends_it_within_the_bound_past_an_ssh_that_outlasts_the_kick() {
    let daemon = TestDaemon::spawn();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("ssh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ntrap '' TERM\nPMACS_ATTACH_PROTOCOL_FD=2 '{pmacs}' --daemon-attach --socket '{socket}'\n",
            pmacs = env!("CARGO_BIN_EXE_pmacs"),
            socket = daemon.socket_path().display(),
        ),
    )
    .expect("write the stand-in");
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    let mut tui = common::pty::spawn_pmacs_in_pty(
        &["--attach", "farhost"],
        &[
            ("HOME", dir.path()),
            ("XDG_CONFIG_HOME", dir.path()),
            (PMACS_TEST_SSH_BIN, &script),
        ],
        24,
        80,
    );
    tui.wait_for_output("*scratch*", Duration::from_secs(30));
    let pid = nix::unistd::Pid::from_raw(i32::try_from(daemon.pid()).expect("pid"));
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGSTOP).expect("SIGSTOP");
    let pressed = Instant::now();
    tui.write_input(b"\x1b[24~").expect("F12");
    let ended = tui.wait_for_exit(Duration::from_secs(12));
    let took = pressed.elapsed();
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT).expect("SIGCONT");
    let status = ended.expect("F12 held by an ssh that outlasted its SIGTERM");
    assert!(status.success(), "{status:?}");
    assert!(
        took < Duration::from_secs(8),
        "F12 took {} ms, against a stated bound of about 5 s",
        took.as_millis()
    );
}

/// A real interactive shell in a PTY, with nothing of a user's in it, that a row types
/// into; `None` where it is not installed, which a CI runner's may not be.
#[cfg(target_os = "linux")]
fn interactive_shell(
    shell: &[&str],
    dir: &Path,
    envs: &[(&str, &Path)],
) -> Option<common::pty::PmacsPty> {
    let found = std::process::Command::new("sh")
        .args(["-c", &format!("command -v {}", shell[0])])
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipped: no {} on this machine", shell[0]);
        return None;
    }
    let mut all: Vec<(&str, &Path)> = vec![("HOME", dir), ("XDG_CONFIG_HOME", dir)];
    all.extend_from_slice(envs);
    Some(common::pty::spawn_shell_in_pty(shell, &all, 24, 80))
}

/// **Review 1's `p1_fg`, what it found holding, as guards:** a TUI stopped past the bound
/// and brought back by a real shell's `fg`, on either route: on a fresh view, its notice
/// saying why, and a key typed after that arrives. `fg` hands a job's modes back as the
/// shell has them, which differs by shell, so a row under `bash` is not this row. `fish`
/// is the owner's login shell.
#[cfg(target_os = "linux")]
fn fg_comes_back_under(shell: &[&str]) {
    let pmacs = env!("CARGO_BIN_EXE_pmacs");
    let local = |socket: &Path| format!("'{pmacs}' --attach --socket '{}'\r", socket.display());
    for remote in [false, true] {
        let daemon = TestDaemon::spawn_with_config(KEEPS_WRITING);
        let socket = daemon.socket_path();
        let ssh = BridgedSsh::new(socket);
        let script = ssh.dir.path().join("ssh");
        let dir = tempfile::TempDir::new().expect("tempdir");
        let Some(mut pane) = interactive_shell(shell, dir.path(), &[(PMACS_TEST_SSH_BIN, &script)])
        else {
            return;
        };
        let line = if remote {
            format!("'{pmacs}' --attach farhost\r")
        } else {
            local(socket)
        };
        pane.write_input(line.as_bytes()).expect("the command");
        pane.wait_for_output("*scratch*", Duration::from_secs(30));
        past_the_age_slack();
        stop_the_shells_child_until_dropped(&pane, &daemon);
        pane.write_input(b"fg\r").expect("fg");
        pane.wait_for_output("reconnected on a new view", Duration::from_secs(30));
        pane.write_input(b"\x1bx").expect("M-x");
        pane.wait_for_output("M-x", Duration::from_secs(10));
        pane.write_input(b"\x07").expect("C-g");
        pane.write_input(b"\x1b[24~").expect("F12");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn p1_fg_under_fish_comes_back_on_either_route() {
    fg_comes_back_under(&["fish", "--no-config", "--private"]);
}

#[cfg(target_os = "linux")]
#[test]
fn p1_fg_under_zsh_comes_back_on_either_route() {
    fg_comes_back_under(&["zsh", "-f", "-i"]);
}

/// **Review 1's Low 2, its `p8_stop`, as ruled:** a TUI stopped past the bound hears no
/// `Goodbye`, so after `pmacs --daemon --stop` its `fg` starts a new daemon that keeps
/// running over the remote route, saying so, and ends the local one, saying why. The owner
/// accepted this on 2026-11-06, so this pins what the documents state. A guard.
#[test]
fn p8_a_stop_a_stopped_tui_did_not_hear_starts_a_daemon_over_the_remote_route_alone() {
    let stopped_past_the_bound = |daemon: &TestDaemon, tui: &common::pty::PmacsPty| {
        let pid =
            nix::unistd::Pid::from_raw(i32::try_from(tui.process_id().expect("pid")).expect("pid"));
        tui.wait_for_output("*scratch*", Duration::from_secs(30));
        nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGSTOP).expect("SIGSTOP");
        let deadline = Instant::now() + Duration::from_mins(2);
        while !daemon_log(daemon).contains("sent no EOF within") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            daemon_log(daemon).contains("sent no EOF within"),
            "{}",
            daemon_log(daemon)
        );
        pid
    };

    let mut daemon = TestDaemon::spawn_with_config(KEEPS_WRITING);
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut tui = attached_tui(
        daemon.socket_path(),
        dir.path(),
        &[(PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1"))],
    );
    let pid = stopped_past_the_bound(&daemon, &tui);
    stop_the_daemon(&daemon, dir.path());
    daemon.wait_for_exit().expect("the daemon is gone");
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT).expect("SIGCONT");
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("the local route ends");
    assert!(!status.success(), "{status:?}");
    let said = tui.after_the_alternate_screen();
    assert!(said.contains("is gone"), "{said:?}");

    let mut daemon = TestDaemon::spawn_with_config(KEEPS_WRITING);
    let started = StartedByTheBridge {
        lockfile: daemon.lockfile_path(),
        _holder: None,
    };
    let first = started.pid().expect("the daemon's lockfile");
    let ssh = BridgedSsh::new(daemon.socket_path());
    let mut tui = ssh.attach("1");
    let pid = stopped_past_the_bound(&daemon, &tui);
    stop_the_daemon(&daemon, dir.path());
    daemon.wait_for_exit().expect("the daemon is gone");
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT).expect("SIGCONT");
    tui.wait_for_output(NEW_DAEMON_NOTICE, Duration::from_secs(30));
    tui.write_input(b"\x1b[24~").expect("F12");
    let status = tui
        .wait_for_exit(Duration::from_secs(10))
        .expect("F12 ends the session");
    assert!(status.success(), "{status:?}");
    let second = started.pid().expect("the new daemon's lockfile");
    assert_ne!(second, first, "a daemon of its own");
    assert!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(second), None).is_ok(),
        "it keeps running"
    );
}
