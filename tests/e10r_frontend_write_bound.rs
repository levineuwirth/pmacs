//! E10r --- a frontend that stops reading holds up nobody (#390).
//!
//! The daemon writes to every frontend from its one dispatcher thread, so
//! until E10r a frontend that stopped reading and kept its socket open
//! froze every other frontend for as long as it stayed stopped: nothing
//! bounded the write. A frame that is not whole within
//! `pmacs::frontend_stream::FRAME_WRITE_BOUND` is now a failed write, on
//! E10d's one path.
//!
//! Every row reads an interval on a frontend that kept reading: the
//! longest it waited for a key's answer while the other stopped
//! ([`Served`]). A frontend here is a socket with no reader, which is what
//! a stopped process is to the daemon; the real processes are at the end
//! of the file, stopped by `SIGSTOP`. The rows run on a real daemon, whose
//! idle ticks are what fills a stopped frontend's socket, in about a
//! second (#390's comment, and #457, which is why no row has to type to
//! reach the bound).
//!
//! An assertion allows [`SLACK`] past the bound, for a loaded machine; each
//! row prints what it read. At the base, with nothing bounding the write,
//! each of these waits out [`LIMIT`] instead, so the slack does not reach
//! the failure.

#![cfg(feature = "crdt")]

mod common;

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pmacs::daemon::WRITE_FAILURE_DRAIN;
use pmacs::frontend_stream::FRAME_WRITE_BOUND;
use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{InitialTarget, InstanceMessage, Key, Modifiers, PROTOCOL_VERSION};
use pmacs_protocol::{FrontendId, MAX_FRAME_BYTES};

use common::daemon::TestDaemon;
use common::gpu_probe::fact;
use common::wire::WireSession;

/// How far past the bound an interval may read before a row calls it
/// failed: a loaded machine's, not the product's.
const SLACK: Duration = Duration::from_secs(3);
/// How long a row serves a frontend before it stops looking: a stopped
/// frontend's fill, the bound, the drain and room for a loaded machine.
const LIMIT: Duration = Duration::from_secs(20);
const SIZE: CellSize = CellSize::new(24, 80);

fn daemon_log(daemon: &TestDaemon) -> String {
    std::fs::read_to_string(format!("{}.stderr.log", daemon.socket_path().display()))
        .unwrap_or_default()
}

/// The daemon's log lines saying a write to `fid` ran past the bound.
fn bound_failures(daemon: &TestDaemon, fid: FrontendId) -> usize {
    let who = format!("{fid:?} ");
    daemon_log(daemon)
        .lines()
        .filter(|line| line.contains(&who) && line.contains("was not written within"))
        .count()
}

fn torn_down(daemon: &TestDaemon, fid: FrontendId) -> bool {
    daemon_log(daemon).contains(&format!(
        "{fid:?} sent no EOF within {} ms of its failed write; tearing it down",
        WRITE_FAILURE_DRAIN.as_millis()
    ))
}

/// What a frontend that kept reading saw: the longest a key waited for its
/// answer, and how many keys were answered.
struct Served {
    longest_answer: Duration,
    answered: usize,
    unanswered_at_the_limit: bool,
}

