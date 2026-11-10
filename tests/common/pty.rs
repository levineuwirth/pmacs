//! Real-PTY pmacs spawner shared across integration tests.
//!
//! `PmacsPty` owns a real PTY pair plus a pmacs child running
//! inside it; a background reader thread drains the master so
//! pmacs's writes never block on a full terminal output buffer.
//! `spawn_pmacs_in_pty` is the constructor.
//!
//! First consumer: M5.8 session-reconnect tests
//! (`tests/m5_8_acceptance.rs`). Second consumer: M10.11
//! doubled-PTY two-laptop tests (`tests/m10_11_acceptance.rs`).

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pmacs::ansi::{AnsiParser, AnsiParserProfile};
use pmacs::cell::{CellCoord, CellSize, Glyph};
use pmacs::terminal::screen::TerminalScreen;
use portable_pty::{CommandBuilder, PtySize};

/// Pmacs spawned inside a real PTY. Holds the master so the slave
/// stays alive; `child` is reapable via `try_wait`. The writer
/// allows the test to inject keystrokes (e.g. `\x03` for Ctrl-C);
/// the reader is kept around so the slave's output buffer doesn't
/// fill and block pmacs's writes.
pub struct PmacsPty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    reader_thread: thread::JoinHandle<()>,
    output: Arc<Mutex<Vec<u8>>>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

impl PmacsPty {
    /// Inject bytes into pmacs's stdin via the PTY master.
    pub fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Resize the real host PTY in terminal cells.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())
    }

    /// Snapshot all bytes emitted by pmacs to its host terminal.
    #[must_use]
    pub fn output(&self) -> Vec<u8> {
        self.output
            .lock()
            .expect("PTY output mutex poisoned")
            .clone()
    }

    /// Poll-wait for pmacs to exit, up to `timeout`. Returns the
    /// exit status on success, `None` on timeout (and leaves the
    /// child running for the caller to clean up).
    pub fn wait_for_exit(&mut self, timeout: Duration) -> Option<portable_pty::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => {}
                Err(_) => return None,
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait until `needle` is in what pmacs wrote to its terminal, or say what it
    /// wrote instead.
    pub fn wait_for_output(&self, needle: &str, patience: Duration) {
        let deadline = Instant::now() + patience;
        loop {
            let output = String::from_utf8_lossy(&self.output()).into_owned();
            if output.contains(needle) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{needle:?} never reached the terminal, which was given: {:?}",
                output.chars().rev().take(400).collect::<String>()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Everything pmacs wrote, for a row that has seen the child exit: the reader thread may
    /// not have the last of it yet, so this waits for that thread to reach the terminal's
    /// end, under a limit, since something the child left behind may hold the terminal open
    /// past it (#485). [`Self::output`] is what has been read so far.
    pub fn output_once_exited(&self) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.reader_thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        self.output()
    }

    /// What pmacs wrote after it last left the alternate screen: what its user
    /// is left looking at, which is where a reason has to be to be read. A row calls
    /// this having seen the child exit, so it reads [`Self::output_once_exited`].
    pub fn after_the_alternate_screen(&self) -> String {
        let output = self.output_once_exited();
        let leave = b"\x1b[?1049l";
        let at = output
            .windows(leave.len())
            .rposition(|w| w == leave)
            .expect("it left the alternate screen");
        String::from_utf8_lossy(&output[at + leave.len()..]).into_owned()
    }

    /// The OS process id of the running pmacs child, if portable-pty
    /// can surface it. Used by M10.11's Drop-discipline test to
    /// verify post-drop process death via `kill(pid, 0)`.
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }
}

