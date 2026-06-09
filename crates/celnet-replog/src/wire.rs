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
//!
//! # The InstallSnapshot RPC (§7)
//!
//! [`Message::InstallSnapshot`] is the third leader→follower RPC: when a leader has
//! **compacted past** the entries a far-behind (or freshly-restarted) follower
//! needs — the follower's required next index has fallen below the leader's log
//! `base_index`, so the entries to bridge the gap no longer exist in the leader's
//! log — the leader cannot `AppendEntries` the missing prefix. Instead it ships the
//! durable [`crate::compaction::Snapshot`] bytes (the applied
//! [`crate::state::BookState`] captured at `(last_included_index,
//! last_included_term)`). The follower durably installs it, discards any
//! conflicting prefix, reseeds its applied state from the snapshot, and resumes
//! normal [`Message::AppendEntries`] from `last_included_index + 1`. The reply
//! reuses [`Message::AppendReply`] (carrying the follower's `term` so a stale
//! leader steps down, and on success `match_index == last_included_index` so the
//! leader advances `next_index`/`match_index` exactly as it would after a
//! successful AppendEntries — one reply shape, no extra variant).

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
    /// Leader → follower (§7): transfer the durable snapshot when the leader has
    /// compacted past the entries the follower needs. The follower durably installs
    /// the snapshot, reseeds its applied state and watermarks, discards any
    /// conflicting log prefix, and resumes AppendEntries from the index just after
    /// the boundary. The reply reuses [`Message::AppendReply`]: on success its
    /// `match_index` is `last_included_index`, and the follower's `term` is always
    /// reported (so a stale leader steps down).
    InstallSnapshot {
        /// The leader's current term (a follower observing a higher term steps the
        /// leader down via the reply; a stale leader's install is rejected).
        term: u64,
        /// A stable identifier of the leader (its listen port) — informational,
        /// mirroring `candidate_id` on [`Message::RequestVote`].
        leader_id: u64,
        /// The absolute log index of the last entry the snapshot subsumes
        /// (inclusive). The follower resumes replication from here + 1.
        last_included_index: u64,
        /// The term of the entry at `last_included_index` (retained so log-matching
        /// at the new snapshot boundary succeeds after the prefix is discarded).
        last_included_term: u64,
        /// The canonical [`crate::compaction::Snapshot`] bytes (its
        /// [`crate::compaction::Snapshot::encode`] form: a CRC-protected capture of
        /// the applied [`crate::state::BookState`] at the boundary). Bounded by
        /// [`MAX_FRAME_LEN`] like every frame.
        snapshot_bytes: Vec<u8>,
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
            Message::InstallSnapshot {
                term,
                leader_id,
                last_included_index,
                last_included_term,
                snapshot_bytes,
            } => {
                buf.push(6u8);
                buf.extend_from_slice(&term.to_le_bytes());
                buf.extend_from_slice(&leader_id.to_le_bytes());
                buf.extend_from_slice(&last_included_index.to_le_bytes());
                buf.extend_from_slice(&last_included_term.to_le_bytes());
                buf.extend_from_slice(&(snapshot_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(snapshot_bytes);
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
            6 => {
                let term = c.u64()?;
                let leader_id = c.u64()?;
                let last_included_index = c.u64()?;
                let last_included_term = c.u64()?;
                let len = c.u32()? as usize;
                let snapshot_bytes = c.take(len)?.to_vec();
                Ok(Message::InstallSnapshot {
                    term,
                    leader_id,
                    last_included_index,
                    last_included_term,
                    snapshot_bytes,
                })
            }
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
    use std::time::Duration;

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
            Message::InstallSnapshot {
                term: 14,
                leader_id: 40_007,
                last_included_index: 128,
                last_included_term: 13,
                snapshot_bytes: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
            },
            // An empty-state snapshot (e.g. boundary 0 of an empty book) still
            // round-trips with a zero-length payload.
            Message::InstallSnapshot {
                term: 1,
                leader_id: 40_008,
                last_included_index: 0,
                last_included_term: 1,
                snapshot_bytes: vec![],
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
    fn truncated_install_snapshot_is_rejected() {
        let m = Message::InstallSnapshot {
            term: 5,
            leader_id: 40_009,
            last_included_index: 7,
            last_included_term: 4,
            snapshot_bytes: vec![1, 2, 3, 4, 5],
        };
        let bytes = m.encode();
        for cut in 1..bytes.len() {
            let _ = Message::decode(&bytes[..cut]); // must not panic
        }
        // Lopping the last payload byte off makes the declared length overrun the
        // buffer ⇒ Malformed, never a silently-short snapshot.
        assert!(matches!(
            Message::decode(&bytes[..bytes.len() - 1]),
            Err(WireError::Malformed)
        ));
    }

    #[test]
    fn install_snapshot_traverses_a_real_loopback_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let addr = listener.local_addr().unwrap();
        let sent = Message::InstallSnapshot {
            term: 9,
            leader_id: 40_010,
            last_included_index: 256,
            last_included_term: 8,
            snapshot_bytes: vec![7u8; 4096], // a non-trivial multi-KiB payload
        };
        let sent2 = sent.clone();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // A bounded read so a regression that drops the write fails FAST (a
            // mutation that makes `write_frame` a no-op would otherwise hang here).
            conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            read_frame(&mut conn).expect("frame round-trips")
        });
        let mut client = TcpStream::connect(addr).expect("connect");
        write_frame(&mut client, &sent2).unwrap();
        let got = server.join().unwrap();
        assert_eq!(got, sent);
    }

    #[test]
    fn wire_error_display_is_exact() {
        // Pins the Display strings (kills `WireError::fmt -> Ok(Default::default())`,
        // which would emit empty strings).
        assert_eq!(
            WireError::Io(io::Error::from(io::ErrorKind::UnexpectedEof)).to_string(),
            format!("wire io: {}", io::Error::from(io::ErrorKind::UnexpectedEof))
        );
        assert_eq!(
            WireError::FrameTooLarge(123).to_string(),
            "wire frame too large: 123"
        );
        assert_eq!(WireError::Malformed.to_string(), "wire frame malformed");
    }

    #[test]
    fn max_frame_len_is_exactly_64_mib() {
        // 64 * 1024 * 1024. Kills `* with +` on line 50 (`64 + 1024 + 1024` =
        // 2112 != 67108864), which would also wrongly reject legitimately-sized
        // frames at runtime.
        assert_eq!(MAX_FRAME_LEN, 67_108_864);
        assert_eq!(MAX_FRAME_LEN, 64 * 1024 * 1024);
    }

    #[test]
    fn read_frame_rejects_oversize_prefix_at_and_above_the_bound() {
        // A length prefix of MAX_FRAME_LEN + 1 must be rejected as FrameTooLarge,
        // while exactly MAX_FRAME_LEN is accepted-as-a-length (it then tries to read
        // the body). This pins the `len > MAX_FRAME_LEN` boundary in read_frame
        // (line 412): `> with ==` would reject a too-large length only when it is
        // EXACTLY equal (wrong), and `> with >=` would WRONGLY reject the
        // exactly-MAX legal length.
        //
        // Every server socket here carries a SHORT read-timeout so that, whenever the
        // length check fails to reject and `read_frame` proceeds to read a body that
        // never arrives, the read fails FAST as an Io-timeout (a deterministic, fast
        // CAUGHT) rather than blocking until the mutation-test timeout.
        //
        // (a) Over-bound prefix MAX+1: the original `len > MAX` rejects → FrameTooLarge
        //     BEFORE reading any body (fast). The `> with ==` mutant (`MAX+1 == MAX`
        //     false) and `> with <` (`MAX+1 < MAX` false) do NOT reject → proceed to
        //     read the 64 MiB+1 body that never comes → Io-timeout. So FrameTooLarge
        //     vs Io distinguishes the original from both mutants.
        let got = drive_read_frame((MAX_FRAME_LEN + 1).to_be_bytes().to_vec());
        assert!(
            matches!(got, Err(WireError::FrameTooLarge(n)) if n == MAX_FRAME_LEN + 1),
            "a prefix above the bound is FrameTooLarge (kills `> with ==`/`<`)"
        );
        // (b) Exactly-MAX prefix (a LEGAL length): the original `MAX > MAX` is false →
        //     proceeds to read the body (none sent) → Io-timeout. The `> with >=`
        //     mutant (`MAX >= MAX` true) WRONGLY rejects it as FrameTooLarge. So Io
        //     (not FrameTooLarge) proves the bound is strict `>` (kills `> with >=`).
        let got = drive_read_frame(MAX_FRAME_LEN.to_be_bytes().to_vec());
        assert!(
            matches!(got, Err(WireError::Io(_))),
            "exactly-MAX is a legal length: Io-timeout on the body, not FrameTooLarge (kills `> with >=`)"
        );
    }

    /// Send `prefix_bytes` to a fresh loopback `read_frame` server whose socket has a
    /// short read-timeout, then return its result. The timeout guarantees the call
    /// returns fast even when a (mutated) `read_frame` proceeds to read a body that is
    /// never sent — turning a would-be hang into a deterministic `Io` error.
    fn drive_read_frame(prefix_bytes: Vec<u8>) -> Result<Message, WireError> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_millis(250)))
                .unwrap();
            read_frame(&mut conn)
        });
        let mut client = TcpStream::connect(addr).unwrap();
        client.write_all(&prefix_bytes).unwrap();
        client.flush().unwrap();
        let got = server.join().unwrap();
        drop(client);
        got
    }

    #[test]
    fn read_frame_or_idle_returns_idle_only_at_a_frame_boundary_timeout() {
        // A read timeout with NO bytes sent → Idle (the WouldBlock/TimedOut guard at
        // line 450 accepts it). Kills `matches! with false`, which would turn this
        // boundary timeout into an Io error.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_millis(150)))
                .unwrap();
            read_frame_or_idle(&mut conn)
        });
        let _client = TcpStream::connect(addr).unwrap(); // connect, send nothing
        assert!(
            matches!(server.join().unwrap(), Ok(FrameRead::Idle)),
            "a boundary timeout with no bytes is Idle (kills `matches! with false`)"
        );
    }

    #[test]
    fn read_frame_or_idle_completes_a_real_frame() {
        // A full frame sent after the connect must be returned as Frame(..), proving
        // the guard does NOT swallow live data as idle.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sent = Message::StatusRequest;
        let sent2 = sent.clone();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            read_frame_or_idle(&mut conn)
        });
        let mut client = TcpStream::connect(addr).unwrap();
        write_frame(&mut client, &sent2).unwrap();
        let got = server.join().unwrap();
        assert!(
            matches!(got, Ok(FrameRead::Frame(m)) if m == sent),
            "a complete frame is returned, not Idle"
        );
    }

    /// Send `prefix` (a 4-byte length) to a fresh `read_frame_or_idle` server, then
    /// CLOSE the connection so the post-prefix body read hits EOF immediately. The
    /// close makes the call return FAST (no block) even when the mutated length check
    /// fails to reject and proceeds to read a body — `read_exact` errors on the EOF.
    fn drive_idle_prefix_then_close(prefix: [u8; 4]) -> Result<FrameRead, WireError> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            read_frame_or_idle(&mut conn)
        });
        let mut client = TcpStream::connect(addr).unwrap();
        client.write_all(&prefix).unwrap();
        client.flush().unwrap();
        drop(client); // clean EOF after the 4 prefix bytes
        server.join().unwrap()
    }

    #[test]
    fn read_frame_or_idle_length_boundary_is_strict() {
        // The idle-path length guard `if len > MAX_FRAME_LEN` (line 464). After the
        // first byte the read-timeout is cleared, so a body that never arrives would
        // block — we instead CLOSE the connection right after the 4-byte prefix, so
        // the post-prefix body read fails FAST with EOF (`Io`) and the call returns
        // deterministically. The two distinguishing prefixes:
        //  * MAX+1 (over-bound): original `len > MAX` true → FrameTooLarge (no body
        //    read). `> with ==` (`MAX+1 == MAX` false) → reads body → EOF `Io`. So
        //    FrameTooLarge vs Io kills `> with ==`.
        let got = drive_idle_prefix_then_close((MAX_FRAME_LEN + 1).to_be_bytes());
        assert!(
            matches!(got, Err(WireError::FrameTooLarge(n)) if n == MAX_FRAME_LEN + 1),
            "an over-bound idle prefix is FrameTooLarge (kills 464 `> with ==`)"
        );
        //  * exactly-MAX (legal): original `MAX > MAX` false → reads the body → EOF
        //    `Io`. `> with >=` (`MAX >= MAX` true) → WRONGLY rejects as FrameTooLarge.
        //    So Io (not FrameTooLarge) kills `> with >=`.
        let got = drive_idle_prefix_then_close(MAX_FRAME_LEN.to_be_bytes());
        assert!(
            matches!(got, Err(WireError::Io(_))),
            "exactly-MAX is a legal idle length: EOF Io on the body, not FrameTooLarge (kills 464 `> with >=`)"
        );
    }

    #[test]
    fn write_frame_actually_writes_the_framed_bytes() {
        // A `write_frame -> Ok(())` mutant writes NOTHING. To catch it WITHOUT
        // hanging (the loopback reader would block forever on a missing prefix), the
        // receiver uses a short read-timeout: a correct write_frame delivers a full,
        // decodable frame; the no-op mutant delivers zero bytes → the reader times
        // out (Err), which this asserts against. So the test is fast either way and
        // distinguishes the two.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sent = Message::Status {
            last_index: 42,
            term: 9,
        };
        let sent2 = sent.clone();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_millis(500)))
                .unwrap();
            read_frame(&mut conn)
        });
        let mut client = TcpStream::connect(addr).unwrap();
        write_frame(&mut client, &sent2).expect("write_frame must succeed");
        let got = server.join().unwrap();
        assert_eq!(
            got.expect("a real write_frame delivers a decodable frame"),
            sent,
            "write_frame must put the framed bytes on the wire (kills the Ok(()) no-op)"
        );
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
            conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            read_frame(&mut conn).expect("frame round-trips")
        });
        let mut client = TcpStream::connect(addr).expect("connect");
        write_frame(&mut client, &sent2).unwrap();
        let got = server.join().unwrap();
        assert_eq!(got, sent);
    }
}