/// One key at a time on `session`, each timed from its write to the
/// `CursorByte` that moves past it, until `done` holds at the end of an
/// answer or [`LIMIT`] passes. A dot starts no completion, so the answer
/// measures the dispatcher and not a provider.
fn serve_until(session: &mut WireSession, done: impl Fn() -> bool) -> Served {
    let (_, mut at) = session.where_am_i();
    let started = Instant::now();
    let mut served = Served {
        longest_answer: Duration::ZERO,
        answered: 0,
        unanswered_at_the_limit: false,
    };
    while started.elapsed() < LIMIT {
        session.key(Key::Char('.'), Modifiers::NONE);
        let sent = Instant::now();
        let mut answered = false;
        while started.elapsed() < LIMIT && !answered {
            match session.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(InstanceMessage::CursorByte { byte_pos, .. }) if byte_pos > at => {
                    at = byte_pos;
                    answered = true;
                }
                Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                // Its reader's read timed out, ten seconds with no frame:
                // the interval below says so.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        served.longest_answer = served.longest_answer.max(sent.elapsed());
        if !answered {
            // A key caught by the limit in the ordinary run of keys says
            // nothing; the interval above decides.
            served.unanswered_at_the_limit = sent.elapsed() > FRAME_WRITE_BOUND + SLACK;
            break;
        }
        served.answered += 1;
        if done() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    served
}

fn assert_held_up_by_at_most_the_bound(served: &Served, daemon: &TestDaemon, what: &str) {
    eprintln!(
        "{what}: longest answer {} ms over {} keys, bound {} ms",
        served.longest_answer.as_millis(),
        served.answered,
        FRAME_WRITE_BOUND.as_millis()
    );
    assert!(
        !served.unanswered_at_the_limit && served.longest_answer <= FRAME_WRITE_BOUND + SLACK,
        "{what}: the frontend that kept reading waited {} ms for a key's answer, against a {} ms \
         bound{}; log:\n{}",
        served.longest_answer.as_millis(),
        FRAME_WRITE_BOUND.as_millis(),
        if served.unanswered_at_the_limit {
            ", and never had it"
        } else {
            ""
        },
        daemon_log(daemon)
    );
}

/// `fixture_bytes` of text in a file under `dir`, lines of letters no
/// compressor shortens: a buffer's snapshot is compressed, and text that
/// shrinks would never fill a socket (a repeated line of 1 MiB made a
/// 21 KB bootstrap).
fn fixture(dir: &Path, fixture_bytes: usize) -> std::path::PathBuf {
    const LETTERS: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    let mut text = Vec::with_capacity(fixture_bytes);
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    while text.len() < fixture_bytes {
        for _ in 0..63 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            text.push(LETTERS[(state & 63) as usize]);
        }
        text.push(b'\n');
    }
    text.truncate(fixture_bytes);
    let path = dir.join("held.txt");
    std::fs::write(&path, text).expect("write the fixture");
    path
}

/// #390's own row. Two frontends; one stops reading and never closes. The
/// daemon writes past its buffer with nothing typed there. The other is
/// held up once, for at most the bound, and answers keys throughout; the
/// stopped one's write fails once, and it is torn down at the drain's
/// deadline and no sooner.
#[test]
fn e10r_i_a_frontend_that_stops_reading_holds_up_nobody() {
    let daemon = TestDaemon::spawn();
    let mut other = WireSession::grid(&daemon, SIZE);
    let stopped = WireSession::grid(&daemon, SIZE);
    let fid = stopped.fid;
    stopped.stop_reading();
    let served = serve_until(&mut other, || torn_down(&daemon, fid));
    assert_held_up_by_at_most_the_bound(&served, &daemon, "a frontend stopped while attached");
    assert_eq!(
        bound_failures(&daemon, fid),
        1,
        "its write failed once and nothing was written after it: {}",
        daemon_log(&daemon)
    );
    assert!(
        torn_down(&daemon, fid),
        "torn down at the drain's deadline: {}",
        daemon_log(&daemon)
    );
}

/// The same, the frontend stopping during its bootstrap: its snapshot is
/// larger than its socket holds, so the dispatcher's write to it waits
/// with no session registered and no tick to reach it. The other is held
/// up by at most the bound, the provisional session goes as a failed one
/// does, its socket shut, and the part of the frame already written is all
/// the frontend ever gets.
#[test]
fn e10r_ii_a_frontend_stopping_in_its_all_buffer_bootstrap_holds_up_nobody() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = fixture(dir.path(), 4 * 1024 * 1024);
    let daemon = TestDaemon::holding(&file);
    let mut other = WireSession::grid(&daemon, SIZE);
    let (mut stopped, fid) = WireSession::grid_unread(&daemon, SIZE);
    let failed = || bound_failures_named(&daemon, "send BufferSnapshot for") == 1;
    let served = serve_until(&mut other, failed);
    assert_held_up_by_at_most_the_bound(&served, &daemon, "a frontend stopped in its bootstrap");
    assert!(
        failed(),
        "the bootstrap's sweep failed at the bound: {}",
        daemon_log(&daemon)
    );
    assert_part_of_a_frame_then_eof(&mut stopped, fid, &daemon);
}