impl Drop for PmacsPty {
    fn drop(&mut self) {
        // Best-effort cleanup if the test panicked / timed out.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Attach a real TUI to `socket` in a PTY, `home` its home and `envs` on top.
pub fn attached_tui(socket: &Path, home: &Path, envs: &[(&str, &Path)]) -> PmacsPty {
    let mut all: Vec<(&str, &Path)> = vec![("HOME", home), ("XDG_CONFIG_HOME", home)];
    all.extend_from_slice(envs);
    spawn_pmacs_in_pty(
        &["--attach", "--socket", socket.to_str().expect("utf-8")],
        &all,
        24,
        80,
    )
}

/// Spawn pmacs inside a fresh PTY pair, returning a handle that
/// owns the master + child + reader-thread. The reader thread
/// drains the master so pmacs's writes never block on a full
/// terminal buffer.
pub fn spawn_pmacs_in_pty(args: &[&str], envs: &[(&str, &Path)], rows: u16, cols: u16) -> PmacsPty {
    spawn_pmacs_in_pty_under(&[], args, envs, rows, cols)
}

/// [`spawn_pmacs_in_pty`], with pmacs run by `wrapper`: its first word
/// is the program the PTY runs, the rest that program's arguments,
/// which pmacs's path and `args` follow. An empty `wrapper` runs pmacs
/// itself. The PTY's child, the process [`PmacsPty::wait_for_exit`] and
/// [`PmacsPty::process_id`] name, is the wrapper.
pub fn spawn_pmacs_in_pty_under(
    wrapper: &[&str],
    args: &[&str],
    envs: &[(&str, &Path)],
    rows: u16,
    cols: u16,
) -> PmacsPty {
    spawn_tapped(wrapper, args, envs, rows, cols, None)
}

/// `program` and its arguments in a fresh PTY with no pmacs on its command line: a real
/// shell, say, into which a row types a command as a user does. Its `envs` are as
/// [`spawn_pmacs_in_pty`]'s, and what it runs is the row's to name.
#[cfg(target_os = "linux")]
pub fn spawn_shell_in_pty(
    program: &[&str],
    envs: &[(&str, &Path)],
    rows: u16,
    cols: u16,
) -> PmacsPty {
    spawn_program(program, envs, rows, cols, None)
}

/// What pmacs wrote to its terminal, one read of the master at a time,
/// each stamped with the instant the reader thread had it: the second
/// half of [`spawn_pmacs_in_pty_tapped`]. A row timing what reaches the
/// screen reads this rather than polling [`PmacsPty::output`], which
/// copies everything written so far and says nothing of when.
pub type OutputTap = std::sync::mpsc::Receiver<(Instant, Vec<u8>)>;

/// [`spawn_pmacs_in_pty`], with every read of the master also sent,
/// stamped, through the returned [`OutputTap`] (E10k.1's timing rows).
/// [`PmacsPty::output`] still accumulates the whole stream.
pub fn spawn_pmacs_in_pty_tapped(
    args: &[&str],
    envs: &[(&str, &Path)],
    rows: u16,
    cols: u16,
) -> (PmacsPty, OutputTap) {
    let (tx, rx) = std::sync::mpsc::channel();
    (spawn_tapped(&[], args, envs, rows, cols, Some(tx)), rx)
}

fn spawn_tapped(
    wrapper: &[&str],
    args: &[&str],
    envs: &[(&str, &Path)],
    rows: u16,
    cols: u16,
    tap: Option<std::sync::mpsc::Sender<(Instant, Vec<u8>)>>,
) -> PmacsPty {
    let pmacs = env!("CARGO_BIN_EXE_pmacs");
    let mut program: Vec<&str> = wrapper.to_vec();
    program.push(pmacs);
    program.extend_from_slice(args);
    spawn_program(&program, envs, rows, cols, tap)
}

fn spawn_program(
    program: &[&str],
    envs: &[(&str, &Path)],
    rows: u16,
    cols: u16,
    tap: Option<std::sync::mpsc::Sender<(Instant, Vec<u8>)>>,
) -> PmacsPty {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let (first, rest) = program.split_first().expect("a program to run");
    let mut cmd = CommandBuilder::new(first);
    for arg in rest {
        cmd.arg(arg);
    }
    // Backfill any of the five storage variables the caller did not set
    // (the archived test-ambient-config-isolation framing §1.6a). Callers
    // pin the roots their assertions read (a per-test `XDG_CONFIG_HOME`,
    // a `PMACS_STATE_HOME` they inspect); the rest would otherwise be
    // inherited, and the child would materialize bundled packages into
    // the developer's real data root. Caller entries win: this only
    // fills holes.
    let fallback = pmacs::bootstrap::BootstrapRoots::isolated_under(&super::iso::base());
    for (key, value) in fallback.child_env() {
        if envs.iter().any(|(k, _)| *k == key) {
            continue;
        }
        std::fs::create_dir_all(&value).expect("create isolated root for PTY child");
        cmd.env(key, value.as_os_str());
    }
    for (k, v) in envs {
        let mut value = OsString::new();
        value.push(v);
        cmd.env(k, value);
    }

    let child = pair.slave.spawn_command(cmd).expect("spawn pmacs");
    // Drop the slave on our side so EOF on the master is detectable
    // when the child exits (pmacs's stdio is the only thing keeping
    // the slave alive after this).
    drop(pair.slave);

    let writer = pair.master.take_writer().expect("take_writer");
    let mut reader = pair.master.try_clone_reader().expect("try_clone_reader");
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = output.clone();
    // Drain and retain reader bytes so terminal-mode restoration can be
    // asserted without ever exposing pmacs to the test runner's own TTY.
    let reader_thread = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    let at = Instant::now();
                    captured
                        .lock()
                        .expect("PTY output mutex poisoned")
                        .extend_from_slice(&buf[..read]);
                    if let Some(tap) = &tap {
                        // A row that stopped listening is not this
                        // thread's failure; keep draining.
                        let _ = tap.send((at, buf[..read].to_vec()));
                    }
                }
            }
        }
    });

    PmacsPty {
        child,
        writer,
        reader_thread,
        output,
        master: pair.master,
    }
}

