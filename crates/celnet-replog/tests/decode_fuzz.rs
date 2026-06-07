//! Property-based adversarial-bytes hardening of the untrusted byte decoders.
//!
//! `celnet-replog` parses bytes that arrive over a **real TCP socket** from a
//! peer and bytes read back from **disk** — both genuinely untrusted inputs. The
//! hand-rolled decoders (`LogEntry::decode`, `Message::decode`, `Snapshot::decode`,
//! `BookState::decode`, `BookUpdate::decode`) must therefore satisfy a hard
//! contract on **arbitrary** bytes:
//!
//!   * **no panic** — no index-out-of-bounds, no `unwrap`/`expect` on an
//!     attacker-controlled length, no slice overrun, no arithmetic overflow;
//!   * **Ok-or-typed-Err** — every result is a valid value or one of the crate's
//!     own error enums, never a partial/garbage value;
//!   * **bounded allocation** — a crafted length field can never make a decoder
//!     allocate more than the input bears out.
//!
//! These are the *same* properties the nightly `cargo-fuzz` targets in `fuzz/`
//! assert under coverage-guided search. The fuzz crate is nightly-only and
//! excluded from the workspace gate, so this file re-establishes the property
//! with `proptest` inside `just check` — coverage-guided fuzzing and randomized
//! property testing are complementary, and the gate must hold the line regardless
//! of whether a nightly fuzz lane ran.
//!
//! The strategies deliberately mix three byte-shapes: uniform-random bytes
//! (broad coverage), bytes biased toward decoder structure (a valid frame whose
//! tail is then corrupted — the torn-tail / CRC-flip corners), and
//! near-boundary-length buffers (off-by-one truncation).

use proptest::prelude::*;

use celnet_replog::{BookState, BookUpdate, LogEntry, Message, Snapshot};

/// A strategy producing adversarial byte buffers up to 4 KiB:
///   * fully random bytes, and
///   * "valid-ish" frames built from a real encoder then mutated (a flipped byte
///     and/or a truncated tail), to drive the CRC / length-overrun branches that
///     pure noise reaches only rarely.
fn adversarial_bytes() -> impl Strategy<Value = Vec<u8>> {
    let random = prop::collection::vec(any::<u8>(), 0..4096);

    // A real `LogEntry` frame, then optionally flip a byte and/or truncate — the
    // structurally-valid-but-corrupt corner shared by every framed decoder.
    let mutated_entry = (
        any::<u64>(),
        any::<u64>(),
        prop::collection::vec(any::<u8>(), 0..256),
        any::<Option<usize>>(),
        any::<Option<usize>>(),
    )
        .prop_map(|(term, index, payload, flip, truncate)| {
            let mut bytes = LogEntry::new(term, index, payload).encode();
            if let Some(i) = flip
                && !bytes.is_empty()
            {
                let idx = i % bytes.len();
                bytes[idx] ^= 0xff;
            }
            if let Some(t) = truncate
                && !bytes.is_empty()
            {
                bytes.truncate(t % bytes.len());
            }
            bytes
        });

    // A real wire `Message` frame (entry-bearing AppendEntries), similarly mutated.
    let mutated_message = (
        any::<u64>(),
        prop::collection::vec(
            (
                any::<u64>(),
                any::<u64>(),
                prop::collection::vec(any::<u8>(), 0..64),
            ),
            0..8,
        ),
        any::<Option<usize>>(),
    )
        .prop_map(|(term, raw_entries, truncate)| {
            let entries = raw_entries
                .into_iter()
                .enumerate()
                .map(|(i, (t, _idx, p))| LogEntry::new(t, i as u64, p))
                .collect();
            let mut bytes = Message::AppendEntries {
                term,
                prev_log_index: u64::MAX,
                prev_log_term: 0,
                entries,
                leader_commit: u64::MAX,
            }
            .encode();
            if let Some(t) = truncate
                && !bytes.is_empty()
            {
                bytes.truncate(t % bytes.len());
            }
            bytes
        });

    prop_oneof![random, mutated_entry, mutated_message]
}