/// The same on a semantic session's bootstrap, whose target's snapshot is
/// a direct write of its own.
#[test]
fn e10r_ii_a_frontend_stopping_in_its_target_bootstrap_holds_up_nobody() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = fixture(dir.path(), 4 * 1024 * 1024);
    let daemon = TestDaemon::spawn();
    let mut other = WireSession::grid(&daemon, SIZE);
    let (mut stopped, fid) = WireSession::semantic_unread(
        &daemon,
        Some(InitialTarget {
            cwd: dir.path().to_str().expect("utf-8").as_bytes().to_vec(),
            path: file.to_str().expect("utf-8").as_bytes().to_vec(),
        }),
    );
    // Past the bound and a margin; reading to EOF below says the socket was shut.
    let started = Instant::now();
    let served = serve_until(&mut other, || started.elapsed() > FRAME_WRITE_BOUND * 2);
    assert_held_up_by_at_most_the_bound(&served, &daemon, "a frontend stopped in its target");
    assert_part_of_a_frame_then_eof(&mut stopped, fid, &daemon);
}

/// How many log lines hold both `needle` and the bound's own words.
fn bound_failures_named(daemon: &TestDaemon, needle: &str) -> usize {
    daemon_log(daemon)
        .lines()
        .filter(|line| line.contains(needle) && line.contains("was not written within"))
        .count()
}

/// What `stopped` still holds: fewer bytes than the frame its header
/// declares, then EOF, the daemon having shut the socket. The read timeout is
/// the attach's ten seconds: macOS refuses `SO_RCVTIMEO` on a shut socket.
fn assert_part_of_a_frame_then_eof(stopped: &mut UnixStream, fid: FrontendId, daemon: &TestDaemon) {
    let mut held = Vec::new();
    stopped.read_to_end(&mut held).unwrap_or_else(|error| {
        panic!(
            "{fid:?} should read to EOF, the daemon having shut its socket: {error}; log:\n{}",
            daemon_log(daemon)
        )
    });
    let mut at = 0;
    while at + 4 <= held.len() {
        let len = u32::from_be_bytes(held[at..at + 4].try_into().expect("four bytes")) as usize;
        if at + 4 + len > held.len() {
            eprintln!(
                "{fid:?} read {} bytes of a {len} byte frame, then EOF",
                held.len() - at - 4
            );
            return;
        }
        at += 4 + len;
    }
    panic!(
        "{fid:?} held {} bytes of whole frames: the bound cut no frame; log:\n{}",
        held.len(),
        daemon_log(daemon)
    );
}

/// A reader that trickles: it always makes enough room for one more wait,
/// so a timeout renewed by each partial write (`set_write_timeout`) would
/// hold the dispatcher for the whole frame, some twenty seconds here. The
/// bound is per frame, so it holds up the other for at most the bound.
#[test]
fn e10r_iii_a_reader_that_trickles_holds_up_nobody_past_the_bound() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = fixture(dir.path(), 6 * 1024 * 1024);
    let daemon = TestDaemon::holding(&file);
    let mut other = WireSession::grid(&daemon, SIZE);
    let (mut trickling, fid) = WireSession::grid_unread(&daemon, SIZE);
    let stop = Arc::new(AtomicBool::new(false));
    let reader_stop = Arc::clone(&stop);
    let reader = std::thread::spawn(move || {
        // 32 KiB every 50 ms: 640 KiB a second, so the snapshot, about twice
        // the text since it holds the state and the log, takes about 20 s.
        let mut chunk = vec![0u8; 32 * 1024];
        while !reader_stop.load(Ordering::SeqCst) {
            match trickling.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    });
    let failed = || bound_failures_named(&daemon, "send BufferSnapshot for") == 1;
    let served = serve_until(&mut other, failed);
    stop.store(true, Ordering::SeqCst);
    assert_held_up_by_at_most_the_bound(&served, &daemon, "a reader that trickles");
    assert!(
        failed(),
        "{fid:?}'s bootstrap failed at the bound: {}",
        daemon_log(&daemon)
    );
    reader.join().expect("reader");
}

