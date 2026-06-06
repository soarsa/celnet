//! Length-prefixed framing and the leader↔follower message types, over real
//! blocking `std::net::TcpStream` sockets.
//!
//! The transport is deliberately the simplest honest real-socket transport: a
//! blocking [`std::net::TcpStream`] with **4-byte big-endian length-prefixed**
//! frames. Each frame is one [`Message`]. There is no shared-memory shortcut —
//! a follower process talks to the leader process strictly through bytes on a
//! loopback TCP connection, exactly as a cross-host deployment would (the only
//! difference being the route the kernel takes; see the crate-root honest
//! boundary on what loopback does and does not prove).

use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::entry::LogEntry;

/// Maximum accepted frame length (64 MiB), guarding the read-side allocation
/// against a corrupt/hostile length prefix — mirrors the journal's payload bound.
pub const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;

/// A protocol message exchanged between a leader and a follower.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Leader → follower: append this entry to your journal in index order.
    ///
    /// `leader_commit` piggybacks the leader's current commit index so a
    /// follower can advance its own applied/commit watermark for already-quorum-
    /// committed entries without a second round trip.
    Append {
        /// The entry to durably append.
        entry: LogEntry,
        /// The leader's commit index at send time (entries `<=` this are
        /// quorum-committed and safe to apply).
        leader_commit: u64,
    },
    /// Follower → leader: I have **durably** appended every entry up to and
    /// including `match_index` (in this `term`). The leader counts these acks to
    /// advance the commit index on a majority.
    Ack {
        /// The follower's current durable high-water index.
        match_index: u64,
        /// The term the follower is acknowledging under (rejects stale leaders).
        term: u64,
    },
    /// Leader → follower: advance your applied/commit watermark to
    /// `leader_commit` (no new entry). Sent after a quorum-commit so a follower
    /// applies the just-committed tail (the entry whose own Append predated the
    /// commit it enabled). The follower replies [`Message::Ack`].
    Commit {
        /// The leader's commit index (entries `<=` this are quorum-committed).
        leader_commit: u64,
    },
    /// Either direction: a request for the peer's durable high-water index, used
    /// by a recovering/standby node to learn how far behind it is.
    StatusRequest,
    /// Reply to [`Message::StatusRequest`]: the responder's durable high-water
    /// index and current term.
    Status {
        /// The responder's durable high-water (last appended) index, or
        /// `u64::MAX` sentinel meaning "empty log".
        last_index: u64,
        /// The responder's current term.
        term: u64,
    },
}

/// Sentinel `last_index` meaning the log is empty (no entries yet).
pub const EMPTY_LOG: u64 = u64::MAX;

