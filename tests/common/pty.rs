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

use portable_pty::{CommandBuilder, PtySize};

/// Pmacs spawned inside a real PTY. Holds the master so the slave
/// stays alive; `child` is reapable via `try_wait`. The writer
/// allows the test to inject keystrokes (e.g. `\x03` for Ctrl-C);
/// the reader is kept around so the slave's output buffer doesn't
/// fill and block pmacs's writes.
pub struct PmacsPty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    _reader_thread: thread::JoinHandle<()>,
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

/// Spawn pmacs inside a fresh PTY pair, returning a handle that
/// owns the master + child + reader-thread. The reader thread
/// drains the master so pmacs's writes never block on a full
/// terminal buffer.
pub fn spawn_pmacs_in_pty(args: &[&str], envs: &[(&str, &Path)], rows: u16, cols: u16) -> PmacsPty {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_pmacs"));
    for arg in args {
        cmd.arg(arg);
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
                Ok(read) => captured
                    .lock()
                    .expect("PTY output mutex poisoned")
                    .extend_from_slice(&buf[..read]),
            }
        }
    });

    PmacsPty {
        child,
        writer,
        _reader_thread: reader_thread,
        output,
        master: pair.master,
    }
}
