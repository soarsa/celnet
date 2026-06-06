//! Length-prefixed framing and the Raft RPC message types, over real blocking
//! `std::net::TcpStream` sockets.
//!
//! The transport is the simplest honest real-socket transport: a blocking
//! [`std::net::TcpStream`] carrying **4-byte big-endian length-prefixed** frames,
//! each holding one [`Message`]. There is no shared-memory shortcut — a node
//! process talks to a peer process strictly through bytes on a loopback TCP
//! connection, exactly as a cross-host deployment would (the only difference being
//! the route the kernel takes; see the crate-root honest boundary on what loopback
//! does and does not prove).
//!
//! # The two Raft RPCs
//!
//! * [`Message::AppendEntries`] — a leader replicates a (possibly empty, i.e. a
//!   heartbeat) run of [`LogEntry`]s, carrying `prev_log_index`/`prev_log_term`
//!   for the §5.3 log-matching check and `leader_commit` for commit propagation.
//!   The reply is [`Message::AppendReply`] (`success` + the follower's resulting
//!   `match_index`, plus its `term` so a stale leader steps down).
//! * [`Message::RequestVote`] — a candidate solicits a vote, carrying its
//!   `last_log_index`/`last_log_term` so a voter can apply the §5.4.1 up-to-date
//!   rule. The reply is [`Message::VoteReply`] (`granted` + the voter's `term`).
//!
//! [`Message::StatusRequest`]/[`Message::Status`] remain for an out-of-band
//! liveness/high-water probe.

use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::entry::LogEntry;

/// Maximum accepted frame length (64 MiB), guarding the read-side allocation
/// against a corrupt/hostile length prefix — mirrors the journal's payload bound.
pub const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;

/// Sentinel `last_index` / `prev_log_index` meaning "no entry" (empty log, or no
/// preceding entry). Shared with [`crate::log::EMPTY_PREV`].
pub const EMPTY_LOG: u64 = u64::MAX;

/// A protocol message exchanged between cluster nodes.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Leader → follower: replicate `entries` (empty ⇒ heartbeat). The follower
    /// applies the §5.3 log-matching check against `prev_log_index`/`prev_log_term`
    /// and, on success, reconciles its log (truncating any conflicting tail) and
    /// advances its commit watermark toward `leader_commit`.
    AppendEntries {
        /// The leader's current term.
        term: u64,
        /// Index of the entry immediately preceding `entries`, or [`EMPTY_LOG`] if
        /// `entries` begins at index 0 (no preceding entry).
        prev_log_index: u64,
        /// Term of the entry at `prev_log_index` (ignored when `prev_log_index`
        /// is [`EMPTY_LOG`]).
        prev_log_term: u64,
        /// The entries to replicate, in contiguous ascending index order (may be
        /// empty for a pure heartbeat / commit-advance).
        entries: Vec<LogEntry>,
        /// The leader's commit index ([`EMPTY_LOG`] if nothing committed).
        leader_commit: u64,
    },
    /// Follower → leader: the result of an [`Message::AppendEntries`].
    AppendReply {
        /// The follower's current term (a leader observing a higher term steps down).
        term: u64,
        /// Whether the log-matching check passed and the entries were appended.
        success: bool,
        /// On success, the follower's resulting durable high-water index; on
        /// failure, the follower's current high-water (so the leader can back up
        /// `next_index` toward a matching point). [`EMPTY_LOG`] if empty.
        match_index: u64,
    },
    /// Candidate → peer: solicit a vote in `term`.
    RequestVote {
        /// The candidate's term. For a **pre-vote** (`pre_vote == true`) this is the
        /// term the candidate *would* run in (its current term + 1) — a hypothetical,
        /// NOT yet adopted: a pre-vote never causes either side to change its
        /// persistent term, so a flaky node cannot disrupt a healthy leader by
        /// bumping terms (Ongaro thesis §9.6, the Pre-Vote optimisation).
        term: u64,
        /// `true` for a pre-vote straw poll (no term change on either side); `false`
        /// for a real vote that durably records `voted_for`.
        pre_vote: bool,
        /// A stable identifier of the candidate (its listen port) so a voter's
        /// `voted_for` records *who* it voted for and re-grants idempotently.
        candidate_id: u64,
        /// Index of the candidate's last log entry ([`EMPTY_LOG`] if empty).
        last_log_index: u64,
        /// Term of the candidate's last log entry (0 if empty).
        last_log_term: u64,
    },
    /// Peer → candidate: the vote decision.
    VoteReply {
        /// The voter's current term (a candidate observing a higher term steps down).
        /// For a pre-vote reply this is the voter's real current term, used only to
        /// let a hopelessly-behind pre-candidate learn it should not proceed.
        term: u64,
        /// `true` if this replies to a pre-vote straw poll (the granter has NOT
        /// recorded any vote); `false` for a real vote grant.
        pre_vote: bool,
        /// Whether the (pre-)vote was granted.
        granted: bool,
    },
    /// Either direction: a request for the peer's durable high-water index + term.
    StatusRequest,
    /// Reply to [`Message::StatusRequest`].
    Status {
        /// The responder's durable high-water (last appended) index, or
        /// [`EMPTY_LOG`] meaning "empty log".
        last_index: u64,
        /// The responder's current term.
        term: u64,
    },
}

