//! E10r --- the daemon's end of a frontend's socket, whose every frame is
//! written within a bound (#390).
//!
//! The daemon writes to every frontend from its one dispatcher thread, so a
//! write that waits on a frontend that has stopped reading holds up every
//! other frontend for as long as it waits. A [`FrontendStream`] is what the
//! accept loop wraps a socket in before its first byte, and it has no way to
//! write but [`FrontendStream::write_message`]: a frame that is not whole
//! within [`FRAME_WRITE_BOUND`] of its first byte fails, as
//! [`crate::transport::TransportError::Io`], which is what a failed write
//! already is on E10d's one path: the stream leaves its session, so a
//! part-written frame never gets another byte, and the drain begins. A
//! bootstrap's write that fails this way unregisters its provisional session
//! as any failed one does. There is no `impl Write` here, and the socket is
//! private, so a write past the bound does not compile.
//!
//! The bound is per frame, not per system call. The socket is non-blocking
//! for its whole life, through every clone, and the waiting happens here, in
//! `poll`: a write for what is left of the frame's time, a read for as long as
//! it takes, so the reader thread sees what a blocking socket would show it. A
//! healthy small frame costs one `write` per piece, as a blocking write does,
//! and no wait. Two shorter roads do not work. `set_write_timeout`:
//! `SO_SNDTIMEO` starts again at each wait, inside one `send` as well as
//! between them, so one call writing a large frame to a reader that trickles
//! runs far past it (measured on Linux 7.2: a 200 ms timeout, one 4 MiB `send`,
//! a reader taking 200 KiB every 50 ms, and the call returned whole after a
//! second). `MSG_DONTWAIT` on a blocking socket, which leaves the reader's clone
//! alone: on macOS the write to a socket nobody read never returned (both of CI
//! 41934447652's macOS legs, this module's row running to the job's ceiling).

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use serde::Serialize;

use crate::transport::{TransportError, write_message};

/// How long one frame may take to reach a frontend's socket, from its first
/// byte to its last: at most the hold-up a frontend that stops reading costs
/// the others, once.
///
/// Two seconds, the most the owner's ruling of 2026-11-04 allows. A healthy
/// local frontend takes milliseconds for the largest frame, so this is not
/// about them: a larger bound is what a frontend behind a slow link needs,
/// since a frame that cannot reach it in time can never be delivered at all.
///
/// A constant, as [`crate::daemon::WRITE_FAILURE_DRAIN`] is, for its reason:
/// what a user would tune it for is a trade inside a connection already in
/// trouble, one hitch for every other frontend against dropping a frontend
/// that is only slow.
pub const FRAME_WRITE_BOUND: Duration = Duration::from_secs(2);

/// The daemon's end of one frontend's socket. See the module doc.
#[derive(Debug)]
pub struct FrontendStream {
    stream: UnixStream,
    bound: Duration,
}

impl FrontendStream {
    /// Wrap an accepted socket, before anything is written to it, and make it
    /// non-blocking from here on, whatever it inherited from the listener.
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        Self::with_bound(stream, FRAME_WRITE_BOUND)
    }

    /// [`Self::new`] with another bound, for this module's rows.
    pub fn with_bound(stream: UnixStream, bound: Duration) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self { stream, bound })
    }

    /// Write `message` as one frame, within the bound. A frame past the bound
    /// fails with [`TransportError::Io`] having written part of itself, so the
    /// stream can carry nothing more: the caller stops writing to it, which
    /// E10d's path and the bootstrap both do. A message the transport refuses
    /// writes nothing and leaves the stream whole.
    pub fn write_message<M: Serialize>(&mut self, message: &M) -> Result<(), TransportError> {
        let mut frame = BoundedFrame {
            stream: &self.stream,
            deadline: Instant::now() + self.bound,
            bound: self.bound,
        };
        write_message(&mut frame, message)
    }

    /// Another handle on the same socket, with the same bound: the reader
    /// thread's and the dispatcher's halves.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            stream: self.stream.try_clone()?,
            bound: self.bound,
        })
    }

    /// Shut the socket down, as [`UnixStream::shutdown`].
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.stream.shutdown(how)
    }
}

/// Reads wait as a blocking socket's do, unbounded: a frontend owes the daemon
/// no input, and waiting for it holds up only its own reader thread. A shut
/// socket answers `poll` at once and the read says EOF.
impl Read for FrontendStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.stream.read(buf) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let mut fds = [PollFd::new(self.stream.as_fd(), PollFlags::POLLIN)];
                    match poll(&mut fds, PollTimeout::NONE) {
                        Ok(_) | Err(Errno::EINTR) => {}
                        Err(errno) => return Err(errno.into()),
                    }
                }
                other => return other,
            }
        }
    }
}

impl AsFd for FrontendStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.stream.as_fd()
    }
}

/// One frame's writes, sharing one deadline.
struct BoundedFrame<'a> {
    stream: &'a UnixStream,
    deadline: Instant,
    bound: Duration,
}

impl BoundedFrame<'_> {
    /// Wait for room in the socket for what is left of the frame's time. A
    /// wait that returns early only means the next `write` looks again: a peer
    /// gone is that `write`'s error, and no room is this wait's next round.
    fn wait_for_room(&self) -> io::Result<()> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(self.past_the_bound());
        }
        // Rounded up, so a wait never ends a millisecond before its deadline
        // only to spin; the bound is far under `u16::MAX` milliseconds.
        let millis = u16::try_from(left.as_millis() + 1).unwrap_or(u16::MAX);
        let mut fds = [PollFd::new(self.stream.as_fd(), PollFlags::POLLOUT)];
        match poll(&mut fds, PollTimeout::from(millis)) {
            Ok(0) => Err(self.past_the_bound()),
            Ok(_) | Err(Errno::EINTR) => Ok(()),
            Err(errno) => Err(errno.into()),
        }
    }

    fn past_the_bound(&self) -> io::Error {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "the frame was not written within {} ms",
                self.bound.as_millis()
            ),
        )
    }
}

