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

use pmacs_protocol::message::{
    AttachRequest, FrontendEvent, InstanceMessage, Key, Modifiers, MouseKind,
};

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

/// A daemon message's name for a timeline, with the field that matters.
fn received_name(m: &InstanceMessage) -> String {
    match m {
        InstanceMessage::CursorByte { byte_pos, .. } => format!("CursorByte({byte_pos})"),
        InstanceMessage::DispatchIdle { idle } => format!("DispatchIdle({idle})"),
        InstanceMessage::InputHandled { events } => format!("InputHandled({events})"),
        other => format!("{other:?}")
            .split(['(', ' ', '{'])
            .next()
            .unwrap_or("")
            .to_owned(),
    }
}

type Trigger = Box<dyn Fn(&FrontendEvent) -> bool + Send>;

#[derive(Default)]
struct ProxyState {
    sent: Vec<(Instant, Sent)>,
    /// What the daemon wrote, by variant, as the proxy read it.
    received: Vec<(Instant, String)>,
    /// The next event this accepts holds it and every event after it
    /// until `.1` has passed from its read; one-shot.
    arm: Option<(Trigger, Duration)>,
    hold_until: Option<Instant>,
    /// What the daemon writes after the handshake's reply reaches the
    /// frontend no sooner than this long behind the reply.
    first_frame_late: Duration,
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
        let down_state = Arc::clone(&state);
        std::thread::spawn(move || {
            let mut reply_at = None;
            while let Some(frame) = read_frame(&mut back_read) {
                let at = Instant::now();
                let name = postcard::from_bytes::<InstanceMessage>(&frame[4..])
                    .map_or_else(|_| "?".to_owned(), |m| received_name(&m));
                let late = {
                    let mut st = down_state.lock().expect("proxy state");
                    st.received.push((at, name));
                    st.first_frame_late
                };
                // The first frame is the handshake's reply; the session's
                // frames follow it, `late` behind it at the soonest.
                let release = match reply_at {
                    None => {
                        reply_at = Some(at);
                        at + latency
                    }
                    Some(reply) => (at + latency).max(reply + latency + late),
                };
                if down_tx.send((release, frame)).is_err() {
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

    /// Before a frontend attaches: its session's first frame, and what
    /// follows, reach it no sooner than `late` behind the handshake's
    /// reply, so that the frontend has forwarded input before it has read
    /// any frame, as it has on a link whose far side is slow to paint.
    fn first_frame_late(&self, late: Duration) {
        self.state.lock().expect("proxy state").first_frame_late = late;
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

    /// Both directions since `from`, in order, ms after it: a failing
    /// row's account of what the frontend had read when it decided.
    fn timeline(&self, from: Instant) -> String {
        let st = self.state.lock().expect("proxy state");
        let mut all: Vec<(Instant, String)> = st
            .sent
            .iter()
            .filter(|(at, _)| *at >= from)
            .map(|(at, s)| (*at, format!("  frontend -> {s:?}")))
            .chain(
                st.received
                    .iter()
                    .filter(|(at, _)| *at >= from)
                    .map(|(at, s)| (*at, format!("daemon -> {s}"))),
            )
            .collect();
        all.sort_by_key(|(at, _)| *at);
        all.iter()
            .map(|(at, s)| format!("{:9.2} {s}", at.duration_since(from).as_secs_f64() * 1000.0))
            .collect::<Vec<_>>()
            .join("\n")
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
    let mut tui = tui_just_started(socket, home, remote);
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

/// [`tui`] as it is the moment the process is started: nothing waited for
/// and nothing typed, for a row whose state begins before the first frame.
fn tui_just_started(socket: &Path, home: &Path, remote: Option<&Path>) -> ScreenTui {
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
    ScreenTui::new(pty, tap, ROWS, COLS)
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
/// `b` typed 150 ms after it goes as a key, the prompt opens, and no `b`
/// reaches the buffer. At the base the `CursorByte`s the daemon's ticks
/// write while the read is held re-ground the cursor, and `b` goes as
/// text (E10u.1 (c): 20 of 20 at 200 ms). The gap is past the pump's
/// 50 ms poll three times over: at 50 ms the base's pump had or had not
/// read a cursor by whether its wait ran out before `b` came, so a row
/// there passed at the base by timing alone.
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
        let from = Instant::now();
        let (sent, prompt) = c_x_b(&mut tui, &proxy, Some(Duration::from_millis(150)));
        assert_eq!(
            sent,
            vec![ctrl('x'), plain('b')],
            "trial {trial}: `b` typed while the daemon had not read `C-x` must go as a key; \
             what passed:\n{}",
            proxy.timeline(from)
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
/// the daemon's read of the first key held 300 ms: `C-x k`, `M-g g`, and a
/// letter typed after `M-x`, `C-s` and `M-%`, each second key 150 ms after
/// the first and a key. At the base each went as text in 16 to 17 of 20 at
/// a 30 ms gap. The gap was 50 ms, the base pump's poll, until review 1
/// found the row passing at the base in one bite of three (its Low 1), as
/// (i) did before `b66a66b`; and every case is tried before the row
/// asserts, so that a failure names each case that went wrong.
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
    let mut wrong = Vec::new();
    for (first, first_sent, second, needle) in cases {
        let Sent::Key(Key::Char(second_char), _) = second else {
            unreachable!()
        };
        let first_key = first_sent.clone();
        proxy.hold_from(move |e| sent_of(e) == first_key, HOLD);
        let mark = proxy.mark();
        tui.write(first);
        // Past the base pump's 50 ms poll, inside the hold (as row (i)).
        std::thread::sleep(Duration::from_millis(150));
        tui.write(second_char.to_string().as_bytes());
        let sent = proxy.wait_sent(mark, 2, needle);
        let shown = tui
            .watch_for(Duration::from_secs(3), &|t| t.screen_contains(needle))
            .is_some();
        if sent[..2] != [first_sent.clone(), second.clone()] || !shown {
            wrong.push(format!(
                "{needle}: sent {:?}, the prompt {}",
                &sent[..2],
                if shown { "shown" } else { "never shown" }
            ));
        }
        cancel(&mut tui, needle);
        if sent[1] == Sent::Op {
            // A second key taken as text: with the prefix or prompt gone,
            // take it back, so the next case starts as this one did.
            tui.write(b"\x7f");
            tui.settle(
                Duration::from_millis(250),
                PATIENCE,
                "the stray letter taken back",
                &|_| true,
            );
        }
    }
    assert!(
        wrong.is_empty(),
        "after each first key the next must go as a key, its prompt open: {wrong:#?}"
    );
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

/// E10z.2 (iii) and the ruling on the count (D47): a character typed 150
/// ms after a click, with the daemon's read of the click held 300 ms, goes
/// as a key and lands where the click put the caret. At the base it went as text at
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
    let from = Instant::now();
    // SGR mouse: a left press and release on `a`, one-based.
    tui.write(format!("\x1b[<0;{c};{r}M\x1b[<0;{c};{r}m", c = col + 1, r = row + 1).as_bytes());
    // Past the base pump's 50 ms poll, inside the hold (as row (i)).
    std::thread::sleep(Duration::from_millis(150));
    tui.write(b"x");
    let sent = proxy.wait_sent(mark, 3, "the click and `x`");
    assert_eq!(
        sent.last().cloned(),
        Some(plain('x')),
        "`x` typed while the daemon had not read the click must go as a key; what passed:\n{}",
        proxy.timeline(from)
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

/// Run `pmacs-gpu --headless-probe` on `proxy`'s socket with `script`,
/// or `None` where the probe cannot run (no binary, no adapter), as the
/// GPU rows above do.
fn gpu_script(
    proxy: &Proxy,
    script: &str,
    label: &str,
) -> Option<std::collections::HashMap<String, String>> {
    let binary = gpu_probe_binary()?;
    let report = proxy.path.with_file_name(format!("{label}.report"));
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(&proxy.path)
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", script)
        .env("PMACS_GPU_PROBE_ACTION", "popup")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "40000")
        .output()
        .expect("run the headless GPU probe");
    let facts = probe_facts(&output, &report, label)?;
    assert_eq!(
        fact(&facts, "ready"),
        "true",
        "{label}: the window got ready"
    );
    Some(facts)
}

/// Review 1's Medium 1, the GPU's own paste: with the daemon's read of a
/// `Paste` held 300 ms, `x` typed 60 ms after it goes as a key, and with
/// `y` typed 350 ms after `x`, once the daemon has answered both, the
/// daemon's buffer reads `PQxy`. Before the fix round the window marked
/// nothing after a paste and sent both as ops, and its optimistic floor
/// put `y` inside the pasted text, `PyQx` (3 of 3 by review 1's probe
/// through the real Ctrl-V on a private compositor's clipboard, and this
/// row at `3833dfb`); the `paste:` step sends what Ctrl-V sends once it
/// has read the clipboard, through the same function. `y` may go as an
/// op: by then the count covers the paste, and the caret is the daemon's.
#[test]
fn e10z_r1_the_gpu_window_holds_text_typed_after_its_paste() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    proxy.hold_from(|e| matches!(e, FrontendEvent::Paste { .. }), HOLD);
    let from = Instant::now();
    let Some(_facts) = gpu_script(
        &proxy,
        "quiet:400 paste:PQ quiet:60 tap:x quiet:350 tap:y quiet:1500 answer:after",
        "e10z-r1-gpu-paste",
    ) else {
        return;
    };
    let sent = proxy.since(0);
    let at = sent
        .iter()
        .position(|s| matches!(s, Sent::Other(name) if name == "Paste"))
        .unwrap_or_else(|| panic!("the window sent no Paste: {sent:?}"));
    let after: Vec<Sent> = sent[at + 1..]
        .iter()
        .filter(|s| matches!(s, Sent::Key(..) | Sent::Op))
        .cloned()
        .collect();
    let timeline = proxy.timeline(from);
    // What the daemon holds, read by a terminal attached afterwards.
    let home = tempfile::tempdir().expect("home");
    let tui = tui(&proxy.path, home.path(), None);
    let text = buffer_text(&tui);
    assert!(
        after.first() == Some(&plain('x')) && text.contains("PQxy"),
        "`x`, typed while the daemon had not read the paste, must go as a key, and both \
         letters land after the paste: sent after the paste {after:?}, the buffer\n{text}\n\
         what passed:\n{timeline}"
    );
}

/// A bottom panel on `C-c 9`, as `bottom_panel_stage2b_gpu_acceptance`
/// opens one: `pmacs.window.display` with a side.
const PANEL_ON_KEY: &str = r#"
pmacs.command.define {
  name = "e10z.panel",
  description = "Open a bottom panel for E10z's GPU row.",
  fn = function()
    pmacs.window.display(pmacs.buffer.create("*e10z-panel*"), { side = "bottom", height = 4 })
  end,
}
pmacs.keymap.bind { scope = "global", sequence = "C-c 9", command = "e10z.panel" }
"#;

/// Review 1's Medium 1, the panel: with the daemon's read of a left press
/// on the panel held 300 ms, `x` typed 60 ms after the click goes as a
/// key, which the daemon gives the panel the click focused. Before the
/// fix round the window marked nothing after a panel gesture and sent
/// `x` as an op into the document it still showed, which the daemon,
/// its focus moved to the side window, does not take
/// (`validate_remote_crdt_op`). The `panel-click:` step presses and
/// releases on a cell of the presented panel through the panel's sender.
#[test]
fn e10z_r1_the_gpu_window_holds_a_key_typed_after_a_panel_click() {
    let daemon = TestDaemon::spawn_with_config(&format!(
        "{NO_REOPEN}pmacs.lsp.config = {{}}\n{PANEL_ON_KEY}"
    ));
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    proxy.hold_from(
        |e| {
            matches!(
                e,
                FrontendEvent::PanelPointer {
                    kind: MouseKind::Down(_),
                    ..
                } | FrontendEvent::PanelPointerMapped {
                    kind: MouseKind::Down(_),
                    ..
                }
            )
        },
        HOLD,
    );
    let from = Instant::now();
    let Some(facts) = gpu_script(
        &proxy,
        "key:C-c key:9 quiet:500 panel-click:1,1 quiet:60 tap:x quiet:1500 answer:after",
        "e10z-r1-gpu-panel",
    ) else {
        return;
    };
    let sent = proxy.since(0);
    let at = sent
        .iter()
        .position(|s| matches!(s, Sent::Other(name) if name.starts_with("PanelPointer")))
        .unwrap_or_else(|| panic!("the window sent no panel gesture: {sent:?}; {facts:?}"));
    let after: Vec<Sent> = sent[at + 1..]
        .iter()
        .filter(|s| matches!(s, Sent::Key(..) | Sent::Op))
        .cloned()
        .collect();
    assert_eq!(
        after.first(),
        Some(&plain('x')),
        "`x`, typed while the daemon had not read the panel click, must go as a key: \
         sent {sent:?}\nwhat passed:\n{}",
        proxy.timeline(from)
    );
}

/// Review 1's Medium 2 (a): over a link slower than the hands, a character
/// typed 100 ms after a buttonless pointer motion goes as an op, as it did
/// at the base (`d8fd793`): a motion moves no caret and flips nothing, so
/// it is not something the next key waits for. At `b66a66b` every one went
/// as a key, the pump raising what it needs after every forwarded frame
/// (review 1: 0 of 10 over the link, 10 of 10 at the base and locally). A
/// regression row: it passes at the base. At review 1's 50 ms the base
/// failed 2 of 10 here, the gap the base pump's 50 ms poll, which decided a
/// key before reading the daemon's re-grounding cursor whenever the key
/// came before the poll ran out; 100 ms is past the poll and well inside
/// the link's 200 ms round trip.
#[test]
fn e10z_r1_text_after_a_pointer_motion_over_a_link_stays_optimistic() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let mut phase = Phase::new();
    let mut late = Vec::new();
    for trial in 0..10u16 {
        // everything before the motion answered
        std::thread::sleep(Duration::from_millis(600));
        phase.wait(60);
        let mark = proxy.mark();
        // SGR 35: a motion with no button held, over the buffer
        tui.write(format!("\x1b[<35;{};5M", 30 + trial % 5).as_bytes());
        std::thread::sleep(Duration::from_millis(100));
        tui.write(b"x");
        let sent = proxy.wait_sent(mark, 2, "a motion, then x");
        if sent[..2] != [Sent::Mouse, Sent::Op] {
            late.push(format!("trial {trial}: {:?}", &sent[..2]));
        }
    }
    assert!(
        late.is_empty(),
        "a character after a pointer motion went as a key over the link: {late:#?}"
    );
}

/// Review 1's Medium 2 (a), its second half, found by CI's macOS luajit leg
/// at `722f154` (the motion row above, 3 of 10 keys there): a pointer motion
/// raised nothing, but the pump still marked the caret stale after it, so a
/// character typed before some `CursorByte` came went as a key. A motion,
/// or a focus change, moves no caret, so it marks nothing: twenty trials of
/// a motion over a link and `x` 5 ms after it, every `x` an op. At `722f154`
/// `x` went as a key whenever no cursor had arrived in those 5 ms.
#[test]
fn e10z_r1_a_pointer_motion_leaves_the_caret_fresh() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let mut phase = Phase::new();
    let mut late = Vec::new();
    for trial in 0..20u16 {
        std::thread::sleep(Duration::from_millis(400));
        phase.wait(60);
        let mark = proxy.mark();
        tui.write(format!("\x1b[<35;{};5M", 30 + trial % 5).as_bytes());
        std::thread::sleep(Duration::from_millis(5));
        tui.write(b"x");
        let sent = proxy.wait_sent(mark, 2, "a motion, then x");
        if sent[..2] != [Sent::Mouse, Sent::Op] {
            late.push(format!("trial {trial}: {:?}", &sent[..2]));
        }
    }
    assert!(
        late.is_empty(),
        "a character 5 ms after a pointer motion went as a key: {late:#?}"
    );
}

/// How many of the characters of each word typed one every `gap` went as
/// ops, over `tui`, each word on a fresh line, everything before it
/// answered; and the words the screen shows wrong.
fn type_words(
    tui: &mut ScreenTui,
    proxy: &Proxy,
    words: &[&str],
    gap: Duration,
) -> (usize, usize, Vec<String>) {
    let mut phase = Phase::new();
    let (mut ops, mut chars, mut wrong) = (0, 0, Vec::new());
    for word in words {
        std::thread::sleep(Duration::from_millis(800));
        phase.wait(60);
        let mark = proxy.mark();
        for c in word.chars() {
            tui.write(c.to_string().as_bytes());
            std::thread::sleep(gap);
        }
        let sent = proxy.wait_sent(mark, word.len(), word);
        ops += sent[..word.len()]
            .iter()
            .filter(|s| **s == Sent::Op)
            .count();
        chars += word.len();
        let shown = tui
            .watch_for(Duration::from_secs(5), &|t| {
                buffer_text(t)
                    .lines()
                    .any(|l| l.trim_end().ends_with(&format!(" {word}")))
            })
            .is_some();
        if !shown {
            wrong.push(format!("{word}: {}", buffer_text(tui)));
        }
        tui.write(b"\r");
    }
    (ops, chars, wrong)
}

/// Review 1's Medium 2 (b): over a link, 100 ms each way, a word typed a
/// character every 60 ms, each on a fresh line, keeps its optimistic pace:
/// at least 27 of its 30 characters go as ops, and every word reads right.
/// At `b66a66b` 3 of 30 went as ops (review 1), the first of each word: the
/// mirror re-grounded on a `CursorByte` older than its own ops, behind the
/// character just typed, and the next fell back to a key that the rest
/// then waited on; at the base (`d8fd793`) 9 of 30. The bound is the fix
/// round's: 30 of 30 in each of six runs with the fix, room left for three
/// to go as keys on a loaded machine.
#[test]
fn e10z_r1_typing_over_a_link_keeps_its_optimistic_pace() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let words = ["hello", "there", "quick", "brown", "foxes", "jumps"];
    let (ops, chars, wrong) = type_words(&mut tui, &proxy, &words, Duration::from_millis(60));
    eprintln!("e10z_r1 typing over the link at 60 ms: {ops} of {chars} characters as ops");
    assert!(
        wrong.is_empty(),
        "every word reads right over the link: {wrong:#?}"
    );
    assert!(
        ops >= 27,
        "{ops} of {chars} characters typed 60 ms apart over the link went as ops; \
         at least 27 must"
    );
}

/// The fix round's own finding, by the walk of the owner's guide: over a
/// link, 100 ms each way, a sentence typed a character every 60 ms reads on
/// the pane as it was typed, with no repaint, as it reads in the buffer.
/// With Medium 2 (b) alone the mirror kept its caret over a frame older
/// than its ops, but the frame's `Cursor` moved the terminal's caret back,
/// so the next optimistic paint printed a column early; where the true
/// character was a space the daemon, which diffs against what it last sent,
/// never repainted the cell, and the pane read `theqquick brownofox`. The
/// base (`d8fd793`) did the same wherever two ops went in a row; `b66a66b`
/// sent nearly every character as a key over a link, and read right.
#[test]
fn e10z_r1_a_sentence_typed_over_a_link_reads_right_on_the_pane() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let sentence = "the quick brown fox jumps over the lazy dog";
    let mut wrong = Vec::new();
    for trial in 0..3 {
        std::thread::sleep(Duration::from_millis(800));
        let mark = proxy.mark();
        for c in sentence.chars() {
            tui.write(c.to_string().as_bytes());
            std::thread::sleep(Duration::from_millis(60));
        }
        let sent = proxy.wait_sent(mark, sentence.len(), "the sentence");
        let ops = sent.iter().filter(|s| **s == Sent::Op).count();
        let shown = tui
            .watch_for(Duration::from_secs(5), &|t| {
                buffer_text(t)
                    .lines()
                    .any(|l| l.trim_end().ends_with(&format!(" {sentence}")))
            })
            .is_some();
        if !shown {
            let line = buffer_text(&tui)
                .lines()
                .rfind(|l| l.contains("lazy") || l.contains("quick"))
                .unwrap_or("")
                .trim_end()
                .to_owned();
            wrong.push(format!("trial {trial} ({ops} ops): {line:?}"));
        }
        tui.write(b"\r");
    }
    assert!(
        wrong.is_empty(),
        "the pane must read the sentence as typed: {wrong:#?}"
    );
}

/// A pointer crossing the pane as a hand moves it, measured at fix round 1's
/// addition through `foot`, tmux and crossterm on a private headless sway:
/// about 115 buttonless motions a second at 1000 px/s, 50 at 400 and up to
/// 280 at 2500 (`~/build/e10z-f1/rate`). This writes a motion every 8.5 ms
/// after `text`'s every letter, the letters `gap` apart, crossing row 4.
fn type_beside_a_moving_pointer(tui: &mut ScreenTui, text: &str, gap: Duration, start_col: u16) {
    let motions = (gap.as_micros() / 8_500).max(1) as u32;
    let step = gap / motions;
    let mut col = start_col;
    for c in text.chars() {
        tui.write(c.to_string().as_bytes());
        for _ in 0..motions {
            std::thread::sleep(step);
            col = 2 + (col + 1) % (COLS - 4);
            tui.write(format!("\x1b[<35;{col};4M").as_bytes());
        }
    }
}

/// The letters, ops or keys, among what the frontend sent.
fn letters(sent: &[Sent]) -> Vec<Sent> {
    sent.iter()
        .filter(|s| matches!(s, Sent::Op | Sent::Key(..)))
        .cloned()
        .collect()
}

/// The owner's ruling of 2026-11-18 on fix round 1's open question: over a
/// link, 100 ms each way, a sentence typed a letter every 60 ms with the
/// pointer moving the whole time, at the rate a hand gives (a motion every
/// 8.5 ms, about 7 between two letters), keeps its optimistic pace: at
/// least 40 of its 43 letters go as ops in each of three trials, and the
/// pane reads it right. The bound is the addition's own: 43 of 43 in every
/// trial of its runs, room left for three on a loaded machine. At `96ba5d1`
/// a motion stopped the mirror keeping its caret and counted toward the
/// run-ahead bound, and the letters went as keys; at the base (`d8fd793`)
/// the pane misplaced letters as without the pointer.
#[test]
fn e10z_r1_typing_beside_a_moving_pointer_over_a_link_keeps_its_pace() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let sentence = "the quick brown fox jumps over the lazy dog";
    let mut wrong = Vec::new();
    for trial in 0..3u16 {
        std::thread::sleep(Duration::from_millis(800));
        let mark = proxy.mark();
        type_beside_a_moving_pointer(&mut tui, sentence, Duration::from_millis(60), trial * 17);
        let start = Instant::now();
        let typed = loop {
            let got = letters(&proxy.since(mark));
            if got.len() >= sentence.len() || start.elapsed() > PATIENCE {
                break got;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let ops = typed.iter().filter(|s| **s == Sent::Op).count();
        eprintln!("e10z_r1 beside a moving pointer, trial {trial}: {ops} of 43 letters as ops");
        let shown = tui
            .watch_for(Duration::from_secs(5), &|t| {
                buffer_text(t)
                    .lines()
                    .any(|l| l.trim_end().ends_with(&format!(" {sentence}")))
            })
            .is_some();
        if ops < 40 || !shown {
            let line = buffer_text(&tui)
                .lines()
                .rfind(|l| l.contains("lazy") || l.contains("quick"))
                .unwrap_or("")
                .trim_end()
                .to_owned();
            wrong.push(format!("trial {trial}: {ops} of 43 ops, the pane {line:?}"));
        }
        tui.write(b"\r");
    }
    assert!(
        wrong.is_empty(),
        "beside a moving pointer at least 40 of 43 letters go as ops and the pane reads \
         the sentence: {wrong:#?}"
    );
}

/// The ruling's other half: a focus change, `FocusGained` or `FocusLost`,
/// between two letters over the link does not stop the mirror keeping its
/// caret. Ten trials of a letter, a focus change 5 ms later and a second
/// letter 60 ms after the first: both go as ops every time. At `96ba5d1` a
/// focus change was an unanswered event the mirror kept no caret across,
/// and a cursor older than the first letter sent the second as a key.
#[test]
fn e10z_r1_a_focus_change_between_two_letters_keeps_the_pace() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let mut phase = Phase::new();
    let mut late = Vec::new();
    for trial in 0..10u16 {
        std::thread::sleep(Duration::from_millis(600));
        phase.wait(60);
        let mark = proxy.mark();
        tui.write(b"a");
        std::thread::sleep(Duration::from_millis(5));
        tui.write(if trial % 2 == 0 { b"\x1b[I" } else { b"\x1b[O" });
        std::thread::sleep(Duration::from_millis(55));
        tui.write(b"b");
        let sent = proxy.wait_sent(mark, 3, "a, a focus change, b");
        let expect_focus = if trial % 2 == 0 {
            "FocusGained"
        } else {
            "FocusLost"
        };
        if sent[..3] != [Sent::Op, Sent::Other(expect_focus.to_owned()), Sent::Op] {
            late.push(format!("trial {trial}: {:?}", &sent[..3]));
        }
    }
    assert!(
        late.is_empty(),
        "a letter after a focus change over the link went as a key: {late:#?}"
    );
}

/// The run-ahead bound with a pointer moving at a daemon that does not
/// answer: the read held 6 s from the first of 24 letters typed 20 ms
/// apart, two motions between each two letters, and still 16 letters go as
/// ops and the rest as keys: motions do not count toward the bound, and do
/// not loosen it. At `96ba5d1` motions counted, and the letters went as
/// keys after five or six.
#[test]
fn e10z_r1_the_run_ahead_bound_holds_beside_a_moving_pointer() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    let hold = Duration::from_secs(6);
    let inside = Duration::from_millis(5500);
    proxy.hold_from(|e| matches!(e, FrontendEvent::CrdtOp { .. }), hold);
    let mark = proxy.mark();
    let first = Instant::now();
    type_beside_a_moving_pointer(
        &mut tui,
        "abcdefghijklmnopqrstuvwx",
        Duration::from_millis(20),
        5,
    );
    let start = Instant::now();
    let typed = loop {
        let got = letters(&proxy.since(mark));
        if got.len() >= 24 || start.elapsed() > PATIENCE {
            break got;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let took = first.elapsed();
    assert!(
        took < inside && typed.len() >= 24,
        "premise: the 24 letters reached the proxy {} ms after the first was typed, \
         not inside the {} ms hold ({} letters)",
        took.as_millis(),
        hold.as_millis(),
        typed.len()
    );
    let ops = typed[..24].iter().take_while(|s| **s == Sent::Op).count();
    let rest_keys = typed[ops..24].iter().all(|s| matches!(s, Sent::Key(..)));
    assert!(
        ops == 16 && rest_keys,
        "16 letters as ops and then keys beside a moving pointer at a daemon that \
         answered none: {:?}",
        &typed[..24]
    );
    // E10z fix round 2 --- the plain row's last step, the pointer still
    // moving: once the daemon has answered, a letter goes as an op again.
    let mut col = 5u16;
    let until = Instant::now() + PATIENCE;
    while !buffer_text(&tui)
        .lines()
        .any(|l| l.trim_end().ends_with(" abcdefghijklmnopqrstuvwx"))
    {
        assert!(
            Instant::now() < until,
            "all 24 letters on the pane beside the moving pointer: {}",
            tui.describe()
        );
        move_the_pointer(&mut tui, Duration::from_millis(50), &mut col);
    }
    move_the_pointer(&mut tui, Duration::from_millis(500), &mut col);
    let mark = proxy.mark();
    tui.write(b"y");
    move_the_pointer(&mut tui, Duration::from_millis(300), &mut col);
    let after = letters(&proxy.since(mark));
    assert_eq!(
        after.first(),
        Some(&Sent::Op),
        "once answered, a letter goes as an op again beside the moving pointer: {after:?}"
    );
}

/// The fix round's run-ahead bound (the owner's ruling of 2026-11-18): with
/// the daemon's read held from the first of 24 letters, typed 20 ms apart,
/// the first 16 go as ops and the rest as keys, so a daemon that stops
/// answering is owed at most 16 ops; and once it has answered them, a
/// letter goes as an op again, so nothing is stranded. At `1d34cf8`, with
/// no bound, all 24 went as ops (and E10r's `r2` and `h2`, 6,000 letters at
/// a stopped daemon, failed on the minutes the daemon took over the ops).
/// The hold is 6 s and the row requires all 24 read inside it: at 1.5 s
/// CI's macOS legs read 24 ops at `722f154`, the suite there four times as
/// slow as on the laptop, which a hold that ended before the 16th letter
/// would give with the bound in place.
#[test]
fn e10z_r1_optimistic_typing_runs_ahead_of_the_answer_by_at_most_16() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    let hold = Duration::from_secs(6);
    let inside = Duration::from_millis(5500);
    proxy.hold_from(|e| matches!(e, FrontendEvent::CrdtOp { .. }), hold);
    let mark = proxy.mark();
    let first = Instant::now();
    for c in "abcdefghijklmnopqrstuvwx".chars() {
        tui.write(c.to_string().as_bytes());
        std::thread::sleep(Duration::from_millis(20));
    }
    let sent = proxy.wait_sent(mark, 24, "24 letters");
    let took = first.elapsed();
    assert!(
        took < inside,
        "premise: the 24 letters reached the proxy {} ms after the first was typed, \
         not inside the {} ms hold",
        took.as_millis(),
        hold.as_millis()
    );
    let ops = sent[..24].iter().take_while(|s| **s == Sent::Op).count();
    let rest_keys = sent[ops..24].iter().all(|s| matches!(s, Sent::Key(..)));
    assert!(
        ops == 16 && rest_keys,
        "16 letters as ops and then keys while the daemon had answered none: {:?}",
        &sent[..24]
    );
    tui.wait_for(PATIENCE, "all 24 letters", &|t| {
        buffer_text(t)
            .lines()
            .any(|l| l.trim_end().ends_with(" abcdefghijklmnopqrstuvwx"))
    });
    std::thread::sleep(Duration::from_millis(500));
    let mark = proxy.mark();
    tui.write(b"y");
    let sent = proxy.wait_sent(mark, 1, "y");
    assert_eq!(
        sent[0],
        Sent::Op,
        "once answered, a letter goes as an op again"
    );
}

/// A pointer crossing row 4 for `time`, a buttonless motion every 8.5 ms as
/// [`type_beside_a_moving_pointer`] writes them, the pane read between
/// them, so that the screen a row reads next is the one the pointer was
/// moving over and not one from before it.
fn move_the_pointer(tui: &mut ScreenTui, time: Duration, col: &mut u16) {
    let until = Instant::now() + time;
    while Instant::now() < until {
        *col = 2 + (*col + 1) % (COLS - 4);
        tui.write(format!("\x1b[<35;{col};4M").as_bytes());
        let _ = tui.watch_for(Duration::from_micros(8_500), &|_| false);
    }
}

/// Letters `gap` apart with the pointer moving between them.
fn type_beside(tui: &mut ScreenTui, text: &str, gap: Duration, col: &mut u16) {
    for c in text.chars() {
        tui.write(c.to_string().as_bytes());
        move_the_pointer(tui, gap, col);
    }
}

/// Review 2's Medium 1, the pace: over a link, 100 ms each way, with the
/// pointer moving the whole time, text typed after a round-tripped key the
/// daemon has answered goes as ops again, as it does with the pointer
/// still. `ab`, the key, 700 ms (three round trips and more), then `cdefgh`
/// 60 ms apart: at least five of the six as ops, after Enter and after
/// `C-e`, which is E10z.2's own guard beside a pointer. At `8e7e230` a
/// motion in flight counted as something the daemon had yet to answer, so
/// the `CursorByte` that answers the key was taken for older than the
/// mirror's caret and not applied, the stale mark the key left stood, and
/// every letter went as a key: 0 of 6 by the review's probe (6 of 6 with no
/// pointer, 2 of 6 at `d8fd793`).
#[test]
fn e10z_r2_text_after_an_answered_key_beside_a_moving_pointer_is_optimistic() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let mut col = 5u16;
    let mut wrong = Vec::new();
    for (trial, key) in [b"\r".as_slice(), b"\x05", b"\r"].into_iter().enumerate() {
        std::thread::sleep(Duration::from_millis(800));
        move_the_pointer(&mut tui, Duration::from_millis(500), &mut col);
        type_beside(&mut tui, "ab", Duration::from_millis(60), &mut col);
        move_the_pointer(&mut tui, Duration::from_millis(400), &mut col);
        tui.write(key);
        move_the_pointer(&mut tui, Duration::from_millis(700), &mut col);
        let mark = proxy.mark();
        type_beside(&mut tui, "cdefgh", Duration::from_millis(60), &mut col);
        move_the_pointer(&mut tui, Duration::from_millis(300), &mut col);
        let typed = letters(&proxy.since(mark));
        let ops = typed.iter().take(6).filter(|s| **s == Sent::Op).count();
        eprintln!(
            "e10z_r2 after an answered key beside a moving pointer, trial {trial}: {ops} of 6"
        );
        if ops < 5 {
            wrong.push(format!("trial {trial}: {ops} of 6 ops, {typed:?}"));
        }
        tui.write(b"\r");
    }
    assert!(
        wrong.is_empty(),
        "text typed after an answered key beside a moving pointer must go as ops: {wrong:#?}"
    );
}

/// Review 2's Medium 1, the caret: the same state, read on the pane. After
/// Enter, with the pointer still moving and nothing else typed, the
/// terminal's caret is on the new line. At `8e7e230` it stayed at the end
/// of the old line for as long as the pointer moved: the frame's `Cursor`
/// was held back because a motion was unanswered, and the caret put back
/// where it had been. The caret is read while the pointer moves.
#[test]
fn e10z_r2_the_caret_follows_enter_beside_a_moving_pointer() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let mut col = 5u16;
    move_the_pointer(&mut tui, Duration::from_millis(500), &mut col);
    type_beside(&mut tui, "ab", Duration::from_millis(60), &mut col);
    move_the_pointer(&mut tui, Duration::from_millis(600), &mut col);
    let before = tui.cursor().expect("a caret after ab");
    tui.write(b"\r");
    move_the_pointer(&mut tui, Duration::from_millis(900), &mut col);
    let after = tui.cursor().expect("a caret after Enter");
    assert_eq!(
        (after.row, after.col),
        (before.row + 1, before.col - 2),
        "after Enter the caret is at the start of the next line while the pointer moves: \
         {before:?} then {after:?}; {}",
        tui.describe()
    );
}

/// Review 2's Medium 1, the pane: a file opened beside a moving pointer,
/// then a word. The word goes as ops (the mirror re-grounds on the new
/// buffer's first cursor) and must be painted where it goes: on the file's
/// first line, which is empty so that the word is the optimistic path's,
/// and nowhere else. At `8e7e230` the new buffer's `CursorByte` was applied
/// while the frame's `Cursor` was held for the motions in flight, so the
/// word was painted on the bottom row, where the prompt's caret had been,
/// and stayed there until something repainted the grid; the buffer was
/// right throughout. The prompt comes prefilled with the daemon's
/// directory, which the row deletes before it types the file's path.
#[test]
fn e10z_r2_a_word_typed_in_a_file_opened_beside_a_moving_pointer_is_painted_where_it_goes() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let file = home.path().join("f.txt");
    std::fs::write(&file, "\nsecond line\nthird line\n").expect("the file");
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let mut col = 5u16;
    move_the_pointer(&mut tui, Duration::from_millis(500), &mut col);
    tui.write(b"\x18");
    move_the_pointer(&mut tui, Duration::from_millis(500), &mut col);
    tui.write(b"\x06");
    // The prompt's field: what stands after its label, up to the
    // candidate hint the prompt shows beside it.
    let field = |t: &ScreenTui| -> String {
        t.bottom_row()
            .strip_prefix("Find file: ")
            .and_then(|rest| rest.split(' ').next())
            .unwrap_or("")
            .to_owned()
    };
    let until = Instant::now() + PATIENCE;
    while !field(&tui).starts_with('/') {
        assert!(
            Instant::now() < until,
            "the find-file prompt beside the moving pointer: {}",
            tui.describe()
        );
        move_the_pointer(&mut tui, Duration::from_millis(50), &mut col);
    }
    move_the_pointer(&mut tui, Duration::from_millis(400), &mut col);
    let prefill = field(&tui).chars().count();
    tui.write(&vec![0x7f; prefill]);
    while !field(&tui).is_empty() {
        assert!(
            Instant::now() < until,
            "the prompt's prefill deleted: {}",
            tui.describe()
        );
        move_the_pointer(&mut tui, Duration::from_millis(50), &mut col);
    }
    type_beside(
        &mut tui,
        file.to_str().expect("utf-8"),
        Duration::from_millis(30),
        &mut col,
    );
    move_the_pointer(&mut tui, Duration::from_millis(400), &mut col);
    tui.write(b"\r");
    while !tui.screen_contains("second line") {
        assert!(
            Instant::now() < until,
            "the file on the pane beside the moving pointer: {}",
            tui.describe()
        );
        move_the_pointer(&mut tui, Duration::from_millis(50), &mut col);
    }
    move_the_pointer(&mut tui, Duration::from_secs(1), &mut col);
    let mark = proxy.mark();
    type_beside(&mut tui, "hello", Duration::from_millis(60), &mut col);
    move_the_pointer(&mut tui, Duration::from_millis(800), &mut col);
    let typed = letters(&proxy.since(mark));
    tui.settle(Duration::from_millis(300), PATIENCE, "the word", &|_| true);
    let rows = tui.rows();
    let strays: Vec<&String> = rows.iter().skip(1).filter(|r| r.contains("hel")).collect();
    assert!(
        rows[0].trim_end() == " 1 hello" && strays.is_empty(),
        "hello ({typed:?}) is on the file's first line and nowhere else on the pane: {}",
        tui.describe()
    );
}

/// Review 2's Medium 1, the attach: the pointer already moving when the TUI
/// attaches over the link, then a word. At `8e7e230` no `Cursor` of the
/// session had been applied, each read while a motion was unanswered, so
/// the terminal showed no caret, its own stood where the first frame's last
/// span left it, past the last cell of the pane, and on the review's
/// terminal the first optimistic paint wrapped and scrolled the whole pane
/// up a row, the word painted along the bottom. This suite's screen model
/// does not scroll there, so the row reads the cause as well as the pane:
/// before the word the caret stands at line 1's start beside the gutter,
/// after it behind the word; the word reads on line 1, the mode line stands
/// on its own row saying `L1:C6`, and no other row holds any of the word.
/// Whether the first frame is read before the first motion is forwarded is
/// a race the review's release build lost and this suite's debug build
/// wins, so the proxy holds the session's first frame 500 ms behind the
/// handshake's reply: the state is the review's every time.
#[test]
fn e10z_r2_a_word_typed_after_attaching_beside_a_moving_pointer_leaves_the_pane_whole() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    proxy.first_frame_late(Duration::from_millis(500));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui_just_started(&proxy.path, home.path(), Some(&script));
    let mut col = 5u16;
    let until = Instant::now() + PATIENCE;
    while !tui.screen_contains("*scratch*") {
        assert!(
            Instant::now() < until,
            "the attached *scratch* beside the moving pointer: {}",
            tui.describe()
        );
        move_the_pointer(&mut tui, Duration::from_millis(50), &mut col);
    }
    move_the_pointer(&mut tui, Duration::from_millis(1500), &mut col);
    let before = tui.cursor();
    let mark = proxy.mark();
    type_beside(&mut tui, "hello", Duration::from_millis(60), &mut col);
    move_the_pointer(&mut tui, Duration::from_secs(1), &mut col);
    let typed = letters(&proxy.since(mark));
    eprintln!("e10z_r2 attached beside a moving pointer: caret {before:?}, hello went {typed:?}");
    tui.settle(Duration::from_millis(300), PATIENCE, "the word", &|_| true);
    let rows = tui.rows();
    let mode_line = usize::from(ROWS) - 2;
    let strays: Vec<(usize, &String)> = rows
        .iter()
        .enumerate()
        .filter(|(i, r)| *i != 0 && r.contains("hel"))
        .collect();
    let after = tui.cursor();
    assert!(
        before.map(|c| (c.row, c.col)) == Some((0, 3))
            && after.map(|c| (c.row, c.col)) == Some((0, 8))
            && rows[0].trim_end() == " 1 hello"
            && rows[mode_line].contains("*scratch*")
            && rows[mode_line].contains("L1:C6")
            && strays.is_empty(),
        "the caret stands at line 1's start before the word ({before:?}) and behind it after \
         ({after:?}); hello ({typed:?}) reads on line 1 beside the gutter, the mode line on row \
         {mode_line} says L1:C6, and no other row holds the word: {}",
        tui.describe()
    );
}