impl Message {
    /// Encode to deterministic bytes (1-byte tag + fields).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Message::AppendEntries {
                term,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            } => {
                buf.push(0u8);
                buf.extend_from_slice(&term.to_le_bytes());
                buf.extend_from_slice(&prev_log_index.to_le_bytes());
                buf.extend_from_slice(&prev_log_term.to_le_bytes());
                buf.extend_from_slice(&leader_commit.to_le_bytes());
                buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
                for e in entries {
                    let eb = e.encode();
                    buf.extend_from_slice(&(eb.len() as u32).to_le_bytes());
                    buf.extend_from_slice(&eb);
                }
            }
            Message::AppendReply {
                term,
                success,
                match_index,
            } => {
                buf.push(1u8);
                buf.extend_from_slice(&term.to_le_bytes());
                buf.push(u8::from(*success));
                buf.extend_from_slice(&match_index.to_le_bytes());
            }
            Message::RequestVote {
                term,
                pre_vote,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                buf.push(2u8);
                buf.extend_from_slice(&term.to_le_bytes());
                buf.push(u8::from(*pre_vote));
                buf.extend_from_slice(&candidate_id.to_le_bytes());
                buf.extend_from_slice(&last_log_index.to_le_bytes());
                buf.extend_from_slice(&last_log_term.to_le_bytes());
            }
            Message::VoteReply {
                term,
                pre_vote,
                granted,
            } => {
                buf.push(3u8);
                buf.extend_from_slice(&term.to_le_bytes());
                buf.push(u8::from(*pre_vote));
                buf.push(u8::from(*granted));
            }
            Message::StatusRequest => buf.push(4u8),
            Message::Status { last_index, term } => {
                buf.push(5u8);
                buf.extend_from_slice(&last_index.to_le_bytes());
                buf.extend_from_slice(&term.to_le_bytes());
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
        let mut c = Cursor::new(rest);
        match tag {
            0 => {
                let term = c.u64()?;
                let prev_log_index = c.u64()?;
                let prev_log_term = c.u64()?;
                let leader_commit = c.u64()?;
                let n = c.u32()? as usize;
                let mut entries = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    let elen = c.u32()? as usize;
                    let eb = c.take(elen)?;
                    entries.push(LogEntry::decode(eb).map_err(|_| WireError::Malformed)?);
                }
                Ok(Message::AppendEntries {
                    term,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                })
            }
            1 => {
                let term = c.u64()?;
                let success = c.bool()?;
                let match_index = c.u64()?;
                Ok(Message::AppendReply {
                    term,
                    success,
                    match_index,
                })
            }
            2 => Ok(Message::RequestVote {
                term: c.u64()?,
                pre_vote: c.bool()?,
                candidate_id: c.u64()?,
                last_log_index: c.u64()?,
                last_log_term: c.u64()?,
            }),
            3 => Ok(Message::VoteReply {
                term: c.u64()?,
                pre_vote: c.bool()?,
                granted: c.bool()?,
            }),
            4 => Ok(Message::StatusRequest),
            5 => Ok(Message::Status {
                last_index: c.u64()?,
                term: c.u64()?,
            }),
            _ => Err(WireError::Malformed),
        }
    }
}