impl Write for BoundedFrame<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut stream = self.stream;
        loop {
            match stream.write(buf) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => self.wait_for_room()?,
                other => return other,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::read_message;
    use std::thread;

    /// How far past a bound a row allows, for a loaded machine. The trickle
    /// row's whole frame would take four seconds, so this stays under that.
    const SLACK: Duration = Duration::from_secs(2);

    /// A message of `len` payload bytes that encodes in one copy, so a row
    /// timing a write does not time a debug build's byte-by-byte encoder.
    struct Payload(Vec<u8>);

    impl Serialize for Payload {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(&self.0)
        }
    }

    fn big(len: usize) -> Payload {
        Payload(vec![0x5a; len])
    }

    /// The peer of a stream given `bound`.
    fn pair(bound: Duration) -> (FrontendStream, UnixStream) {
        let (ours, theirs) = UnixStream::pair().expect("socketpair");
        (
            FrontendStream::with_bound(ours, bound).expect("non-blocking"),
            theirs,
        )
    }

    #[test]
    fn a_frame_a_reader_takes_arrives_whole_past_the_socket_buffer() {
        let (mut stream, mut peer) = pair(Duration::from_secs(30));
        let sent = big(4 * 1024 * 1024);
        let expect = sent.0.clone();
        let reader = thread::spawn(move || read_message::<Vec<u8>>(&mut peer).expect("read"));
        stream.write_message(&sent).expect("write");
        assert_eq!(reader.join().expect("reader"), expect);
    }

    #[test]
    fn a_frame_nobody_reads_fails_at_the_bound_and_no_later() {
        let bound = Duration::from_millis(300);
        let (mut stream, _peer) = pair(bound);
        let started = Instant::now();
        let error = stream
            .write_message(&big(4 * 1024 * 1024))
            .expect_err("nobody reads");
        let took = started.elapsed();
        match &error {
            TransportError::Io(io) => assert_eq!(io.kind(), io::ErrorKind::TimedOut, "{io}"),
            other => panic!("not an I/O failure: {other:?}"),
        }
        assert!(took >= bound, "failed early: {took:?}");
        assert!(
            took < bound + SLACK,
            "failed {took:?} into a {bound:?} bound"
        );
    }

    /// The row `set_write_timeout` fails: a reader that always makes enough
    /// room for one more wait renews no deadline here.
    #[test]
    fn a_reader_that_trickles_renews_nothing() {
        let bound = Duration::from_millis(400);
        let (mut stream, mut peer) = pair(bound);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_stop = std::sync::Arc::clone(&stop);
        let reader = thread::spawn(move || {
            let mut chunk = vec![0u8; 200 * 1024];
            peer.set_read_timeout(Some(Duration::from_millis(50)))
                .expect("timeout");
            while !reader_stop.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = peer.read(&mut chunk);
                thread::sleep(Duration::from_millis(50));
            }
        });
        // 16 MiB at 4 MiB a second would take four seconds; the bound is 400 ms.
        let started = Instant::now();
        let error = stream
            .write_message(&big(16 * 1024 * 1024 - 64))
            .expect_err("the trickle is too slow for the bound");
        let took = started.elapsed();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        reader.join().expect("reader");
        assert!(
            matches!(&error, TransportError::Io(io) if io.kind() == io::ErrorKind::TimedOut),
            "{error:?}"
        );
        assert!(took < bound + SLACK, "held {took:?} on a {bound:?} bound");
    }

    /// The socket is non-blocking and the reader thread must not see it: a read
    /// waits for its data, and a socket shut by its peer says EOF.
    #[test]
    fn a_read_waits_for_its_frame_and_says_eof_at_the_end() {
        let (mut stream, mut peer) = pair(Duration::from_secs(30));
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            write_message(&mut peer, &vec![1u8, 2, 3]).expect("write");
        });
        assert_eq!(
            read_message::<Vec<u8>>(&mut stream).expect("waited for the frame"),
            vec![1u8, 2, 3]
        );
        writer.join().expect("writer");
        assert!(
            matches!(
                read_message::<Vec<u8>>(&mut stream),
                Err(TransportError::Eof)
            ),
            "the peer's end closed with the writer"
        );
    }

    #[test]
    fn a_peer_gone_fails_as_a_socket_error_at_once() {
        let (mut stream, peer) = pair(Duration::from_secs(30));
        drop(peer);
        let started = Instant::now();
        let error = stream.write_message(&big(16)).expect_err("peer is gone");
        match &error {
            TransportError::Io(io) => assert_eq!(io.kind(), io::ErrorKind::BrokenPipe, "{io}"),
            other => panic!("not an I/O failure: {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_frame_past_the_wire_cap_is_refused_before_a_byte() {
        let (mut stream, mut peer) = pair(Duration::from_secs(30));
        let error = stream
            .write_message(&big(pmacs_protocol::MAX_FRAME_BYTES + 1))
            .expect_err("past the frame");
        assert!(
            matches!(error, TransportError::FrameTooLarge { .. }),
            "{error:?}"
        );
        peer.set_nonblocking(true).expect("nonblocking");
        let mut one = [0u8; 1];
        assert_eq!(
            peer.read(&mut one).expect_err("nothing written").kind(),
            io::ErrorKind::WouldBlock
        );
        stream.write_message(&big(8)).expect("the stream is whole");
        peer.set_nonblocking(false).expect("blocking");
        assert_eq!(read_message::<Vec<u8>>(&mut peer).expect("read"), big(8).0);
    }
}
