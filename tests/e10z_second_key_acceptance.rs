//! E10z.2 --- the second key stays a key (#470): once a frontend has
//! forwarded an event the daemon must answer, nothing it types goes as a
//! `CrdtOp` until the daemon has answered it, which since E10z.1 the daemon
//! says on the wire (`InstanceMessage::InputHandled`).
//!
//! Every row drives a real frontend against a real daemon through a proxy
//! on the socket, [`Proxy`], E10u.1 (c)'s control written into the suite:
//! the daemon's bytes pass at once (or a stated latency later), and each
//! event the frontend sends is decoded, recorded with what it is (a `Key`,
//! or text applied as a `CrdtOp`) and passed on, at once, a stated latency
//! later, or held, from the event a row names, for a stated time. The
//! record is the seam the row reads: what the frontend decided a key was.
//! Neither binary is changed by the proxy.
//!
//! Witnesses fail at E10z's base (`d8fd793`), each by #470's mechanism.
//! Guards pass there and after: the fix must not be to round-trip
//! everything, strand text after a key that flips nothing, or reorder a
//! word typed straight after a chord.
#![cfg(feature = "crdt")]

mod common;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pmacs_protocol::message::{AttachRequest, FrontendEvent, Key, Modifiers, MouseKind};

use common::daemon::{NO_REOPEN, TestDaemon};
use common::gpu_probe::{fact, gpu_probe_binary, probe_facts};
use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

const ROWS: u16 = 24;
const COLS: u16 = 100;
/// How long a row waits for anything it expects to come.
const PATIENCE: Duration = Duration::from_secs(15);
/// How long a held read is held: E10u.1 (c)'s control.
const HOLD: Duration = Duration::from_millis(300);

/// What the frontend sent, as the proxy decoded it.
#[derive(Clone, Debug, PartialEq)]
enum Sent {
    Key(Key, Modifiers),
    /// Text the frontend applied to its own replica and sent as an op.
    Op,
    Mouse,
    Other(String),
}

fn sent_of(event: &FrontendEvent) -> Sent {
    match event {
        FrontendEvent::Key(k) => Sent::Key(k.key, k.mods),
        FrontendEvent::CrdtOp { .. } => Sent::Op,
        FrontendEvent::Mouse(_) | FrontendEvent::Pointer { .. } => Sent::Mouse,
        other => Sent::Other(
            format!("{other:?}")
                .split(['(', ' ', '{'])
                .next()
                .unwrap_or("")
                .to_owned(),
        ),
    }
}

type Trigger = Box<dyn Fn(&FrontendEvent) -> bool + Send>;

#[derive(Default)]
struct ProxyState {
    sent: Vec<(Instant, Sent)>,
    /// The next event this accepts holds it and every event after it
    /// until `.1` has passed from its read; one-shot.
    arm: Option<(Trigger, Duration)>,
    hold_until: Option<Instant>,
}

/// A socket between one frontend and the daemon (see the module's note).
struct Proxy {
    path: PathBuf,
    state: Arc<Mutex<ProxyState>>,
}

/// Read one length-prefixed frame, its prefix included, or `None` at EOF.
fn read_frame(from: &mut impl Read) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    from.read_exact(&mut len).ok()?;
    let n = u32::from_be_bytes(len) as usize;
    let mut frame = len.to_vec();
    frame.resize(4 + n, 0);
    from.read_exact(&mut frame[4..]).ok()?;
    Some(frame)
}

