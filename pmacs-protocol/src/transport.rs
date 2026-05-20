// transport.rs --- Length-prefix postcard codec.

//! Length-prefix framed postcard codec for the M5.5 frontend ↔ instance
//! protocol (T M5.5b).
//!
//! # Wire format
//!
//! Each message is encoded as:
//!
//! ```text
//! [u32 big-endian length][postcard bytes]
//! ```
//!
//! - The length is the byte count of the postcard payload that follows.
//!   Zero-length payloads are valid (postcard encodes some types as
//!   empty byte strings).
//! - Length values exceeding [`MAX_FRAME_BYTES`] are rejected without
//!   allocation on the read side, and refused before any bytes hit the
//!   wire on the write side. This caps both worst-case allocation and
//!   the maximum legitimate message size — large payloads should be
//!   chunked at a higher layer.
//!
//! # Encoding choice
//!
//! Postcard is a Serde-driven, no-std-friendly format chosen for its
//! compactness on the cell-stream traffic (60 Hz cell-delta frames
//! dominate the wire) and for the future option of a thin attach
//! client without `tokio`. Schema evolution is handled by an explicit
//! version handshake rather than the encoding itself; see
//! [`crate::PROTOCOL_VERSION`].
//!
//! The worker-protocol encoding (spec §5.5) remains `MessagePack` via
//! `rmp-serde`; that subsystem values schema flexibility over wire
//! compactness.

use serde::{Serialize, de::DeserializeOwned};
use std::io::{Read, Write};

/// Maximum legitimate frame payload size, in bytes.
///
/// 16 MiB. Comfortably above any single cell-delta frame in v0.1: a
/// full 4K terminal at 60 Hz with truecolor styling fits well under a
/// megabyte per frame. Frames larger than this are presumed bugs or
/// hostile peers and are rejected.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Errors produced by [`read_message`] and [`write_message`].
#[derive(Debug)]
pub enum TransportError {
    /// Underlying I/O failed (broken pipe, connection reset, etc.).
    Io(std::io::Error),
    /// Postcard refused to encode the message (typically a `Serialize`
    /// implementation returning an error).
    Encode(postcard::Error),
    /// Postcard refused to decode the bytes — malformed payload from
    /// peer, or peer running an incompatible message shape that
    /// slipped past the version handshake.
    Decode(postcard::Error),
    /// Advertised or computed frame length exceeded [`MAX_FRAME_BYTES`].
    FrameTooLarge {
        /// The length the peer advertised (read side) or the size of
        /// the encoded payload (write side).
        len: usize,
    },
    /// Peer disconnected before a full frame could be read. The same
    /// error is returned for "EOF before any bytes," "EOF mid
    /// length-prefix," and "EOF mid payload"; the caller treats these
    /// identically.
    Eof,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "transport I/O error: {e}"),
            Self::Encode(e) => write!(f, "transport encode error: {e}"),
            Self::Decode(e) => write!(f, "transport decode error: {e}"),
            Self::FrameTooLarge { len } => {
                write!(f, "frame length {len} exceeds maximum {MAX_FRAME_BYTES}")
            }
            Self::Eof => write!(f, "peer disconnected before frame complete"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Encode(e) | Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Read a single framed message from `reader`.
///
/// Returns [`TransportError::Eof`] if the peer disconnected before a
/// full frame arrived (whether at the start, mid length-prefix, or
/// mid payload — all three look identical to the caller).
pub fn read_message<M: DeserializeOwned>(reader: &mut impl Read) -> Result<M, TransportError> {
    let mut len_buf = [0u8; 4];
    read_exact_or_eof(reader, &mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(TransportError::FrameTooLarge { len });
    }
    let mut buf = vec![0u8; len];
    read_exact_or_eof(reader, &mut buf)?;
    postcard::from_bytes(&buf).map_err(TransportError::Decode)
}

/// Write a single framed message to `writer`.
///
/// Returns [`TransportError::FrameTooLarge`] if the encoded form
/// exceeds [`MAX_FRAME_BYTES`]; in that case no bytes are written.
pub fn write_message<M: Serialize>(writer: &mut impl Write, msg: &M) -> Result<(), TransportError> {
    let payload = postcard::to_allocvec(msg).map_err(TransportError::Encode)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(TransportError::FrameTooLarge { len: payload.len() });
    }
    let len = u32::try_from(payload.len()).expect("payload length bounded by MAX_FRAME_BYTES");
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&payload)?;
    Ok(())
}

