//! FIXT/4.4 session layer: the logon/logout/heartbeat/test-request/resend/
//! sequence-reset FSM plus a message store (in-memory and file-backed).
//!
//! The session layer is transport-agnostic: it consumes already-framed inbound
//! [`FrameCursor`]s and emits owned outbound frames as `Vec<u8>`, driven by a
//! monotonic clock the caller advances. The async I/O lives in
//! [`crate::acceptor`] / [`crate::initiator`], which pump bytes between a socket
//! and this FSM. This separation lets the FSM be tested deterministically and
//! lets the same state machine serve both session roles.
//!
//! Sequence-number management implements the FIX recovery protocol:
//! out-of-sequence inbound triggers a `ResendRequest(2)`; an inbound resend
//! request is answered with a gap-fill `SequenceReset(4)` covering the
//! administrative messages we will not replay. The store persists every
//! outbound application message so a resend can re-transmit it.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

use crate::dictionary::MsgType;
use crate::framing::{FrameCursor, FrameError, parse_uint};
use crate::messages::Header;
use crate::{dictionary, messages};

/// The session role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Quote venue — waits for an inbound `Logon` then mirrors it.
    Acceptor,
    /// Price taker / hedge — initiates the `Logon`.
    Initiator,
}

/// The session FSM state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// No logon yet exchanged.
    Disconnected,
    /// `Logon` sent (initiator) or received (acceptor), awaiting confirmation.
    LogonPending,
    /// Fully established; heartbeats flowing.
    Active,
    /// `Logout` sent or received; awaiting confirmation / teardown.
    LogoutPending,
}

/// Stores outbound application messages so they can be re-sent on a
/// `ResendRequest`. The store is keyed by `MsgSeqNum`.
pub trait MessageStore {
    /// Persist an outbound message under its sequence number.
    fn put(&mut self, seq: u64, bytes: &[u8]);
    /// Retrieve a stored message by sequence number.
    fn get(&self, seq: u64) -> Option<Vec<u8>>;
}

/// An in-memory message store backed by a `BTreeMap` (ordered for range
/// replay).
#[derive(Debug, Default)]
pub struct InMemoryStore {
    msgs: BTreeMap<u64, Vec<u8>>,
}

impl InMemoryStore {
    /// A fresh empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl MessageStore for InMemoryStore {
    fn put(&mut self, seq: u64, bytes: &[u8]) {
        self.msgs.insert(seq, bytes.to_vec());
    }
    fn get(&self, seq: u64) -> Option<Vec<u8>> {
        self.msgs.get(&seq).cloned()
    }
}

/// A file-backed message store: each outbound message is appended to a log file
/// as `seq\tlen\t<bytes>`, and an in-memory index mirrors the BTreeMap for fast
/// lookup. No heavy dependency — a plain append-only file (the plan forbids
/// Mongo/other heavy stores).
#[derive(Debug)]
pub struct FileStore {
    index: BTreeMap<u64, Vec<u8>>,
    file: std::fs::File,
}

impl FileStore {
    /// Open (creating/truncating) a store at `path`.
    ///
    /// # Errors
    /// Propagates any filesystem error opening the log file.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            index: BTreeMap::new(),
            file,
        })
    }
}

impl MessageStore for FileStore {
    fn put(&mut self, seq: u64, bytes: &[u8]) {
        self.index.insert(seq, bytes.to_vec());
        // Best-effort durable append; framing the record length avoids SOH
        // ambiguity within the message bytes.
        let header = format!("{seq}\t{}\t", bytes.len());
        let _ = self.file.write_all(header.as_bytes());
        let _ = self.file.write_all(bytes);
        let _ = self.file.write_all(b"\n");
        let _ = self.file.flush();
    }
    fn get(&self, seq: u64) -> Option<Vec<u8>> {
        self.index.get(&seq).cloned()
    }
}

/// A session-layer error surfaced to the transport layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// The inbound frame failed framing/checksum validation.
    Framing(FrameError),
    /// The inbound frame failed dialect validation.
    Dictionary(dictionary::DictError),
    /// A message arrived in a state that does not permit it.
    UnexpectedState {
        /// The state the session was in.
        state: SessionState,
    },
    /// `CompID` mismatch on an inbound message.
    CompIdMismatch,
}

