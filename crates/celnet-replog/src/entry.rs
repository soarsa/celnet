//! The replicated [`LogEntry`] and its deterministic byte codec.
//!
//! A `LogEntry` is the unit of replication: a `(term, index)`-stamped payload
//! that the leader appends to its [`celnet_journal::Journal`] and streams to
//! followers, who append it **in the same order** to their own journals. The
//! `(term, index)` pair plus a payload CRC make every entry self-describing on
//! the wire and on disk, so a follower can reject a misordered or corrupt entry
//! rather than silently diverge.
//!
//! # Why a second CRC over the journal's own CRC
//!
//! The journal already CRC-protects each *record* it stores (header + payload).
//! That protects the durable bytes at rest. This entry-level CRC protects the
//! `(term, index, payload)` tuple **on the wire** — it is computed by the leader
//! before transmission and re-checked by a follower on receipt, independently of
//! whether the bytes ever reach a disk. The two are complementary: wire integrity
//! (here) and at-rest integrity (journal).

use celnet_journal::crc32;

/// The Raft-style leadership term in which an entry was proposed.
///
/// Terms are monotonic across leadership changes. A thin leader-replicated log
/// (this crate's scope) uses the term to *reject stale leaders*: a follower that
/// has acknowledged term `t` will not accept an entry stamped with a term `< t`.
/// Full leader **election** that advances the term automatically is the next
/// increment (documented in the crate root), not built here.
pub type Term = u64;

/// The log index of an entry: a strictly monotonic `0, 1, 2, …` position that is
/// identical on every replica that holds the entry. It coincides with the
/// journal sequence number on each node, by construction (the leader appends in
/// index order and followers mirror that order).
pub type Index = u64;

/// One replicated log entry: a `(term, index)`-stamped opaque payload, integrity
/// protected by a CRC over the framed tuple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// The leadership term in which this entry was proposed.
    pub term: Term,
    /// The strictly-monotonic replicated log index.
    pub index: Index,
    /// The opaque, state-machine-defined payload bytes (here, an encoded
    /// [`crate::state::BookUpdate`]).
    pub payload: Vec<u8>,
}

impl LogEntry {
    /// Construct a new entry.
    #[must_use]
    pub fn new(term: Term, index: Index, payload: Vec<u8>) -> Self {
        Self {
            term,
            index,
            payload,
        }
    }

    /// Encode the entry to its deterministic durable/wire byte form.
    ///
    /// Layout (all little-endian):
    ///
    /// ```text
    /// ┌──────────┬──────────┬─────────────┬──────────────┬────────┐
    /// │ term u64 │ index u64│ plen u32    │ payload plen │ crc u32│
    /// └──────────┴──────────┴─────────────┴──────────────┴────────┘
    /// ```
    ///
    /// The CRC covers `term || index || plen || payload`. Encoding is a pure
    /// function of the fields, so two replicas holding the same entry encode to
    /// byte-identical journal records — the foundation of byte-identical log
    /// comparison across nodes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let plen = self.payload.len() as u32;
        let mut buf = Vec::with_capacity(8 + 8 + 4 + self.payload.len() + 4);
        buf.extend_from_slice(&self.term.to_le_bytes());
        buf.extend_from_slice(&self.index.to_le_bytes());
        buf.extend_from_slice(&plen.to_le_bytes());
        buf.extend_from_slice(&self.payload);
        let crc = crc32(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf
    }

    /// Decode an entry from its byte form, validating the CRC.
    ///
    /// # Errors
    ///
    /// Returns [`EntryError::Truncated`] if `bytes` is shorter than the framed
    /// minimum or the declared payload length overruns the buffer, and
    /// [`EntryError::Crc`] if the recomputed CRC does not match the stored one
    /// (a corrupt or tampered entry — rejected, never applied).
    pub fn decode(bytes: &[u8]) -> Result<Self, EntryError> {
        if bytes.len() < 8 + 8 + 4 + 4 {
            return Err(EntryError::Truncated);
        }
        let term = u64::from_le_bytes(bytes[0..8].try_into().expect("8 bytes"));
        let index = u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
        let plen = u32::from_le_bytes(bytes[16..20].try_into().expect("4 bytes")) as usize;
        let payload_end = 20usize.checked_add(plen).ok_or(EntryError::Truncated)?;
        let crc_end = payload_end.checked_add(4).ok_or(EntryError::Truncated)?;
        if bytes.len() < crc_end {
            return Err(EntryError::Truncated);
        }
        let stored_crc =
            u32::from_le_bytes(bytes[payload_end..crc_end].try_into().expect("4 bytes"));
        let actual_crc = crc32(&bytes[..payload_end]);
        if actual_crc != stored_crc {
            return Err(EntryError::Crc);
        }
        Ok(Self {
            term,
            index,
            payload: bytes[20..payload_end].to_vec(),
        })
    }
}

/// A failure decoding a [`LogEntry`] from bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryError {
    /// The buffer is shorter than the framed entry it claims to hold.
    Truncated,
    /// The recomputed CRC does not match the stored CRC — corrupt or tampered.
    Crc,
}

impl std::fmt::Display for EntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryError::Truncated => write!(f, "log entry truncated"),
            EntryError::Crc => write!(f, "log entry crc mismatch"),
        }
    }
}

impl std::error::Error for EntryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bit_identically() {
        let e = LogEntry::new(7, 42, vec![1, 2, 3, 4, 5]);
        let bytes = e.encode();
        let back = LogEntry::decode(&bytes).expect("decodes");
        assert_eq!(e, back);
        // Encoding is a pure function of the fields.
        assert_eq!(bytes, e.encode());
    }

    #[test]
    fn empty_payload_round_trips() {
        let e = LogEntry::new(0, 0, Vec::new());
        assert_eq!(LogEntry::decode(&e.encode()).unwrap(), e);
    }

    #[test]
    fn flipped_byte_is_rejected() {
        let e = LogEntry::new(1, 1, vec![9, 9, 9]);
        let mut bytes = e.encode();
        bytes[10] ^= 0xff;
        assert_eq!(LogEntry::decode(&bytes), Err(EntryError::Crc));
    }

    #[test]
    fn truncated_is_rejected() {
        let e = LogEntry::new(1, 1, vec![9, 9, 9]);
        let bytes = e.encode();
        assert_eq!(
            LogEntry::decode(&bytes[..bytes.len() - 2]),
            Err(EntryError::Truncated)
        );
    }
}