/// Review 2's Low 2: at the run-ahead bound the painted caret stays ahead of
/// a frame older than the ops. Over a link, 300 ms each way, 24 letters
/// 10 ms apart: 16 go as ops and are painted, the rest as keys. The proxy
/// passes the first op and holds the second and everything behind it 3 s,
/// so the pump reads the daemon's frame for the first letter, and nothing
/// after it, with fifteen of its ops and a key unanswered. The caret must
/// still stand after the sixteenth painted letter. At `8e7e230` a key
/// behind the ops ended the hold of the frame's `Cursor`, and the caret
/// went back to after the first letter with fifteen painted beyond it; the
/// review read it at the line's start for the round trip.
#[test]
fn e10z_r2_the_painted_caret_stays_ahead_of_an_older_frame_at_the_run_ahead_bound() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(300));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    std::thread::sleep(Duration::from_millis(900));
    tui.settle(Duration::from_millis(300), PATIENCE, "the pane", &|_| true);
    let start = tui.cursor().expect("a caret before the letters");
    let ops_seen = std::sync::atomic::AtomicUsize::new(0);
    proxy.hold_from(
        move |e| {
            matches!(e, FrontendEvent::CrdtOp { .. })
                && ops_seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1
        },
        Duration::from_secs(3),
    );
    let mark = proxy.mark();
    for c in "abcdefghijklmnopqrstuvwx".chars() {
        tui.write(c.to_string().as_bytes());
        std::thread::sleep(Duration::from_millis(10));
    }
    let sent = proxy.wait_sent(mark, 24, "24 letters");
    let ops = sent[..24].iter().take_while(|s| **s == Sent::Op).count();
    assert!(
        ops == 16 && sent[16..24].iter().all(|s| matches!(s, Sent::Key(..))),
        "premise: 16 letters as ops and then keys, all before the first answer: {:?}",
        &sent[..24]
    );
    // The first letter's frame is back 600 ms after it was typed; the hold
    // keeps everything else from the daemon until 3 s.
    std::thread::sleep(Duration::from_millis(1200));
    tui.settle(
        Duration::from_millis(300),
        PATIENCE,
        "the first frame",
        &|_| true,
    );
    let held = tui.cursor().expect("a caret at the bound");
    assert_eq!(
        (held.row, held.col),
        (start.row, start.col + 16),
        "with a key and fifteen ops unanswered the caret stands after the sixteenth painted \
         letter, from {start:?}; {}",
        tui.describe()
    );
    tui.wait_for(PATIENCE, "all 24 letters", &|t| {
        buffer_text(t)
            .lines()
            .any(|l| l.trim_end().ends_with(" abcdefghijklmnopqrstuvwx"))
    });
}