/// Carry what `rx` delivers to `to`, each piece no earlier than its
/// release instant, in order; at the end shut both sockets.
fn deliver(rx: &mpsc::Receiver<(Instant, Vec<u8>)>, mut to: UnixStream, other: &UnixStream) {
    while let Ok((release, bytes)) = rx.recv() {
        let now = Instant::now();
        if release > now {
            std::thread::sleep(release - now);
        }
        if to.write_all(&bytes).is_err() {
            break;
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Both);
    let _ = other.shutdown(std::net::Shutdown::Both);
}

impl Proxy {
    /// Listen beside `daemon`'s socket and connect each frontend that
    /// attaches to the daemon, each direction `latency` late.
    fn start(daemon: &TestDaemon, latency: Duration) -> Self {
        let path = daemon
            .socket_path()
            .parent()
            .expect("socket dir")
            .join("p.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind the proxy");
        let target = daemon.socket_path().to_path_buf();
        let state = Arc::new(Mutex::new(ProxyState::default()));
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for front in listener.incoming() {
                let Ok(front) = front else { return };
                let Ok(back) = UnixStream::connect(&target) else {
                    return;
                };
                Self::carry(front, back, latency, Arc::clone(&shared));
            }
        });
        Self { path, state }
    }

    fn carry(
        front: UnixStream,
        back: UnixStream,
        latency: Duration,
        state: Arc<Mutex<ProxyState>>,
    ) {
        // The daemon to the frontend: raw bytes, `latency` late.
        let (down_tx, down_rx) = mpsc::channel();
        let mut back_read = back.try_clone().expect("clone");
        let front_write = front.try_clone().expect("clone");
        let back_shut = back.try_clone().expect("clone");
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 1 << 16];
            while let Ok(n) = back_read.read(&mut buf) {
                if n == 0
                    || down_tx
                        .send((Instant::now() + latency, buf[..n].to_vec()))
                        .is_err()
                {
                    break;
                }
            }
        });
        std::thread::spawn(move || deliver(&down_rx, front_write, &back_shut));

        // The frontend to the daemon: frame by frame, each event decoded
        // and recorded, released `latency` late or at the hold's end.
        let (up_tx, up_rx) = mpsc::channel();
        let mut front_read = front.try_clone().expect("clone");
        let front_shut = front;
        std::thread::spawn(move || {
            let mut index = 0usize;
            let mut semantic = false;
            let mut last_release = Instant::now();
            while let Some(frame) = read_frame(&mut front_read) {
                let read_at = Instant::now();
                let mut release = read_at + latency;
                if index == 0 {
                    // The handshake's `AttachRequest`; a semantic session
                    // sends its bootstrap request next, which is no event.
                    semantic = postcard::from_bytes::<AttachRequest>(&frame[4..])
                        .is_ok_and(|req| req.frontend_capabilities.semantic_render);
                } else if !(semantic && index == 1)
                    && let Ok(event) = postcard::from_bytes::<FrontendEvent>(&frame[4..])
                {
                    let mut st = state.lock().expect("proxy state");
                    if st.arm.as_ref().is_some_and(|(trigger, _)| trigger(&event)) {
                        let (_, hold) = st.arm.take().expect("armed");
                        st.hold_until = Some(read_at + hold);
                    }
                    if let Some(until) = st.hold_until {
                        release = release.max(until);
                    }
                    st.sent.push((read_at, sent_of(&event)));
                }
                index += 1;
                release = release.max(last_release);
                last_release = release;
                if up_tx.send((release, frame)).is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || deliver(&up_rx, back, &front_shut));
    }

    /// Hold the next event `trigger` accepts, and all after it, for `hold`.
    fn hold_from(&self, trigger: impl Fn(&FrontendEvent) -> bool + Send + 'static, hold: Duration) {
        let mut st = self.state.lock().expect("proxy state");
        st.arm = Some((Box::new(trigger), hold));
        st.hold_until = None;
    }

    /// How many events the frontend has sent so far: a mark for [`Self::since`].
    fn mark(&self) -> usize {
        self.state.lock().expect("proxy state").sent.len()
    }

    /// What the frontend sent after `mark`.
    fn since(&self, mark: usize) -> Vec<Sent> {
        self.state.lock().expect("proxy state").sent[mark..]
            .iter()
            .map(|(_, s)| s.clone())
            .collect()
    }

    /// Wait until the frontend has sent `n` events after `mark`.
    fn wait_sent(&self, mark: usize, n: usize, what: &str) -> Vec<Sent> {
        let start = Instant::now();
        loop {
            let got = self.since(mark);
            if got.len() >= n {
                return got;
            }
            assert!(
                start.elapsed() < PATIENCE,
                "{what}: the frontend sent {got:?} after the mark, waiting for {n} events"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn ctrl(c: char) -> Sent {
    Sent::Key(Key::Char(c), Modifiers::CTRL)
}
fn plain(c: char) -> Sent {
    Sent::Key(Key::Char(c), Modifiers::NONE)
}

/// A daemon with no language servers and no desktop to reopen.
fn daemon() -> TestDaemon {
    TestDaemon::spawn_with_config(&format!("{NO_REOPEN}pmacs.lsp.config = {{}}\n"))
}

/// A real `pmacs --attach` in a PTY on `socket` (the proxy's), or through
/// the `ssh` stand-in when `remote` names one, its screen modelled, on
/// `*scratch*` with the caret on a fresh empty last line, where a typed
/// character is the optimistic path's.
fn tui(socket: &Path, home: &Path, remote: Option<&Path>) -> ScreenTui {
    let socket = socket.to_str().expect("utf-8").to_owned();
    let (args, envs): (Vec<&str>, Vec<(&str, &Path)>) = match remote {
        None => (
            vec!["--attach", "--socket", &socket],
            vec![("HOME", home), ("XDG_CONFIG_HOME", home)],
        ),
        Some(script) => (
            vec!["--attach", "farhost"],
            vec![
                ("HOME", home),
                ("XDG_CONFIG_HOME", home),
                (pmacs::attach::PMACS_TEST_SSH_BIN, script),
            ],
        ),
    };
    let (pty, tap) = spawn_pmacs_in_pty_tapped(&args, &envs, ROWS, COLS);
    let mut tui = ScreenTui::new(pty, tap, ROWS, COLS);
    tui.wait_for(PATIENCE, "the attached *scratch*", &|t| {
        t.screen_contains("*scratch*")
    });
    tui.write(b"\x1b>");
    std::thread::sleep(Duration::from_millis(300));
    tui.write(b"\r");
    tui.settle(
        Duration::from_millis(400),
        PATIENCE,
        "the fresh last line",
        &|_| true,
    );
    tui
}

/// The `ssh` stand-in: the real far-side bridge, `--daemon-attach`, on
/// the proxy's socket, so the proxy's latency is the link's.
fn stand_in(dir: &Path, proxy: &Path) -> PathBuf {
    let script = dir.join("ssh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nPMACS_ATTACH_PROTOCOL_FD=2 exec '{}' --daemon-attach --socket '{}'\n",
            env!("CARGO_BIN_EXE_pmacs"),
            proxy.display()
        ),
    )
    .expect("write the stand-in");
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    script
}

/// The buffer's own rows: everything above the modeline.
fn buffer_text(tui: &ScreenTui) -> String {
    tui.rows()
        .into_iter()
        .take_while(|row| !row.contains("*scratch*"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count(tui: &ScreenTui, c: char) -> usize {
    buffer_text(tui).chars().filter(|x| *x == c).count()
}

/// `C-g`, twice, and wait for the screen to settle on the buffer again.
fn cancel(tui: &mut ScreenTui, gone: &str) {
    tui.write(b"\x07");
    std::thread::sleep(Duration::from_millis(100));
    tui.write(b"\x07");
    tui.settle(
        Duration::from_millis(250),
        PATIENCE,
        "the prompt gone",
        &|t| !t.screen_contains(gone),
    );
}

/// A small generator for the random phase before a trial, seeded from the
/// clock: a trial that starts a fixed time after the last locks to the
/// pump's 50 ms poll and reads nothing on a quiet machine (E10u.1 (a)).
struct Phase(u64);
impl Phase {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Self(seed | 1)
    }
    fn wait(&mut self, ms: u64) {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        std::thread::sleep(Duration::from_micros(self.0 % (ms * 1000)));
    }
}

/// One `C-x` then `b` `gap` after it: the frontend's record of the two,
/// and whether the switch-buffer prompt came.
fn c_x_b(tui: &mut ScreenTui, proxy: &Proxy, gap: Option<Duration>) -> (Vec<Sent>, bool) {
    let mark = proxy.mark();
    match gap {
        None => tui.write(b"\x18b"),
        Some(gap) => {
            tui.write(b"\x18");
            std::thread::sleep(gap);
            tui.write(b"b");
        }
    }
    let sent = proxy.wait_sent(mark, 2, "C-x b");
    let prompt = tui
        .watch_for(Duration::from_secs(3), &|t| {
            t.screen_contains("Switch to buffer")
        })
        .is_some();
    (sent[..2].to_vec(), prompt)
}

/// E10z.2 (i): with the daemon's read of `C-x` held 300 ms by the proxy,
/// `b` typed 50 ms after it goes as a key, the prompt opens, and no `b`
/// reaches the buffer. At the base the `CursorByte`s the daemon's ticks
/// write while the read is held re-ground the cursor, and `b` goes as
/// text, 20 of 20 in E10u.1 (c).
#[test]
fn e10z_2_i_with_the_daemons_read_held_the_second_key_stays_a_key() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    for trial in 0..3 {
        let before = count(&tui, 'b');
        proxy.hold_from(
            |e| matches!(e, FrontendEvent::Key(k) if k.key == Key::Char('x') && k.mods == Modifiers::CTRL),
            HOLD,
        );
        let (sent, prompt) = c_x_b(&mut tui, &proxy, Some(Duration::from_millis(50)));
        assert_eq!(
            sent,
            vec![ctrl('x'), plain('b')],
            "trial {trial}: `b` typed while the daemon had not read `C-x` must go as a key"
        );
        assert!(
            prompt,
            "trial {trial}: the prompt opens; {}",
            tui.describe()
        );
        cancel(&mut tui, "Switch to buffer");
        assert_eq!(
            count(&tui, 'b'),
            before,
            "trial {trial}: no `b` in the buffer"
        );
    }
}

/// E10z.2 (ii): 20 trials with no gap and 20 at 30 ms, each after a random
/// 0 to 60 ms, on a local socket with nothing held: the prompt every time
/// and no `b`, so that no trial reads "neither". At the base about three
/// in four went as text (E10z.3's "before", the random phase: 16 and 14 of
/// 20), by the pump's own mechanism, a `DispatchIdle` written and not yet
/// read when the key was decided.
#[test]
fn e10z_2_ii_a_fast_chord_on_a_local_socket_opens_its_prompt_every_time() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    let mut phase = Phase::new();
    let mut wrong = Vec::new();
    for (label, gap) in [("no gap", None), ("30 ms", Some(Duration::from_millis(30)))] {
        for trial in 0..20 {
            let before = count(&tui, 'b');
            phase.wait(60);
            let (sent, prompt) = c_x_b(&mut tui, &proxy, gap);
            if prompt {
                cancel(&mut tui, "Switch to buffer");
            } else {
                tui.settle(
                    Duration::from_millis(250),
                    PATIENCE,
                    "after a trial",
                    &|_| true,
                );
            }
            let stray = count(&tui, 'b').saturating_sub(before);
            if sent != vec![ctrl('x'), plain('b')] || !prompt || stray > 0 {
                wrong.push(format!(
                    "{label} #{trial}: sent {sent:?}, prompt {prompt}, {stray} b"
                ));
                if stray > 0 {
                    tui.write(b"\x7f");
                    tui.settle(
                        Duration::from_millis(250),
                        PATIENCE,
                        "the b taken back",
                        &|_| true,
                    );
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of 40 trials went wrong: {wrong:#?}",
        wrong.len()
    );
}

/// E10z.2 (iii), the chords and prompt keys E10u.1 (e) found it on, with
/// the daemon's read of the first key held: `C-x k`, `M-g g`, and a letter
/// typed straight after `M-x`, `C-s` and `M-%`, each second key a key. At
/// the base each went as text in 16 to 17 of 20 at a 30 ms gap.
#[test]
fn e10z_2_iii_every_prefix_and_prompt_key_holds_the_next_key() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    // (keys, the first key as sent, the second as sent, what to cancel, a needle the prompt shows)
    let cases: [(&[u8], Sent, Sent, &str); 5] = [
        (b"\x18", ctrl('x'), plain('k'), "Kill buffer"),
        (
            b"\x1bg",
            Sent::Key(Key::Char('g'), Modifiers::ALT),
            plain('g'),
            "oto line",
        ),
        (
            b"\x1bx",
            Sent::Key(Key::Char('x'), Modifiers::ALT),
            plain('q'),
            "M-x",
        ),
        (b"\x13", ctrl('s'), plain('q'), "I-search"),
        (
            b"\x1b%",
            Sent::Key(Key::Char('%'), Modifiers::ALT),
            plain('q'),
            "Query replace",
        ),
    ];
    for (first, first_sent, second, needle) in cases {
        let Sent::Key(Key::Char(second_char), _) = second else {
            unreachable!()
        };
        let first_key = first_sent.clone();
        proxy.hold_from(move |e| sent_of(e) == first_key, HOLD);
        let mark = proxy.mark();
        tui.write(first);
        std::thread::sleep(Duration::from_millis(50));
        tui.write(second_char.to_string().as_bytes());
        let sent = proxy.wait_sent(mark, 2, needle);
        assert_eq!(
            sent[..2].to_vec(),
            vec![first_sent.clone(), second.clone()],
            "after {first_sent:?}, the next key must go as a key"
        );
        tui.wait_for(PATIENCE, needle, &|t| t.screen_contains(needle));
        cancel(&mut tui, needle);
    }
}

/// E10z.2 (iii), the remote route, at the owner's hands: `C-x`, then `b`
/// 150 ms later, over the `ssh` stand-in with 100 ms each way, a round
/// trip past the gap. E10u's addition measured 112 ms gaps over a 120 ms
/// round trip at 20 of 20 taken as text at the base, as the owner's least
/// gap; with no hold at all, the link alone is what is late.
#[test]
fn e10z_2_iii_over_a_link_slower_than_the_hands_the_second_key_stays_a_key() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let mut phase = Phase::new();
    for trial in 0..5 {
        let before = count(&tui, 'b');
        phase.wait(60);
        let (sent, prompt) = c_x_b(&mut tui, &proxy, Some(Duration::from_millis(150)));
        assert_eq!(
            sent,
            vec![ctrl('x'), plain('b')],
            "trial {trial}: over the link `b` must go as a key"
        );
        assert!(
            prompt,
            "trial {trial}: the prompt opens; {}",
            tui.describe()
        );
        cancel(&mut tui, "Switch to buffer");
        assert_eq!(
            count(&tui, 'b'),
            before,
            "trial {trial}: no `b` in the buffer"
        );
    }
}

/// E10z.2 (iii) and the ruling on the count (D47): a character typed after
/// a click, with the daemon's read of the click held, goes as a key and
/// lands where the click put the caret. At the base it went as text at
/// the cursor from before the click, 20 of 20 (C10z.1's measurement).
#[test]
fn e10z_2_a_character_after_a_held_click_lands_where_the_click_put_the_caret() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    tui.write(b"ab");
    tui.wait_for(PATIENCE, "`ab` typed", &|t| {
        buffer_text(t)
            .lines()
            .any(|l| l.trim_end().ends_with(" ab"))
    });
    tui.settle(Duration::from_millis(300), PATIENCE, "after `ab`", &|_| {
        true
    });
    let rows = tui.rows();
    let (row, line) = rows
        .iter()
        .enumerate()
        .find(|(_, l)| l.trim_end().ends_with(" ab"))
        .expect("the line holding `ab`");
    let col = line.find("ab").expect("ab");
    proxy.hold_from(
        |e| matches!(e, FrontendEvent::Mouse(m) if matches!(m.kind, MouseKind::Down(_))),
        HOLD,
    );
    let mark = proxy.mark();
    // SGR mouse: a left press and release on `a`, one-based.
    tui.write(format!("\x1b[<0;{c};{r}M\x1b[<0;{c};{r}m", c = col + 1, r = row + 1).as_bytes());
    std::thread::sleep(Duration::from_millis(50));
    tui.write(b"x");
    let sent = proxy.wait_sent(mark, 3, "the click and `x`");
    assert_eq!(
        sent.last().cloned(),
        Some(plain('x')),
        "`x` typed while the daemon had not read the click must go as a key: {sent:?}"
    );
    tui.wait_for(PATIENCE, "`xab`", &|t| {
        buffer_text(t)
            .lines()
            .any(|l| l.trim_end().ends_with(" xab"))
    });
}

/// E10z.2 (iii), `pmacs-gpu`, through `--headless-probe`: `C-x`, then `b`
/// 60 ms later, the daemon's read of `C-x` held 300 ms by the proxy. The
/// window sends `b` as a key and its minibuffer opens. At the base it sent
/// `b` as an op, 20 of 20 (E10u.1 (e)).
#[test]
fn e10z_2_iii_the_gpu_window_holds_the_second_key_too() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let Some(binary) = gpu_probe_binary() else {
        return;
    };
    proxy.hold_from(
        |e| matches!(e, FrontendEvent::Key(k) if k.key == Key::Char('x') && k.mods == Modifiers::CTRL),
        HOLD,
    );
    let report = proxy.path.with_file_name("gpu.report");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(&proxy.path)
        .arg(&report)
        .env(
            "PMACS_GPU_PROBE_TYPE_TEXT",
            "tap:C-x quiet:60 tap:b quiet:1500 answer:after",
        )
        .env("PMACS_GPU_PROBE_ACTION", "popup")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "40000")
        .output()
        .expect("run the headless GPU probe");
    let Some(facts) = probe_facts(&output, &report, "e10z-gpu") else {
        return;
    };
    assert_eq!(fact(&facts, "ready"), "true");
    let keys: Vec<Sent> = proxy
        .since(0)
        .into_iter()
        .filter(|s| matches!(s, Sent::Key(..) | Sent::Op))
        .collect();
    let at = keys
        .iter()
        .position(|s| *s == ctrl('x'))
        .unwrap_or_else(|| panic!("the window sent no C-x: {keys:?}"));
    assert_eq!(
        keys.get(at + 1),
        Some(&plain('b')),
        "the window must send `b` as a key while the daemon has not read `C-x`: {keys:?}"
    );
    assert_eq!(
        fact(&facts, "after.minibuffer_open"),
        "true",
        "the prompt opens"
    );
}

/// Guard, E10z.2: after a motion key the daemon has answered, text goes as
/// ops again, every character, read by count, so that the fix cannot be to
/// round-trip everything. Passes at the base and after.
#[test]
fn e10z_2_guard_text_after_an_answered_motion_key_is_optimistic() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    let mark = proxy.mark();
    tui.write(b"\x05"); // C-e, a motion the daemon answers, at the end already
    std::thread::sleep(Duration::from_millis(300));
    for c in ["x", "y", "z"] {
        tui.write(c.as_bytes());
        std::thread::sleep(Duration::from_millis(150));
    }
    let sent = proxy.wait_sent(mark, 4, "C-e then xyz");
    assert_eq!(
        sent[..4].to_vec(),
        vec![ctrl('e'), Sent::Op, Sent::Op, Sent::Op],
        "three characters typed after an answered motion go as three ops"
    );
}

/// Guard, E10z.2: after a key that flips no `DispatchIdle`, text goes
/// optimistic again within a stated bound, 250 ms on a local socket, so
/// that no key is stranded: ten trials of `C-e`, then a character 250 ms
/// later, each after a random 0 to 60 ms. Passes at the base and after.
#[test]
fn e10z_2_guard_text_goes_optimistic_again_within_the_bound() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    let mut phase = Phase::new();
    let mut late = Vec::new();
    for trial in 0..10 {
        phase.wait(60);
        let mark = proxy.mark();
        tui.write(b"\x05");
        std::thread::sleep(Duration::from_millis(250));
        tui.write(b"w");
        let sent = proxy.wait_sent(mark, 2, "C-e then w");
        if sent[1] != Sent::Op {
            late.push(format!("trial {trial}: {sent:?}"));
        }
    }
    assert!(
        late.is_empty(),
        "a character 250 ms after C-e went as a key: {late:#?}"
    );
}

/// Guard, E10z.2: a word typed straight after a chord arrives whole and in
/// order, into a prompt (`C-x b` and `notes` in one write) and into the
/// buffer (`C-e` and `word` in one write). Passes at the base and after.
#[test]
fn e10z_2_guard_a_word_straight_after_a_chord_arrives_whole_and_in_order() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    tui.write(b"\x18bnotes");
    tui.wait_for(PATIENCE, "the prompt holding `notes`", &|t| {
        t.rows()
            .iter()
            .any(|r| r.contains("Switch to buffer") && r.contains("notes"))
    });
    cancel(&mut tui, "Switch to buffer");
    tui.write(b"\x05word");
    tui.wait_for(PATIENCE, "`word` at the end of the line", &|t| {
        buffer_text(t)
            .lines()
            .any(|l| l.trim_end().ends_with(" word"))
    });
}