/// What the session decided to do with an inbound message: zero or more frames
/// to send back, plus whether the inbound was an application message the caller
/// should process.
#[derive(Debug, Default)]
pub struct SessionAction {
    /// Frames the session wants transmitted (heartbeat replies, resend
    /// requests, gap-fills, logon/logout mirrors).
    pub outbound: Vec<Vec<u8>>,
    /// If set, the inbound application message (its `MsgType`) for the caller's
    /// application layer to handle.
    pub deliver: Option<MsgType>,
}

/// Identity + heartbeat configuration for a session.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Our `SenderCompID`.
    pub sender: Vec<u8>,
    /// The counterparty `TargetCompID`.
    pub target: Vec<u8>,
    /// Heartbeat interval (seconds).
    pub heart_bt_int: u32,
    /// The session role.
    pub role: Role,
}

/// The session FSM, generic over the [`MessageStore`].
///
/// Outbound sequence numbers are stamped by [`Session::next_outbound`]; inbound
/// numbers are validated against [`Session::expected_inbound`]. The caller
/// supplies `SendingTime` bytes (its clock) per outbound frame, keeping the FSM
/// clock-free and deterministic for testing.
#[derive(Debug)]
pub struct Session<S: MessageStore> {
    cfg: SessionConfig,
    state: SessionState,
    next_out: u64,
    expected_in: u64,
    store: S,
    enc: crate::framing::FrameEncoder,
}

impl<S: MessageStore> Session<S> {
    /// Construct a session with the given config and store, starting at
    /// sequence 1 on both sides.
    pub fn new(cfg: SessionConfig, store: S) -> Self {
        Self {
            cfg,
            state: SessionState::Disconnected,
            next_out: 1,
            expected_in: 1,
            store,
            enc: crate::framing::FrameEncoder::new(),
        }
    }

    /// The current FSM state.
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// The next outbound sequence number that will be stamped.
    #[must_use]
    pub fn next_outbound(&self) -> u64 {
        self.next_out
    }

    /// The next inbound sequence number expected.
    #[must_use]
    pub fn expected_inbound(&self) -> u64 {
        self.expected_in
    }