/// E10z's paste on the terminal (D47 counts it), and what review 1's
/// Medium 2 (b) does after one: with the daemon's read of a bracketed
/// paste held 300 ms, `x` typed 150 ms after it goes as a key, and the
/// line reads `PQx`. A paste is not an op the mirror applied, so a
/// `CursorByte` read before the answer covers it re-grounds the mirror and
/// leaves it stale; the mirror keeps no caret across it. At the base
/// (`d8fd793`) `x` went as an op at the caret from before the paste.
#[test]
fn e10z_r1_a_key_typed_after_a_held_paste_goes_as_a_key() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    proxy.hold_from(|e| matches!(e, FrontendEvent::Paste { .. }), HOLD);
    let mark = proxy.mark();
    let from = Instant::now();
    tui.write(b"\x1b[200~PQ\x1b[201~");
    std::thread::sleep(Duration::from_millis(150));
    tui.write(b"x");
    let sent = proxy.wait_sent(mark, 2, "the paste and `x`");
    assert_eq!(
        sent[..2].to_vec(),
        vec![Sent::Other("Paste".to_owned()), plain('x')],
        "`x` typed while the daemon had not read the paste must go as a key; what passed:\n{}",
        proxy.timeline(from)
    );
    tui.wait_for(PATIENCE, "`PQx`", &|t| {
        buffer_text(t)
            .lines()
            .any(|l| l.trim_end().ends_with(" PQx"))
    });
}

