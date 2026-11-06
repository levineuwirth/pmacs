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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pmacs::attach::PMACS_TEST_SSH_BIN;
use pmacs::attach_reconnect::{HANDSHAKE_RETRY_CAP, PMACS_TEST_BACKOFF_SCALE_MS};
use pmacs::frontend_stream::FRAME_WRITE_BOUND;
use pmacs_protocol::message::{
    AttachRequest, GoodbyeReason, Hello, InstanceCapabilities, InstanceIdentity, InstanceMessage,
    PROTOCOL_VERSION,
};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{FRAME_CUT_SHORT_SENTENCE, FrontendId};

use common::daemon::TestDaemon;
use common::pty::attached_tui;

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
}

impl StandInDaemon {
    fn serve(session: impl Fn(usize, &mut UnixStream) + Send + 'static) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let listener = std::os::unix::net::UnixListener::bind(dir.path().join("d.sock"))
            .expect("bind the stand-in's socket");
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let n = counted.fetch_add(1, Ordering::SeqCst) + 1;
                let hello = Hello {
                    protocol_version: PROTOCOL_VERSION,
                    assigned_frontend_id: FrontendId(u64::try_from(n).expect("n")),
                    instance_identity: InstanceIdentity {
                        pmacs_version: "0.0.0-test".to_owned(),
                        build_hash: None,
                        instance_name: Some("stand-in".to_owned()),
                        uptime_secs: 0,
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
        Self { connections, dir }
    }

    fn socket(&self) -> std::path::PathBuf {
        self.dir.path().join("d.sock")
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
        let home = self.dir.path();
        let script = home.join("ssh");
        common::pty::spawn_pmacs_in_pty(
            &["--attach", "farhost"],
            &[
                ("HOME", home),
                ("XDG_CONFIG_HOME", home),
                (PMACS_TEST_SSH_BIN, &script),
                (PMACS_TEST_BACKOFF_SCALE_MS, Path::new(scale_ms)),
            ],
            24,
            80,
        )
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
                "#!/bin/sh\necho x >> '{count}'\nPMACS_ATTACH_PROTOCOL_FD=2 exec '{pmacs}' --daemon-attach --socket '{socket}'\n",
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

    /// A real TUI through the bridge, the backoff scaled to `scale_ms`.
    fn attach(&self, scale_ms: &str) -> common::pty::PmacsPty {
        let home = self.dir.path();
        let script = home.join("ssh");
        common::pty::spawn_pmacs_in_pty(
            &["--attach", "farhost"],
            &[
                ("HOME", home),
                ("XDG_CONFIG_HOME", home),
                (PMACS_TEST_SSH_BIN, &script),
                (PMACS_TEST_BACKOFF_SCALE_MS, Path::new(scale_ms)),
            ],
            24,
            80,
        )
    }
}

fn hello_frame() -> Vec<u8> {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &Hello {
            protocol_version: PROTOCOL_VERSION,
            assigned_frontend_id: FrontendId(2),
            instance_identity: InstanceIdentity {
                pmacs_version: "0.0.0-test".to_owned(),
                build_hash: None,
                instance_name: Some("stand-in".to_owned()),
                uptime_secs: 0,
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
#[cfg(target_os = "linux")]
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