impl Message {
    /// Encode to deterministic bytes (1-byte tag + fields).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Message::Append {
                entry,
                leader_commit,
            } => {
                buf.push(0u8);
                buf.extend_from_slice(&leader_commit.to_le_bytes());
                let e = entry.encode();
                buf.extend_from_slice(&(e.len() as u32).to_le_bytes());
                buf.extend_from_slice(&e);
            }
            Message::Ack { match_index, term } => {
                buf.push(1u8);
                buf.extend_from_slice(&match_index.to_le_bytes());
                buf.extend_from_slice(&term.to_le_bytes());
            }
            Message::StatusRequest => buf.push(2u8),
            Message::Status { last_index, term } => {
                buf.push(3u8);
                buf.extend_from_slice(&last_index.to_le_bytes());
                buf.extend_from_slice(&term.to_le_bytes());
            }
            Message::Commit { leader_commit } => {
                buf.push(4u8);
                buf.extend_from_slice(&leader_commit.to_le_bytes());
            }
        }
        buf
    }

    /// Decode from bytes produced by [`Message::encode`].
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Malformed`] on an unknown tag, a short buffer, or a
    /// nested [`LogEntry`] that fails its own CRC/length check.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let (&tag, rest) = bytes.split_first().ok_or(WireError::Malformed)?;
        match tag {
            0 => {
                if rest.len() < 12 {
                    return Err(WireError::Malformed);
                }
                let leader_commit = u64::from_le_bytes(rest[0..8].try_into().expect("8"));
                let elen = u32::from_le_bytes(rest[8..12].try_into().expect("4")) as usize;
                let start = 12usize;
                let end = start.checked_add(elen).ok_or(WireError::Malformed)?;
                if rest.len() < end {
                    return Err(WireError::Malformed);
                }
                let entry =
                    LogEntry::decode(&rest[start..end]).map_err(|_| WireError::Malformed)?;
                Ok(Message::Append {
                    entry,
                    leader_commit,
                })
            }
            1 => {
                if rest.len() < 16 {
                    return Err(WireError::Malformed);
                }
                Ok(Message::Ack {
                    match_index: u64::from_le_bytes(rest[0..8].try_into().expect("8")),
                    term: u64::from_le_bytes(rest[8..16].try_into().expect("8")),
                })
            }
            2 => Ok(Message::StatusRequest),
            3 => {
                if rest.len() < 16 {
                    return Err(WireError::Malformed);
                }
                Ok(Message::Status {
                    last_index: u64::from_le_bytes(rest[0..8].try_into().expect("8")),
                    term: u64::from_le_bytes(rest[8..16].try_into().expect("8")),
                })
            }
            4 => {
                if rest.len() < 8 {
                    return Err(WireError::Malformed);
                }
                Ok(Message::Commit {
                    leader_commit: u64::from_le_bytes(rest[0..8].try_into().expect("8")),
                })
            }
            _ => Err(WireError::Malformed),
        }
    }
}

/// A transport-layer error.
#[derive(Debug)]
pub enum WireError {
    /// An underlying socket IO error (including a clean peer disconnect, which
    /// surfaces as [`io::ErrorKind::UnexpectedEof`]).
    Io(io::Error),
    /// A frame whose declared length exceeds [`MAX_FRAME_LEN`].
    FrameTooLarge(u32),
    /// A frame whose bytes did not decode to a valid [`Message`].
    Malformed,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Io(e) => write!(f, "wire io: {e}"),
            WireError::FrameTooLarge(n) => write!(f, "wire frame too large: {n}"),
            WireError::Malformed => write!(f, "wire frame malformed"),
        }
    }
}

impl std::error::Error for WireError {}

impl From<io::Error> for WireError {
    fn from(e: io::Error) -> Self {
        WireError::Io(e)
    }
}

/// Write one length-prefixed [`Message`] frame to `stream` and flush it.
///
/// The frame is `len: u32 BE || message-bytes`. Flushing before returning means
/// the bytes have left this process's buffers to the kernel.
///
/// # Errors
///
/// [`WireError::Io`] on a socket write failure, [`WireError::FrameTooLarge`] if
/// the encoded message exceeds [`MAX_FRAME_LEN`].
pub fn write_frame(stream: &mut TcpStream, msg: &Message) -> Result<(), WireError> {
    let body = msg.encode();
    let len = u32::try_from(body.len())
        .ok()
        .filter(|&l| l <= MAX_FRAME_LEN)
        .ok_or(WireError::FrameTooLarge(body.len() as u32))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;
    Ok(())
}

/// Read one length-prefixed [`Message`] frame from `stream` (blocking).
///
/// # Errors
///
/// [`WireError::Io`] on read failure or a peer disconnect mid-frame,
/// [`WireError::FrameTooLarge`] if the prefix exceeds [`MAX_FRAME_LEN`],
/// [`WireError::Malformed`] if the body fails to decode.
pub fn read_frame(stream: &mut TcpStream) -> Result<Message, WireError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(WireError::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    stream.read_exact(&mut body)?;
    Message::decode(&body)
}

/// The outcome of [`read_frame_or_idle`].
pub enum FrameRead {
    /// A complete frame arrived.
    Frame(Message),
    /// No frame began within the idle window — the caller may re-check its stop
    /// flag and call again. **Frame-boundary safe**: this is returned only when
    /// *zero* bytes of a frame had been consumed, so no desync is possible.
    Idle,
}

/// Read one frame, but return [`FrameRead::Idle`] if the read times out **at a
/// frame boundary** (before any byte of the length prefix arrives).
///
/// This is the teardown-liveness read used by the follower serve loop: it lets
/// the loop wake periodically to observe a stop request or a vanished peer
/// without ever leaving a half-consumed frame on the wire. Once the *first*
/// length byte has been read, the rest of the frame is completed with a blocking
/// read (the timeout is cleared), so a slow-but-live sender never desyncs the
/// stream. The caller must have set a read timeout on `stream`.
///
/// # Errors
///
/// As [`read_frame`], plus a genuine mid-frame timeout (after the first byte)
/// surfaces as [`WireError::Io`] — that is a real stalled peer, not idleness.
pub fn read_frame_or_idle(stream: &mut TcpStream) -> Result<FrameRead, WireError> {
    let mut len_buf = [0u8; 4];
    // Try to read the first byte; a clean timeout here means "no frame started".
    match stream.read(&mut len_buf[..1]) {
        Ok(0) => return Err(WireError::Io(io::Error::from(io::ErrorKind::UnexpectedEof))),
        Ok(_) => {}
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(FrameRead::Idle);
        }
        Err(e) => return Err(WireError::Io(e)),
    }
    // A frame has begun. Complete it under a blocking read so a slow sender does
    // not cause a mid-frame desync. Restore the prior timeout afterwards.
    let prior = stream.read_timeout().ok().flatten();
    stream.set_read_timeout(None)?;
    let result = (|| {
        stream.read_exact(&mut len_buf[1..])?;
        let len = u32::from_be_bytes(len_buf);
        if len > MAX_FRAME_LEN {
            return Err(WireError::FrameTooLarge(len));
        }
        let mut body = vec![0u8; len as usize];
        stream.read_exact(&mut body)?;
        Message::decode(&body)
    })();
    stream.set_read_timeout(prior)?;
    result.map(FrameRead::Frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::LogEntry;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    #[test]
    fn messages_round_trip() {
        let msgs = [
            Message::Append {
                entry: LogEntry::new(3, 9, vec![1, 2, 3]),
                leader_commit: 8,
            },
            Message::Ack {
                match_index: 9,
                term: 3,
            },
            Message::StatusRequest,
            Message::Status {
                last_index: 9,
                term: 3,
            },
            Message::Commit { leader_commit: 7 },
        ];
        for m in msgs {
            assert_eq!(Message::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn frames_traverse_a_real_loopback_socket() {
        // Bind on an ephemeral 127.0.0.1 port — a genuine OS socket, not a fake.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let addr = listener.local_addr().unwrap();
        let sent = Message::Append {
            entry: LogEntry::new(1, 0, vec![42, 7]),
            leader_commit: 0,
        };
        let sent2 = sent.clone();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            read_frame(&mut conn).unwrap()
        });
        let mut client = TcpStream::connect(addr).expect("connect");
        write_frame(&mut client, &sent2).unwrap();
        let got = server.join().unwrap();
        assert_eq!(got, sent);
    }
}