/// What review 1's Medium 2 (b) does after a resize: with the daemon's read
/// of a terminal resize held 300 ms, `x` typed 150 ms after it goes as a
/// key. A resize is not an op the mirror applied and raises the answer as
/// a key does, so the mirror keeps no caret across it, and the next key
/// waits for the daemon. At the base (`d8fd793`) `x` went as an op once a
/// tick's `CursorByte` had re-grounded the mirror.
#[test]
fn e10z_r1_a_key_typed_after_a_held_resize_goes_as_a_key() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    proxy.hold_from(|e| matches!(e, FrontendEvent::Resize { .. }), HOLD);
    let mark = proxy.mark();
    let from = Instant::now();
    tui.resize(ROWS, COLS - 4);
    std::thread::sleep(Duration::from_millis(150));
    tui.write(b"x");
    let sent = proxy.wait_sent(mark, 2, "the resize and `x`");
    assert_eq!(
        sent[..2].to_vec(),
        vec![Sent::Other("Resize".to_owned()), plain('x')],
        "`x` typed while the daemon had not read the resize must go as a key; what passed:\n{}",
        proxy.timeline(from)
    );
}

/// Review 1's Low 2: `C-c l`, the chord E10u.1 (e) found and row (iii)
/// left out: with the read of `C-c` held 300 ms, `l` 150 ms later goes as a
/// key and `*lsp*` opens, three trials. At the base (`d8fd793`) `l` went as
/// an op and no `*lsp*` opened (review 1's probe, 0 of 5).
#[test]
fn e10z_r1_c_c_l_holds_its_second_key() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    for trial in 0..3 {
        proxy.hold_from(|e| sent_of(e) == ctrl('c'), HOLD);
        let mark = proxy.mark();
        tui.write(b"\x03");
        std::thread::sleep(Duration::from_millis(150));
        tui.write(b"l");
        let sent = proxy.wait_sent(mark, 2, "C-c l");
        assert_eq!(
            sent[..2].to_vec(),
            vec![ctrl('c'), plain('l')],
            "trial {trial}: `l` typed while the daemon had not read `C-c` must go as a key"
        );
        tui.wait_for(PATIENCE, "*lsp*", &|t| t.screen_contains("*lsp*"));
        close_lsp(&mut tui);
    }
}