/// The guard. A reader four times faster than the bound asks, at the
/// largest frame there is, is healthy: it gets the whole frame, nothing
/// fails, and it is still being served afterwards.
#[test]
fn e10r_iv_a_slow_healthy_reader_at_the_largest_frame_is_not_dropped() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    // A snapshot of text that does not compress is about twice the text,
    // so this one's frame is under the cap by about 2%.
    let file = fixture(dir.path(), 8_200_000);
    let daemon = TestDaemon::holding(&file);
    let (mut slow, fid) = WireSession::grid_unread(&daemon, SIZE);
    let longest = Arc::new(std::sync::Mutex::new((0usize, Duration::ZERO)));
    let reader_longest = Arc::clone(&longest);
    let reader = std::thread::spawn(move || {
        // One rate for every frame, a quarter of the bound for the largest
        // there is: the rule the bound is chosen by. Small frames go through
        // it at once.
        let bytes_per_second = MAX_FRAME_BYTES as f64 / (FRAME_WRITE_BOUND / 4).as_secs_f64();
        slow.set_read_timeout(Some(Duration::from_secs(30)))
            .expect("read timeout");
        let mut header = [0u8; 4];
        while slow.read_exact(&mut header).is_ok() {
            let len = u32::from_be_bytes(header) as usize;
            let started = Instant::now();
            let mut body = vec![0u8; len];
            let mut at = 0;
            while at < len {
                let end = (at + 256 * 1024).min(len);
                if slow.read_exact(&mut body[at..end]).is_err() {
                    return;
                }
                at = end;
                let due = Duration::from_secs_f64(at as f64 / bytes_per_second);
                if let Some(wait) = due.checked_sub(started.elapsed()) {
                    std::thread::sleep(wait);
                }
            }
            let mut best = reader_longest.lock().expect("lock");
            if len > best.0 {
                *best = (len, started.elapsed());
            }
        }
    });
    let deadline = Instant::now() + Duration::from_mins(2);
    while longest.lock().expect("lock").0 < MAX_FRAME_BYTES * 9 / 10 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let (largest, took) = *longest.lock().expect("lock");
    eprintln!(
        "the slow reader took a {largest} byte frame in {} ms",
        took.as_millis()
    );
    assert!(
        largest >= MAX_FRAME_BYTES * 9 / 10,
        "a frame near the largest never arrived whole, the largest {largest} bytes; log:\n{}",
        daemon_log(&daemon)
    );
    // Past a socket's fill and a bound, it is still attached.
    std::thread::sleep(FRAME_WRITE_BOUND * 2);
    assert_eq!(
        bound_failures(&daemon, fid) + bound_failures_named(&daemon, "send BufferSnapshot for"),
        0,
        "a healthy reader was dropped: {}",
        daemon_log(&daemon)
    );
    drop(reader);
}

/// E10r.3 --- dropped this way and resumed, a semantic frontend takes its
/// view back with its token, and a key typed after that arrives. The wire
/// route, which needs no adapter; the real `pmacs-gpu` is below.
#[test]
fn e10r_3_a_semantic_frontend_dropped_and_resumed_takes_back_its_view() {
    let daemon = TestDaemon::spawn();
    let mut other = WireSession::grid(&daemon, SIZE);
    let mut first = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    let token = first.token.expect("a semantic session is minted a token");
    let fid = first.fid;
    let (document, _) = first.where_am_i();
    first.declare_viewport(document);
    first.type_str("abc");
    first.cursor_at(document, 3);
    // Stop reading and never close.
    first.stop_reading();
    let served = serve_until(&mut other, || torn_down(&daemon, fid));
    assert_held_up_by_at_most_the_bound(&served, &daemon, "a semantic frontend stopped");
    assert_eq!(bound_failures(&daemon, fid), 1, "{}", daemon_log(&daemon));
    assert!(torn_down(&daemon, fid), "{}", daemon_log(&daemon));

    let mut back = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, Some(token));
    let (buffer, byte) = back.where_am_i();
    assert_eq!(
        (buffer, byte),
        (document, 3),
        "the token resumes the view; log:\n{}",
        daemon_log(&daemon)
    );
    back.declare_viewport(document);
    back.type_str("d");
    back.cursor_at(document, 4);
}