/// Fill `buf` from `reader`, returning [`TransportError::Eof`] if the
/// peer disconnects before the buffer is full. Retries on
/// [`std::io::ErrorKind::Interrupted`].
///
/// `std::io::Read::read_exact` collapses both "read 0 bytes" and "read
/// some-but-not-all" into `ErrorKind::UnexpectedEof`, but it is not
/// guaranteed to retry on `Interrupted`. This helper makes both
/// behaviors explicit.
fn read_exact_or_eof(reader: &mut impl Read, buf: &mut [u8]) -> Result<(), TransportError> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => return Err(TransportError::Eof),
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(TransportError::Io(e)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AttachRequest, FrontendCapabilities, FrontendEvent, FrontendId, Hello,
        InstanceCapabilities, InstanceIdentity, Key, KeyEvent, Modifiers, PROTOCOL_VERSION,
    };
    use std::io::Cursor;

    fn round_trip<M: Serialize + DeserializeOwned + std::fmt::Debug + PartialEq>(msg: &M) {
        let mut buf = Vec::new();
        write_message(&mut buf, msg).expect("write");
        let mut cursor = Cursor::new(buf);
        let decoded: M = read_message(&mut cursor).expect("read");
        assert_eq!(&decoded, msg);
    }

    #[test]
    fn hello_round_trips_through_transport() {
        let h = Hello {
            protocol_version: PROTOCOL_VERSION,
            assigned_frontend_id: FrontendId(2),
            instance_identity: InstanceIdentity {
                pmacs_version: "0.1.0".into(),
                build_hash: None,
                instance_name: None,
                uptime_secs: 12,
                working_directory: "/tmp".into(),
            },
            instance_capabilities: InstanceCapabilities::default(),
        };
        round_trip(&h);
    }

    #[test]
    fn attach_request_round_trips_through_transport() {
        let req = AttachRequest {
            protocol_version: PROTOCOL_VERSION,
            frontend_capabilities: FrontendCapabilities {
                synchronized_output: true,
                unicode_smp: true,
                true_color: true,
                mouse: true,
                bracketed_paste: true,
                terminal_kind: Some("xterm-256color".into()),
                multi_frontend: false,
                crdt_replica: false,
                semantic_render: false,
            },
            initial_size: crate::cell::CellSize::new(24, 80),
        };
        round_trip(&req);
    }

    #[test]
    fn key_event_round_trips_through_transport() {
        let ev = FrontendEvent::Key(KeyEvent {
            frontend_id: FrontendId(2),
            key: Key::Char('a'),
            mods: Modifiers::CTRL,
            timestamp_ns: 0,
        });
        round_trip(&ev);
    }

    #[test]
    fn empty_input_returns_eof() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        match read_message::<Hello>(&mut cursor) {
            Err(TransportError::Eof) => {}
            other => panic!("expected Eof, got {other:?}"),
        }
    }

    #[test]
    fn truncated_length_prefix_returns_eof() {
        // Two bytes of a four-byte length prefix.
        let mut cursor = Cursor::new(vec![0x00, 0x10]);
        match read_message::<Hello>(&mut cursor) {
            Err(TransportError::Eof) => {}
            other => panic!("expected Eof, got {other:?}"),
        }
    }

    #[test]
    fn truncated_payload_returns_eof() {
        // Length advertises 100 bytes; only 5 follow.
        let mut bytes = 100u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05]);
        let mut cursor = Cursor::new(bytes);
        match read_message::<Hello>(&mut cursor) {
            Err(TransportError::Eof) => {}
            other => panic!("expected Eof, got {other:?}"),
        }
    }

    #[test]
    fn frame_larger_than_max_rejected_without_allocating() {
        // Advertise MAX_FRAME_BYTES + 1; we expect rejection before any
        // body bytes are read.
        let len = u32::try_from(MAX_FRAME_BYTES + 1).expect("fits in u32");
        let bytes = len.to_be_bytes().to_vec();
        let mut cursor = Cursor::new(bytes);
        match read_message::<Hello>(&mut cursor) {
            Err(TransportError::FrameTooLarge { len: l }) => {
                assert_eq!(l, MAX_FRAME_BYTES + 1);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn frame_at_exact_max_size_passes_length_check() {
        // Advertise exactly MAX_FRAME_BYTES — the boundary case must
        // not be rejected by the length check. We don't actually have
        // a payload this large; we expect Eof from the body fetch,
        // which proves the length check passed.
        let len = u32::try_from(MAX_FRAME_BYTES).expect("fits in u32");
        let bytes = len.to_be_bytes().to_vec();
        let mut cursor = Cursor::new(bytes);
        match read_message::<Hello>(&mut cursor) {
            Err(TransportError::Eof) => {}
            other => panic!("expected Eof at MAX_FRAME_BYTES boundary, got {other:?}"),
        }
    }

    #[test]
    fn bad_postcard_bytes_return_decode_error() {
        // Length prefix says 8, payload is garbage bytes that do not
        // decode as a Hello.
        let payload = vec![0xFFu8; 8];
        let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(&payload);
        let mut cursor = Cursor::new(bytes);
        match read_message::<Hello>(&mut cursor) {
            Err(TransportError::Decode(_)) => {}
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn multiple_messages_back_to_back() {
        // Two messages share one buffer; framing must not leak state
        // between them.
        let h1 = FrontendEvent::Detach(FrontendId(1));
        let h2 = FrontendEvent::Detach(FrontendId(2));
        let mut buf = Vec::new();
        write_message(&mut buf, &h1).expect("write 1");
        write_message(&mut buf, &h2).expect("write 2");
        let mut cursor = Cursor::new(buf);
        let d1: FrontendEvent = read_message(&mut cursor).expect("read 1");
        let d2: FrontendEvent = read_message(&mut cursor).expect("read 2");
        match d1 {
            FrontendEvent::Detach(id) => assert_eq!(id, FrontendId(1)),
            other => panic!("expected Detach(1), got {other:?}"),
        }
        match d2 {
            FrontendEvent::Detach(id) => assert_eq!(id, FrontendId(2)),
            other => panic!("expected Detach(2), got {other:?}"),
        }
    }

    #[test]
    fn read_after_consuming_only_message_returns_eof() {
        let h = FrontendEvent::Detach(FrontendId(7));
        let mut buf = Vec::new();
        write_message(&mut buf, &h).expect("write");
        let mut cursor = Cursor::new(buf);
        let _: FrontendEvent = read_message(&mut cursor).expect("read");
        match read_message::<FrontendEvent>(&mut cursor) {
            Err(TransportError::Eof) => {}
            other => panic!("expected Eof after consuming the only message, got {other:?}"),
        }
    }
}
