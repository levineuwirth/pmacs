//! E9 final review: loss reaches the two ends of a forwarded socket at
//! different times. The production GPU reconnects after 500 ms. A live
//! token must not become a successful fresh attach that abandons its view.
#![cfg(feature = "crdt")]

mod common;

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use common::daemon::TestDaemon;

struct LossProxy {
    path: PathBuf,
    held: Arc<Mutex<Vec<(UnixStream, UnixStream)>>>,
    daemon_loss_sent: Arc<AtomicBool>,
}

impl LossProxy {
    fn start(target: &Path, dir: &Path, eof_delay: Duration) -> Self {
        let path = dir.join("proxy.sock");
        let listener = UnixListener::bind(&path).expect("proxy bind");
        let target = target.to_path_buf();
        let held = Arc::new(Mutex::new(Vec::new()));
        let keep = Arc::clone(&held);
        let daemon_loss_sent = Arc::new(AtomicBool::new(false));
        let loss_sent = Arc::clone(&daemon_loss_sent);
        std::thread::spawn(move || {
            for (index, accepted) in listener.incoming().enumerate() {
                let Ok(front) = accepted else { break };
                let Ok(back) = UnixStream::connect(&target) else {
                    break;
                };
                let mut front_read = front.try_clone().expect("clone");
                let mut front_write = front.try_clone().expect("clone");
                let mut back_read = back.try_clone().expect("clone");
                let mut back_write = back.try_clone().expect("clone");
                keep.lock().expect("held").push((front, back));
                let sent = Arc::clone(&loss_sent);
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut front_read, &mut back_write);
                    if index == 0 {
                        std::thread::sleep(eof_delay);
                        sent.store(true, Ordering::SeqCst);
                    }
                    let _ = back_write.shutdown(std::net::Shutdown::Write);
                });
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut back_read, &mut front_write);
                    let _ = front_write.shutdown(std::net::Shutdown::Write);
                });
            }
        });
        Self {
            path,
            held,
            daemon_loss_sent,
        }
    }

    fn cut_front(&self) {
        let held = self.held.lock().expect("held");
        assert_eq!(held.len(), 1, "one established connection before the cut");
        held[0]
            .0
            .shutdown(std::net::Shutdown::Both)
            .expect("cut front");
    }
}

impl Drop for LossProxy {
    fn drop(&mut self) {
        for (front, back) in self.held.lock().expect("held").drain(..) {
            let _ = front.shutdown(std::net::Shutdown::Both);
            let _ = back.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn reconnect_case(eof_delay: Duration) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("chmod");
    let note = dir.path().join("note.txt");
    std::fs::write(&note, "one\ntwo\n").expect("write note");
    let init = format!(
        "pmacs.command.define {{ name = 'e9.open', description = 'open note', \
         fn = function() pmacs.buffer.find_or_open({:?}) end }}\n",
        note.to_str().expect("utf8")
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], &init);
    let proxy = LossProxy::start(daemon.socket_path(), dir.path(), eof_delay);
    let binary = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("binary directory")
        .join("pmacs-gpu");
    if !binary.exists() {
        assert!(
            std::env::var_os("PMACS_REQUIRE_GPU").is_none(),
            "pmacs-gpu must be built"
        );
        eprintln!("skipping: pmacs-gpu is absent");
        return;
    }
    let report = dir.path().join("reconnect.report");
    let ready = dir.path().join("reconnect.report.ready");
    let mut child = std::process::Command::new(binary)
        .arg("--headless-probe")
        .arg(&proxy.path)
        .arg(&report)
        .env("PMACS_GPU_PROBE_ACTION", "reconnect")
        .env("PMACS_GPU_PROBE_TYPE_TEXT", "abc")
        .env("PMACS_GPU_PROBE_COMMAND", "e9.open")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "30000")
        // Use the production half-second first delay, not the test curve.
        .env_remove(pmacs::attach_reconnect::PMACS_TEST_BACKOFF_SCALE_MS)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("GPU probe");
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut cut = false;
    loop {
        if !cut && ready.exists() {
            proxy.cut_front();
            cut = true;
        }
        if child.try_wait().expect("poll GPU").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed-out GPU");
            let output = child.wait_with_output().expect("reap GPU");
            panic!(
                "probe deadline: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("reap GPU");
    if output.status.code() == Some(3) && std::env::var_os("PMACS_REQUIRE_GPU").is_none() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    assert!(
        output.status.success(),
        "GPU probe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(cut, "the probe must establish its view before the cut");
    let text = std::fs::read_to_string(&report).expect("report");
    let facts: std::collections::HashMap<_, _> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    eprintln!(
        "delay {eof_delay:?}; daemon EOF sent={}; report:\n{text}",
        proxy.daemon_loss_sent.load(Ordering::SeqCst)
    );
    assert_eq!(facts["phase.done"], "true");
    assert_eq!(
        facts["cursor.before"], "3",
        "precondition: typed at the note's start"
    );
    assert_eq!(facts["text.before"], r#""abcone\ntwo\n""#);
    assert_eq!(facts["lost"], "true");
    assert_eq!(facts["reconnected"], "true");
    // These assert the production App's accepted session, without a
    // test-only retry until the expected view happens to reappear.
    assert_eq!(
        facts["buffer.after"], facts["buffer.before"],
        "a reconnect preceding the daemon's EOF must preserve the view: {text}"
    );
    assert_eq!(facts["cursor.after"], facts["cursor.before"], "{text}");
    assert_eq!(
        facts["text.closing"], r#""abc!one\ntwo\n""#,
        "the next key must reach the resumed document: {text}"
    );
}

#[test]
fn e9_review3_immediate_daemon_eof_resumes_the_gpu_view() {
    reconnect_case(Duration::ZERO);
}

/// Fails at f991f7b: the GUI reconnects before the daemon sees EOF,
/// accepts a fresh view, replaces its token and loses its buffer or cursor.
/// The old session is merely still live, not expired or from a restart.
#[test]
fn e9_review3_delayed_daemon_eof_must_not_be_a_successful_fresh_view() {
    reconnect_case(Duration::from_secs(2));
}