/// `q` quits `*lsp*`'s list, `C-x 1` leaves one window, and the screen
/// settles on the buffer again.
fn close_lsp(tui: &mut ScreenTui) {
    tui.write(b"q");
    std::thread::sleep(Duration::from_millis(300));
    tui.write(b"\x181");
    tui.settle(
        Duration::from_millis(300),
        PATIENCE,
        "`*lsp*` closed",
        &|t| !t.screen_contains("*lsp*"),
    );
}

/// Review 1's Low 2: the remote route at the owner's hands for more than
/// `C-x b`. Over the `ssh` stand-in, 100 ms each way and nothing held,
/// `C-x k`, `M-x q`, `C-s q` and `C-c l`, each second key 150 ms after the
/// first, and `C-c l q`, each key 150 ms after the last: every key goes as
/// a key and each prompt opens. At the base (`d8fd793`) each second key
/// went as an op (review 1's probes, 0 of 25).
#[test]
fn e10z_r1_over_a_link_every_chord_holds_its_next_key() {
    let daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::from_millis(100));
    let home = tempfile::tempdir().expect("home");
    let script = stand_in(home.path(), &proxy.path);
    let mut tui = tui(&proxy.path, home.path(), Some(&script));
    let cases: [(&[u8], Sent, u8, &str); 4] = [
        (b"\x18", ctrl('x'), b'k', "Kill buffer"),
        (
            b"\x1bx",
            Sent::Key(Key::Char('x'), Modifiers::ALT),
            b'q',
            "M-x",
        ),
        (b"\x13", ctrl('s'), b'q', "I-search"),
        (b"\x03", ctrl('c'), b'l', "*lsp*"),
    ];
    let mut wrong = Vec::new();
    for (first, first_sent, second, needle) in cases {
        let mark = proxy.mark();
        tui.write(first);
        std::thread::sleep(Duration::from_millis(150));
        tui.write(&[second]);
        let sent = proxy.wait_sent(mark, 2, needle);
        let shown = tui
            .watch_for(Duration::from_secs(5), &|t| t.screen_contains(needle))
            .is_some();
        if sent[..2] != [first_sent, plain(second as char)] || !shown {
            wrong.push(format!("{needle}: sent {:?}, shown {shown}", &sent[..2]));
        }
        if needle == "*lsp*" && shown {
            close_lsp(&mut tui);
        } else {
            cancel(&mut tui, needle);
        }
        if sent[1] == Sent::Op {
            tui.write(b"\x7f");
            tui.settle(Duration::from_millis(400), PATIENCE, "taken back", &|_| {
                true
            });
        }
    }
    let mark = proxy.mark();
    tui.write(b"\x03");
    std::thread::sleep(Duration::from_millis(150));
    tui.write(b"l");
    std::thread::sleep(Duration::from_millis(150));
    tui.write(b"q");
    let sent = proxy.wait_sent(mark, 3, "C-c l q");
    if sent[..3] != [ctrl('c'), plain('l'), plain('q')] {
        wrong.push(format!("C-c l q: sent {:?}", &sent[..3]));
    }
    assert!(
        wrong.is_empty(),
        "over the link each next key must go as a key: {wrong:#?}"
    );
}