/// Stop `pid` and read that every one of its threads stopped, on Linux.
fn stop(pid: i32) {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGSTOP,
    )
    .expect("SIGSTOP");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match all_threads_stopped(pid) {
            Some(false) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Some(false) => panic!("{pid} did not stop"),
            Some(true) | None => return,
        }
    }
}

fn resume(pid: i32) {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGCONT,
    )
    .expect("SIGCONT");
}

/// Whether every thread of `pid` reads state `T`; `None` where `/proc` does
/// not say, as on macOS.
fn all_threads_stopped(pid: i32) -> Option<bool> {
    let tasks = std::fs::read_dir(format!("/proc/{pid}/task")).ok()?;
    let mut any = false;
    for task in tasks.flatten() {
        let stat = std::fs::read_to_string(task.path().join("stat")).ok()?;
        let state = stat.rsplit(')').next()?.split_whitespace().next()?;
        any = true;
        if state != "T" {
            return Some(false);
        }
    }
    Some(any)
}

/// The real `pmacs-gpu`, stopped by a signal past the bound and resumed: it
/// says what it lost, reconnects with its token, its view retained, and the
/// key it types after that arrives. Stopped as a child of this process, so
/// nothing but the signal touches it.
#[test]
fn e10r_3_the_real_gpu_stopped_past_the_bound_comes_back() {
    let Some(binary) = common::gpu_probe::gpu_probe_binary() else {
        return;
    };
    let daemon = TestDaemon::spawn();
    let mut other = WireSession::grid(&daemon, SIZE);
    let dir = tempfile::TempDir::new().expect("tempdir");
    let report = dir.path().join("reconnect.report");
    let mut ready = report.as_os_str().to_owned();
    ready.push(".ready");
    let ready = std::path::PathBuf::from(ready);
    let mut child = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", "gpu")
        .env("PMACS_GPU_PROBE_ACTION", "reconnect")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "120000")
        .env(pmacs::attach_reconnect::PMACS_TEST_BACKOFF_SCALE_MS, "20")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the headless GPU probe");
    let deadline = Instant::now() + Duration::from_mins(1);
    while !ready.exists() && Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !ready.exists() {
        let _ = child.kill();
        let output = child.wait_with_output().expect("wait for the probe");
        // No adapter (status 3), as on a runner with no GPU: skipped, which
        // `probe_facts` turns into a panic under `PMACS_REQUIRE_GPU`.
        assert!(
            common::gpu_probe::probe_facts(&output, &report, "before it typed").is_none(),
            "the probe never typed; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let pid = i32::try_from(child.id()).expect("pid");
    stop(pid);
    let dropped = || daemon_log(&daemon).contains("sent no EOF within");
    let served = serve_until(&mut other, dropped);
    assert_eq!(
        all_threads_stopped(pid),
        all_threads_stopped(pid).map(|_| true),
        "the probe stayed stopped throughout"
    );
    assert_held_up_by_at_most_the_bound(&served, &daemon, "the real GPU stopped");
    assert!(dropped(), "{}", daemon_log(&daemon));
    resume(pid);
    let output = child.wait_with_output().expect("wait for the probe");
    let facts = common::gpu_probe::probe_facts(&output, &report, "after a stop")
        .expect("the probe reports");
    assert_eq!(fact(&facts, "lost"), "true", "{facts:?}");
    assert_eq!(fact(&facts, "reconnected"), "true", "{facts:?}");
    assert_eq!(
        fact(&facts, "buffer.after"),
        fact(&facts, "buffer.before"),
        "{facts:?}"
    );
    assert!(
        fact(&facts, "text.closing").contains('!'),
        "the key typed after the reconnect arrived: {facts:?}"
    );
}

/// What the attached TUI does today, stated as a row so that it cannot
/// change unseen: dropped this way and resumed, it reads what is left and
/// EOF, and exits at once with its terminal restored. It does not hang, and
/// it does not come back: a grid session has no token (`Resume` is read only
/// from a semantic session, `src/daemon.rs`), and the local `pmacs --attach`
/// has no reconnect loop (`run_attach` beside `run_attach_ssh`).
#[test]
fn e10r_3_the_attached_tui_stopped_past_the_bound_exits_when_resumed() {
    let daemon = TestDaemon::spawn();
    let mut other = WireSession::grid(&daemon, SIZE);
    let dir = tempfile::TempDir::new().expect("tempdir");
    let home = dir.path().to_path_buf();
    let socket = daemon.socket_path().to_str().expect("utf-8").to_owned();
    let mut tui = common::pty::spawn_pmacs_in_pty(
        &["--attach", "--socket", &socket],
        &[("HOME", &home), ("XDG_CONFIG_HOME", &home)],
        24,
        80,
    );
    let pid = i32::try_from(tui.process_id().expect("pid")).expect("pid");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !String::from_utf8_lossy(&tui.output()).contains("*scratch*") && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    stop(pid);
    let dropped = || daemon_log(&daemon).contains("sent no EOF within");
    let served = serve_until(&mut other, dropped);
    assert_held_up_by_at_most_the_bound(&served, &daemon, "the attached TUI stopped");
    assert!(dropped(), "{}", daemon_log(&daemon));
    resume(pid);
    let status = tui
        .wait_for_exit(Duration::from_secs(20))
        .expect("a resumed TUI exits rather than hangs");
    assert!(status.success(), "{status:?}");
    let output = tui.output();
    assert!(
        output.windows(8).any(|w| w == b"\x1b[?1049l"),
        "it left the alternate screen on its way out"
    );
    // The daemon and the other frontend are none the worse for it.
    other.key(Key::Char('.'), Modifiers::NONE);
    other.where_am_i();
}

/// A paste larger than the socket's buffer through the attached TUI reaches
/// the daemon, and the TUI stays. Until E10r the TUI's reader made the socket
/// non-blocking for a poll loop, `O_NONBLOCK` is shared across clones, and so
/// the paste's write failed with `EAGAIN` and ended the session (at 300 KB;
/// 150 KB was fine).
#[test]
fn e10r_a_paste_past_the_socket_buffer_through_the_attached_tui_arrives() {
    let daemon = TestDaemon::spawn();
    let mut other = WireSession::grid(&daemon, SIZE);
    let dir = tempfile::TempDir::new().expect("tempdir");
    let home = dir.path().to_path_buf();
    let socket = daemon.socket_path().to_str().expect("utf-8").to_owned();
    let mut tui = common::pty::spawn_pmacs_in_pty(
        &["--attach", "--socket", &socket],
        &[("HOME", &home), ("XDG_CONFIG_HOME", &home)],
        24,
        80,
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while !String::from_utf8_lossy(&tui.output()).contains("*scratch*") && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let bytes = 1024 * 1024;
    let mut paste = b"\x1b[200~".to_vec();
    paste.extend(std::iter::repeat_n(b'x', bytes));
    paste.extend_from_slice(b"\x1b[201~");
    tui.write_input(&paste).expect("paste into the pty");
    other.wait("the paste's op", |m| match m {
        InstanceMessage::CrdtOp { op, .. } => (op.bytes.len() >= bytes).then_some(()),
        _ => None,
    });
    assert!(
        tui.wait_for_exit(Duration::from_millis(200)).is_none(),
        "the TUI is still attached; it wrote {:?}",
        String::from_utf8_lossy(&tui.output())
            .chars()
            .rev()
            .take(200)
            .collect::<String>()
    );
}