/// A tiny bounds-checked little-endian field reader for the wire decoder.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Malformed)?;
        let slice = self.buf.get(self.pos..end).ok_or(WireError::Malformed)?;
        self.pos = end;
        Ok(slice)
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    fn bool(&mut self) -> Result<bool, WireError> {
        Ok(self.take(1)?[0] != 0)
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
/// This is the teardown-liveness read used by the serve loop: it lets the loop
/// wake periodically to observe a stop request or a vanished peer without ever
/// leaving a half-consumed frame on the wire. Once the *first* length byte has
/// been read, the rest of the frame is completed with a blocking read (the timeout
/// is cleared), so a slow-but-live sender never desyncs the stream. The caller
/// must have set a read timeout on `stream`.
///
/// # Errors
///
/// As [`read_frame`], plus a genuine mid-frame timeout (after the first byte)
/// surfaces as [`WireError::Io`] — that is a real stalled peer, not idleness.
pub fn read_frame_or_idle(stream: &mut TcpStream) -> Result<FrameRead, WireError> {
    let mut len_buf = [0u8; 4];
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

    fn sample_messages() -> Vec<Message> {
        vec![
            Message::AppendEntries {
                term: 7,
                prev_log_index: 3,
                prev_log_term: 5,
                entries: vec![
                    LogEntry::new(7, 4, vec![1, 2, 3]),
                    LogEntry::new(7, 5, vec![]),
                ],
                leader_commit: 4,
            },
            // heartbeat (empty entries, empty-prev sentinel)
            Message::AppendEntries {
                term: 9,
                prev_log_index: EMPTY_LOG,
                prev_log_term: 0,
                entries: vec![],
                leader_commit: EMPTY_LOG,
            },
            Message::AppendReply {
                term: 7,
                success: true,
                match_index: 5,
            },
            Message::AppendReply {
                term: 8,
                success: false,
                match_index: EMPTY_LOG,
            },
            Message::RequestVote {
                term: 12,
                pre_vote: false,
                candidate_id: 40_001,
                last_log_index: 5,
                last_log_term: 7,
            },
            Message::RequestVote {
                term: 12,
                pre_vote: true,
                candidate_id: 40_002,
                last_log_index: EMPTY_LOG,
                last_log_term: 0,
            },
            Message::VoteReply {
                term: 12,
                pre_vote: false,
                granted: true,
            },
            Message::VoteReply {
                term: 13,
                pre_vote: true,
                granted: false,
            },
            Message::StatusRequest,
            Message::Status {
                last_index: 9,
                term: 3,
            },
        ]
    }

    #[test]
    fn all_messages_round_trip() {
        for m in sample_messages() {
            assert_eq!(Message::decode(&m.encode()).unwrap(), m, "round-trip {m:?}");
        }
    }

    #[test]
    fn truncated_append_entries_is_rejected() {
        let m = Message::AppendEntries {
            term: 1,
            prev_log_index: 0,
            prev_log_term: 1,
            entries: vec![LogEntry::new(1, 1, vec![9])],
            leader_commit: 0,
        };
        let bytes = m.encode();
        // Lopping off the tail must surface as Malformed, never a panic or a
        // silently-short entry list.
        for cut in 1..bytes.len() {
            let _ = Message::decode(&bytes[..cut]); // must not panic
        }
        assert!(matches!(
            Message::decode(&bytes[..bytes.len() - 1]),
            Err(WireError::Malformed)
        ));
    }

    #[test]
    fn frames_traverse_a_real_loopback_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let addr = listener.local_addr().unwrap();
        let sent = Message::AppendEntries {
            term: 1,
            prev_log_index: EMPTY_LOG,
            prev_log_term: 0,
            entries: vec![LogEntry::new(1, 0, vec![42, 7])],
            leader_commit: EMPTY_LOG,
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