/// Review 1's Low 2: `pmacs-gpu` for more than `C-x b`: `C-c l`, `C-s q`
/// and `C-x k`, the first key's read held 300 ms and the second tapped 60
/// ms after it, each in a fresh window on a fresh daemon: the second goes
/// as a key. At the base's window (`8d09a6c`'s, the gate taken out) each
/// went as an op (review 1's hand bite, 6 of 6 for `C-x b`, `C-c l` and
/// `C-x k`).
#[test]
fn e10z_r1_the_gpu_window_holds_every_chords_next_key() {
    let mut wrong = Vec::new();
    for (first, second, first_sent) in [
        ("C-c", 'l', ctrl('c')),
        ("C-s", 'q', ctrl('s')),
        ("C-x", 'k', ctrl('x')),
    ] {
        let daemon = daemon();
        let proxy = Proxy::start(&daemon, Duration::ZERO);
        let held = first_sent.clone();
        proxy.hold_from(move |e| sent_of(e) == held, HOLD);
        let Some(_facts) = gpu_script(
            &proxy,
            &format!("tap:{first} quiet:60 tap:{second} quiet:1500 answer:after"),
            "e10z-r1-gpu-chord",
        ) else {
            return;
        };
        let keys: Vec<Sent> = proxy
            .since(0)
            .into_iter()
            .filter(|s| matches!(s, Sent::Key(..) | Sent::Op))
            .collect();
        let at = keys.iter().position(|s| *s == first_sent);
        if at.and_then(|at| keys.get(at + 1)) != Some(&plain(second)) {
            wrong.push(format!("{first} {second}: {keys:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "the window must send each second key as a key while the daemon has not read \
         the first: {wrong:#?}"
    );
}

/// Review 1's Low 2, the reconnect: the daemon killed and a new one
/// started on the same socket and home, and the terminal attaching again
/// by itself, a session whose counts start over. The first session forwards
/// a held chord and types, so both counts have moved; in the second, text
/// after an answered motion key still goes as ops (a count carried over
/// would strand it), and `C-x b` with the read of `C-x` held still opens
/// the prompt with `b` a key, three trials. At the base (`d8fd793`) the
/// witness half fails, `b` going as an op, as in row (i).
#[test]
fn e10z_r1_after_a_reconnect_the_answer_starts_over() {
    let mut daemon = daemon();
    let proxy = Proxy::start(&daemon, Duration::ZERO);
    let home = tempfile::tempdir().expect("home");
    let mut tui = tui(&proxy.path, home.path(), None);
    // The first session: a held chord, then a word.
    proxy.hold_from(|e| sent_of(e) == ctrl('x'), HOLD);
    tui.write(b"\x18");
    std::thread::sleep(Duration::from_millis(150));
    tui.write(b"b");
    let _ = tui.watch_for(Duration::from_secs(3), &|t| {
        t.screen_contains("Switch to buffer")
    });
    cancel(&mut tui, "Switch to buffer");
    tui.write(b"abcdef");
    std::thread::sleep(Duration::from_millis(600));
    let first_session = proxy.mark();
    assert!(first_session >= 8, "the first session sent its events");

    // The daemon dies; another starts on its socket, in its home.
    let socket = daemon.socket_path().to_path_buf();
    let pgid = i32::try_from(daemon.pid()).expect("pid");
    nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), nix::sys::signal::SIGKILL)
        .expect("kill the daemon's group");
    let _ = daemon.wait_for_exit();
    let _ = std::fs::remove_file(&socket);
    let mut second = common::daemon::spawn_daemon_process_with_env(&socket, &[]);
    common::daemon::wait_for_socket_or_exit(&socket, second.child(), Duration::from_secs(10))
        .expect("the second daemon listens");
    // The pump's notice for a session on a daemon that is not the last
    // one: the terminal has attached again, and to the second daemon.
    tui.wait_for(
        Duration::from_secs(40),
        "the terminal attached again, to a new daemon",
        &|t| t.screen_contains("new daemon"),
    );
    assert_eq!(
        proxy.mark(),
        first_session,
        "nothing went to either daemon between the sessions"
    );
    tui.settle(Duration::from_secs(1), PATIENCE, "reattached", &|_| true);
    tui.write(b"\x1b>");
    std::thread::sleep(Duration::from_millis(300));
    tui.write(b"\r");
    tui.settle(
        Duration::from_millis(500),
        PATIENCE,
        "a fresh line",
        &|_| true,
    );

    // The guard in the second session.
    let mark = proxy.mark();
    tui.write(b"\x05");
    std::thread::sleep(Duration::from_millis(300));
    for c in ["x", "y", "z"] {
        tui.write(c.as_bytes());
        std::thread::sleep(Duration::from_millis(150));
    }
    let sent = proxy.wait_sent(mark, 4, "C-e then xyz after the reconnect");
    assert_eq!(
        sent[..4].to_vec(),
        vec![ctrl('e'), Sent::Op, Sent::Op, Sent::Op],
        "after the reconnect, text after an answered motion goes as ops"
    );
    // The witness in the second session.
    for trial in 0..3 {
        proxy.hold_from(|e| sent_of(e) == ctrl('x'), HOLD);
        let (sent, prompt) = c_x_b(&mut tui, &proxy, Some(Duration::from_millis(150)));
        assert!(
            sent == vec![ctrl('x'), plain('b')] && prompt,
            "after the reconnect, trial {trial}: sent {sent:?}, prompt {prompt}"
        );
        cancel(&mut tui, "Switch to buffer");
    }
    drop(second);
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