/// A TUI in a PTY with its screen modelled from what it writes, since a TUI writes
/// only the cells that changed, so a needle in its bytes can miss text on screen.
/// Extracted at E10r's fix round 2 (D37) from `tests/e10k_large_buffer_typing.rs`.
pub struct ScreenTui {
    pub pty: PmacsPty,
    tap: OutputTap,
    parser: AnsiParser,
    screen: TerminalScreen,
}

impl ScreenTui {
    #[must_use]
    pub fn new(pty: PmacsPty, tap: OutputTap, rows: u16, cols: u16) -> Self {
        Self {
            pty,
            tap,
            // The full-screen profile: the default one is a compile
            // buffer's, which keeps no cursor.
            parser: AnsiParser::with_profile(AnsiParserProfile::FullScreen),
            screen: TerminalScreen::new(CellSize::new(rows.into(), cols.into()), 0)
                .expect("a valid screen"),
        }
    }

    /// Apply `bytes`; whether the published screen or caret changed. A
    /// TUI writes a frame every tick whether or not anything moved, so
    /// a write is not a change.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        let before = self.screen.snapshot();
        for event in self.parser.feed(bytes) {
            let _ = self.screen.apply_event(event);
        }
        let after = self.screen.snapshot();
        before.cells != after.cells || before.cursor != after.cursor
    }

    /// The published character at `(row, col)`.
    pub fn char_at(&self, row: u32, col: u32) -> Option<char> {
        let snap = self.screen.snapshot();
        let cell = snap.cells.get((row * snap.size.cols + col) as usize)?;
        match cell.glyph {
            Glyph::Char(c) => Some(c),
            _ => None,
        }
    }

    pub fn cursor(&self) -> Option<CellCoord> {
        self.screen.snapshot().cursor
    }

    /// Whether `text` stands at `(row, col)` onwards.
    pub fn shows(&self, row: u32, col: u32, text: &str) -> bool {
        text.chars()
            .zip(col..)
            .all(|(ch, c)| self.char_at(row, c) == Some(ch))
    }

    /// The published screen's rows as text.
    pub fn rows(&self) -> Vec<String> {
        let snap = self.screen.snapshot();
        snap.cells
            .chunks(snap.size.cols as usize)
            .map(|row| {
                row.iter()
                    .map(|c| match c.glyph {
                        Glyph::Char(ch) => ch,
                        _ => ' ',
                    })
                    .collect()
            })
            .collect()
    }

    /// The bottom row, as text with its trailing blanks gone.
    pub fn bottom_row(&self) -> String {
        self.rows()
            .last()
            .map_or_else(String::new, |row| row.trim_end().to_owned())
    }

    /// Any row holds `needle`.
    pub fn screen_contains(&self, needle: &str) -> bool {
        self.rows().iter().any(|row| row.contains(needle))
    }

    /// The column the modeline reports (`L<line>:C<col>`): the editor's
    /// own word for where its caret is, which an attached TUI hears from
    /// the daemon, so it confirms what the TUI painted on its own.
    pub fn modeline_col(&self) -> Option<u32> {
        self.rows().iter().rev().find_map(|row| {
            let (_, after) = row.split_once(":C")?;
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
    }

    /// What the row saw, for a failure: the screen, the caret, and the
    /// last of what the TUI wrote.
    pub fn describe(&self) -> String {
        let out = self.pty.output();
        let tail = &out[out.len().saturating_sub(600)..];
        let dump = std::env::temp_dir().join(format!("screen-tui-{}.out", std::process::id()));
        let _ = std::fs::write(&dump, &out);
        format!(
            "caret {:?}, {} bytes written (all of them in {}); screen:\n{}\nlast bytes: {:?}",
            self.cursor(),
            out.len(),
            dump.display(),
            self.rows().join("\n"),
            String::from_utf8_lossy(tail)
        )
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.pty.write_input(bytes).expect("write to the PTY");
    }

    /// Resize the PTY, as a user resizing the terminal does, and the modelled screen with
    /// it, after reading for 200 ms what the TUI wrote at the old size.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let _ = self.watch_for(Duration::from_millis(200), &|_| false);
        self.screen
            .resize(CellSize::new(rows.into(), cols.into()))
            .expect("a valid size");
        self.pty.resize(rows, cols).expect("resize the PTY");
    }

    /// Read until `done` holds; the arrival of the read that made it
    /// hold. Panics after `limit`.
    pub fn wait_for(
        &mut self,
        limit: Duration,
        what: &str,
        done: &dyn Fn(&Self) -> bool,
    ) -> Instant {
        let start = Instant::now();
        if done(self) {
            return start;
        }
        loop {
            let received = limit
                .checked_sub(start.elapsed())
                .and_then(|left| self.tap.recv_timeout(left).ok());
            let Some((at, bytes)) = received else {
                panic!("{what}: not on screen after {limit:?}; {}", self.describe());
            };
            self.feed(&bytes);
            if done(self) {
                return at;
            }
        }
    }

    /// Read for at most `limit` until `done` holds; the arrival of the read that made
    /// it hold, or `None`, with nothing panicking, for a row that keys while it reads.
    pub fn watch_for(&mut self, limit: Duration, done: &dyn Fn(&Self) -> bool) -> Option<Instant> {
        let start = Instant::now();
        if done(self) {
            return Some(start);
        }
        while let Some(left) = limit.checked_sub(start.elapsed()) {
            let Ok((at, bytes)) = self.tap.recv_timeout(left) else {
                return None;
            };
            self.feed(&bytes);
            if done(self) {
                return Some(at);
            }
        }
        None
    }

    /// Read until `done` holds and the screen has not changed for `quiet`.
    pub fn settle(
        &mut self,
        quiet: Duration,
        limit: Duration,
        what: &str,
        done: &dyn Fn(&Self) -> bool,
    ) {
        let start = Instant::now();
        let mut last = Instant::now();
        loop {
            if done(self) && last.elapsed() >= quiet {
                return;
            }
            assert!(
                start.elapsed() < limit,
                "{what}: not settled after {limit:?}; {}",
                self.describe()
            );
            if let Ok((_, bytes)) = self.tap.recv_timeout(Duration::from_millis(10))
                && self.feed(&bytes)
            {
                last = Instant::now();
            }
        }
    }
}