proptest! {
    // A wide budget — these decoders are pure and fast, so we can afford many cases.
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// `LogEntry::decode` never panics and yields a typed result; on `Ok` the
    /// decode is a sound inverse of encode and the payload is bounded by the input.
    #[test]
    fn log_entry_decode_total(data in adversarial_bytes()) {
        match LogEntry::decode(&data) {
            Ok(entry) => {
                prop_assert!(entry.payload.len() <= data.len());
                let back = LogEntry::decode(&entry.encode())
                    .expect("re-encoded entry must decode");
                prop_assert_eq!(entry, back);
            }
            Err(_typed) => {} // EntryError::{Truncated,Crc}
        }
    }

    /// `Message::decode` never panics and yields a typed result; decoded entry
    /// lists / snapshot payloads are bounded by the input, and `Ok` round-trips.
    #[test]
    fn wire_message_decode_total(data in adversarial_bytes()) {
        match Message::decode(&data) {
            Ok(msg) => {
                match &msg {
                    Message::AppendEntries { entries, .. } => {
                        prop_assert!(entries.len() <= data.len());
                    }
                    Message::InstallSnapshot { snapshot_bytes, .. } => {
                        prop_assert!(snapshot_bytes.len() <= data.len());
                    }
                    _ => {}
                }
                let back = Message::decode(&msg.encode())
                    .expect("re-encoded message must decode");
                prop_assert_eq!(msg, back);
            }
            Err(_typed) => {} // WireError::{Malformed,FrameTooLarge,Io}
        }
    }

    /// `Snapshot::decode` never panics and yields a typed result; `Ok` round-trips.
    #[test]
    fn snapshot_decode_total(data in adversarial_bytes()) {
        match Snapshot::decode(&data) {
            Ok(snap) => {
                let back = Snapshot::decode(&snap.encode())
                    .expect("re-encoded snapshot must decode");
                prop_assert_eq!(snap, back);
            }
            Err(_typed) => {} // SnapshotError::Malformed
        }
    }

    /// `BookState::decode` never panics; the decoded entry count is bounded by the
    /// `count * 16` overflow guard (no length amplification), and `Ok` is bit-exact.
    #[test]
    fn book_state_decode_total(data in adversarial_bytes()) {
        match BookState::decode(&data) {
            Ok(state) => {
                let max_entries = data.len().saturating_sub(8) / 16;
                prop_assert!(state.len() <= max_entries);
                let back = BookState::decode(&state.encode())
                    .expect("re-encoded book state must decode");
                prop_assert_eq!(state.to_bits(), back.to_bits());
            }
            Err(_typed) => {} // UpdateError::Truncated
        }
    }

    /// `BookUpdate::decode` never panics and yields a typed result; `Ok` round-trips
    /// bit-exactly (float payloads compared by `to_bits` for NaN/-0.0 safety).
    #[test]
    fn book_update_decode_total(data in adversarial_bytes()) {
        match BookUpdate::decode(&data) {
            Ok(update) => {
                let back = BookUpdate::decode(&update.encode())
                    .expect("re-encoded update must decode");
                prop_assert!(book_update_bit_eq(&update, &back));
            }
            Err(_typed) => {} // UpdateError::{Truncated,UnknownTag}
        }
    }
}

/// Bit-exact equality of two [`BookUpdate`]s (float fields by raw IEEE-754 bits).
fn book_update_bit_eq(a: &BookUpdate, b: &BookUpdate) -> bool {
    match (a, b) {
        (BookUpdate::Set { key: ka, value: va }, BookUpdate::Set { key: kb, value: vb }) => {
            ka == kb && va.to_bits() == vb.to_bits()
        }
        (BookUpdate::Add { key: ka, delta: da }, BookUpdate::Add { key: kb, delta: db }) => {
            ka == kb && da.to_bits() == db.to_bits()
        }
        (BookUpdate::Remove { key: ka }, BookUpdate::Remove { key: kb }) => ka == kb,
        _ => false,
    }
}