    /// Stamp, store (if application) and return an outbound application/admin
    /// frame builder result, advancing the outbound sequence.
    fn emit(
        &mut self,
        build: impl FnOnce(&Header<'_>, &mut crate::framing::FrameEncoder) -> Vec<u8>,
        sending_time: &[u8],
        is_app: bool,
    ) -> Vec<u8> {
        let seq = self.next_out;
        let hdr = Header {
            sender: &self.cfg.sender,
            target: &self.cfg.target,
            seq_num: seq,
            sending_time,
        };
        let frame = build(&hdr, &mut self.enc);
        if is_app {
            self.store.put(seq, &frame);
        }
        self.next_out += 1;
        frame
    }

    /// Begin the session as an initiator by emitting a `Logon(A)`.
    pub fn start_logon(&mut self, sending_time: &[u8], reset_seq: bool) -> Vec<u8> {
        debug_assert_eq!(self.cfg.role, Role::Initiator);
        let hb = self.cfg.heart_bt_int;
        let frame = self.emit(
            |h, e| messages::build_logon(h, hb, reset_seq, e),
            sending_time,
            false,
        );
        self.state = SessionState::LogonPending;
        frame
    }

    /// Emit a `TestRequest(1)` (used to probe a silent peer).
    pub fn send_test_request(&mut self, sending_time: &[u8], id: &[u8]) -> Vec<u8> {
        self.emit(
            |h, e| messages::build_test_request(h, id, e),
            sending_time,
            false,
        )
    }

    /// Emit a scheduled `Heartbeat(0)`.
    pub fn send_heartbeat(&mut self, sending_time: &[u8]) -> Vec<u8> {
        self.emit(
            |h, e| messages::build_heartbeat(h, None, e),
            sending_time,
            false,
        )
    }

    /// Emit an application message, storing it for possible resend. The
    /// `build` closure receives the stamped header and the session's encoder.
    pub fn send_app(
        &mut self,
        sending_time: &[u8],
        build: impl FnOnce(&Header<'_>, &mut crate::framing::FrameEncoder) -> Vec<u8>,
    ) -> Vec<u8> {
        self.emit(build, sending_time, true)
    }

    /// Process one inbound, already-validated frame and return the resulting
    /// [`SessionAction`].
    ///
    /// # Errors
    /// Returns a [`SessionError`] when the frame fails dialect validation, the
    /// `CompID`s mismatch, or the message is illegal in the current state.
    pub fn on_inbound(
        &mut self,
        raw: &[u8],
        sending_time: &[u8],
    ) -> Result<SessionAction, SessionError> {
        let frame = FrameCursor::parse(raw).map_err(SessionError::Framing)?;
        let mt = dictionary::validate(&frame).map_err(SessionError::Dictionary)?;

        // CompID check: SenderCompID(49) and TargetCompID(56) are mandatory
        // session-level fields. A frame missing either, or carrying the wrong
        // identities, is rejected — inbound SenderCompID must equal our
        // configured target and inbound TargetCompID our sender.
        let (in_sender, in_target) = match (frame.get(49), frame.get(56)) {
            (Some(s), Some(t)) => (s, t),
            _ => return Err(SessionError::CompIdMismatch),
        };
        if in_sender != self.cfg.target.as_slice() || in_target != self.cfg.sender.as_slice() {
            return Err(SessionError::CompIdMismatch);
        }

        let mut action = SessionAction::default();
        let in_seq = frame.get(34).and_then(parse_uint);

        // SequenceReset and Logon are processed regardless of seq for recovery.
        match mt {
            MsgType::Logon => {
                self.handle_logon(sending_time, &mut action);
                self.bump_inbound(in_seq);
                return Ok(action);
            }
            MsgType::SequenceReset => {
                // Gap-fill: adopt NewSeqNo(36) as the next expected inbound.
                if let Some(new_seq) = frame.get(36).and_then(parse_uint) {
                    self.expected_in = new_seq;
                }
                return Ok(action);
            }
            MsgType::Logout => {
                self.handle_logout(sending_time, &mut action);
                self.bump_inbound(in_seq);
                return Ok(action);
            }
            _ => {}
        }

        // Sequence-gap detection for ordinary messages.
        if let Some(seq) = in_seq {
            if seq > self.expected_in {
                // Gap: request a resend of the missing range and do not advance.
                let frame_out = self.emit(
                    {
                        let begin = self.expected_in;
                        move |h, e| messages::build_resend_request(h, begin, 0, e)
                    },
                    sending_time,
                    false,
                );
                action.outbound.push(frame_out);
                return Ok(action);
            } else if seq < self.expected_in {
                // Already processed (possible duplicate) — ignore.
                return Ok(action);
            }
        }

        match mt {
            MsgType::Heartbeat => {}
            MsgType::TestRequest => {
                // Reply with a heartbeat echoing the TestReqID.
                let id = frame.get(112).map(<[u8]>::to_vec);
                let frame_out = self.emit(
                    move |h, e| messages::build_heartbeat(h, id.as_deref(), e),
                    sending_time,
                    false,
                );
                action.outbound.push(frame_out);
            }
            MsgType::ResendRequest => {
                // Replay stored application messages in the requested range,
                // gap-filling administrative gaps.
                let begin = frame.get(7).and_then(parse_uint).unwrap_or(1);
                let end_raw = frame.get(16).and_then(parse_uint).unwrap_or(0);
                let end = if end_raw == 0 {
                    self.next_out.saturating_sub(1)
                } else {
                    end_raw
                };
                self.handle_resend(begin, end, sending_time, &mut action);
            }
            // Application messages are delivered to the caller.
            MsgType::QuoteRequest
            | MsgType::Quote
            | MsgType::MassQuote
            | MsgType::QuoteCancel
            | MsgType::NewOrderSingle
            | MsgType::NewOrderMultileg
            | MsgType::ExecutionReport => {
                action.deliver = Some(mt);
            }
            MsgType::Reject => {}
            MsgType::Logon | MsgType::SequenceReset | MsgType::Logout => unreachable!(),
        }

        self.bump_inbound(in_seq);
        Ok(action)
    }

    fn bump_inbound(&mut self, in_seq: Option<u64>) {
        if let Some(seq) = in_seq
            && seq == self.expected_in
        {
            self.expected_in += 1;
        }
    }

    fn handle_logon(&mut self, sending_time: &[u8], action: &mut SessionAction) {
        match self.cfg.role {
            Role::Acceptor => {
                // Mirror the logon back and become active.
                let hb = self.cfg.heart_bt_int;
                let frame = self.emit(
                    move |h, e| messages::build_logon(h, hb, false, e),
                    sending_time,
                    false,
                );
                action.outbound.push(frame);
                self.state = SessionState::Active;
            }
            Role::Initiator => {
                // Logon confirmation from the acceptor.
                self.state = SessionState::Active;
            }
        }
    }

    fn handle_logout(&mut self, sending_time: &[u8], action: &mut SessionAction) {
        if self.state != SessionState::LogoutPending {
            // Mirror the logout (graceful teardown).
            let frame = self.emit(
                |h, e| messages::build_logout(h, None, e),
                sending_time,
                false,
            );
            action.outbound.push(frame);
        }
        self.state = SessionState::Disconnected;
    }

    fn handle_resend(
        &mut self,
        begin: u64,
        end: u64,
        sending_time: &[u8],
        action: &mut SessionAction,
    ) {
        let mut seq = begin;
        let mut gap_start: Option<u64> = None;
        while seq <= end {
            if let Some(stored) = self.store.get(seq) {
                // Flush any pending admin gap as a gap-fill first.
                if let Some(gs) = gap_start.take() {
                    let new_seq = seq;
                    let frame = self.emit_with_seq(
                        gs,
                        move |h, e| messages::build_sequence_reset(h, new_seq, true, e),
                        sending_time,
                    );
                    action.outbound.push(frame);
                }
                action.outbound.push(stored);
            } else {
                // Administrative / missing message: cover with a gap-fill.
                if gap_start.is_none() {
                    gap_start = Some(seq);
                }
            }
            seq += 1;
        }
        if let Some(gs) = gap_start.take() {
            let new_seq = end + 1;
            let frame = self.emit_with_seq(
                gs,
                move |h, e| messages::build_sequence_reset(h, new_seq, true, e),
                sending_time,
            );
            action.outbound.push(frame);
        }
    }

    /// Emit a frame stamped with an explicit sequence number (used for replay /
    /// gap-fill, which must reuse the original sequence). Does not advance
    /// `next_out` and does not re-store.
    fn emit_with_seq(
        &mut self,
        seq: u64,
        build: impl FnOnce(&Header<'_>, &mut crate::framing::FrameEncoder) -> Vec<u8>,
        sending_time: &[u8],
    ) -> Vec<u8> {
        let hdr = Header {
            sender: &self.cfg.sender,
            target: &self.cfg.target,
            seq_num: seq,
            sending_time,
        };
        build(&hdr, &mut self.enc)
    }

    /// A borrow of the underlying store (for tests / introspection).
    #[must_use]
    pub fn store(&self) -> &S {
        &self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &[u8] = b"20260530-12:00:00.000";

    fn cfg(role: Role, sender: &str, target: &str) -> SessionConfig {
        SessionConfig {
            sender: sender.as_bytes().to_vec(),
            target: target.as_bytes().to_vec(),
            heart_bt_int: 30,
            role,
        }
    }

    #[test]
    fn acceptor_mirrors_logon() {
        let mut acc = Session::new(cfg(Role::Acceptor, "VENUE", "TAKER"), InMemoryStore::new());
        let mut init = Session::new(cfg(Role::Initiator, "TAKER", "VENUE"), InMemoryStore::new());
        let logon = init.start_logon(T, false);
        let action = acc.on_inbound(&logon, T).unwrap();
        assert_eq!(acc.state(), SessionState::Active);
        assert_eq!(action.outbound.len(), 1);
        // Initiator receives the mirror and becomes active.
        let confirm = &action.outbound[0];
        init.on_inbound(confirm, T).unwrap();
        assert_eq!(init.state(), SessionState::Active);
    }

    #[test]
    fn test_request_gets_heartbeat() {
        let mut acc = Session::new(cfg(Role::Acceptor, "VENUE", "TAKER"), InMemoryStore::new());
        let mut init = Session::new(cfg(Role::Initiator, "TAKER", "VENUE"), InMemoryStore::new());
        let logon = init.start_logon(T, false);
        acc.on_inbound(&logon, T).unwrap();
        let tr = init.send_test_request(T, b"PING");
        let action = acc.on_inbound(&tr, T).unwrap();
        assert_eq!(action.outbound.len(), 1);
        let frame = FrameCursor::parse(&action.outbound[0]).unwrap();
        assert_eq!(frame.msg_type(), b"0"); // heartbeat
        assert_eq!(frame.get(112), Some(&b"PING"[..]));
    }

    #[test]
    fn gap_triggers_resend_request() {
        let mut acc = Session::new(cfg(Role::Acceptor, "VENUE", "TAKER"), InMemoryStore::new());
        let mut init = Session::new(cfg(Role::Initiator, "TAKER", "VENUE"), InMemoryStore::new());
        let logon = init.start_logon(T, false);
        acc.on_inbound(&logon, T).unwrap();
        assert_eq!(acc.expected_inbound(), 2);
        // Initiator sends seq 2 (heartbeat) then skips to seq 4 (gap).
        let _hb2 = init.send_heartbeat(T); // seq 2
        let _hb3 = init.send_heartbeat(T); // seq 3 (will be "lost")
        let hb4 = init.send_heartbeat(T); // seq 4
        let action = acc.on_inbound(&hb4, T).unwrap();
        assert_eq!(action.outbound.len(), 1);
        let rr = FrameCursor::parse(&action.outbound[0]).unwrap();
        assert_eq!(rr.msg_type(), b"2"); // resend request
        assert_eq!(rr.get(7), Some(&b"2"[..])); // begin = expected (2)
    }

    #[test]
    fn mismatched_compids_rejected() {
        // Acceptor expects sender=VENUE/target=TAKER; a peer that logs on with
        // the wrong CompIDs must be rejected.
        let mut acc = Session::new(cfg(Role::Acceptor, "VENUE", "TAKER"), InMemoryStore::new());
        let mut rogue = Session::new(
            cfg(Role::Initiator, "IMPOSTOR", "VENUE"),
            InMemoryStore::new(),
        );
        let logon = rogue.start_logon(T, false);
        let err = acc.on_inbound(&logon, T).unwrap_err();
        assert_eq!(err, SessionError::CompIdMismatch);
        assert_eq!(acc.state(), SessionState::Disconnected);
    }

    #[test]
    fn missing_compids_rejected() {
        // A session-level frame lacking SenderCompID(49)/TargetCompID(56) is not
        // silently accepted — it is rejected as a CompID violation.
        let mut acc = Session::new(cfg(Role::Acceptor, "VENUE", "TAKER"), InMemoryStore::new());
        let mut enc = crate::framing::FrameEncoder::new();
        enc.clear();
        enc.push(35, MsgType::Heartbeat.as_bytes());
        // Intentionally omit 49 and 56.
        enc.push_int(34, 1);
        enc.push(52, T);
        let raw = enc.finish();
        let err = acc.on_inbound(&raw, T).unwrap_err();
        assert_eq!(err, SessionError::CompIdMismatch);
    }

    #[test]
    fn file_store_persists_and_replays() {
        let dir = std::env::temp_dir().join(format!("celnet-fix-store-{}", std::process::id()));
        let store = FileStore::create(&dir).unwrap();
        let mut acc = Session::new(cfg(Role::Acceptor, "VENUE", "TAKER"), store);
        // Store an app message at seq 1 and read it back.
        let raw = acc.send_app(T, |h, e| {
            let p = messages::QuoteParams {
                quote_req_id: b"R1",
                quote_id: b"Q1",
                symbol: b"EURUSD",
                bid_px: 1.0,
                offer_px: 1.1,
                size: 1.0,
                valid_until: b"20260530-12:00:05.000",
            };
            messages::build_quote(h, &p, e)
        });
        assert_eq!(acc.store().get(1).unwrap(), raw);
        let _ = std::fs::remove_file(&dir);
    }
}
