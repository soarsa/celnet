//! Fuzz target: arbitrary bytes -> the replog **snapshot** decode stack.
//!
//! A consensus [`celnet_replog::Snapshot`] is shipped leader→follower over a TCP
//! socket (the InstallSnapshot RPC) and written to disk; a follower decodes it
//! from raw bytes. The decode is layered:
//!   `Snapshot::decode`  (CRC + header framing)
//!     └─ `BookState::decode`  (entry-count framing, `count * 16` overflow guard)
//! and the smallest unit, `BookUpdate::decode` (a single tagged mutation), is the
//! per-entry replicated-state parser. All three are hand-rolled byte parsers fed
//! untrusted input, so all three are fuzzed here from the same arbitrary buffer.
//!
//! Contract under any input bytes (for each of the three decoders):
//!   * never panics — no slice overrun, no `unwrap`/`expect` on attacker length,
//!     no allocation driven unboundedly by a crafted `count`/`state_len`, and
//!   * returns `Ok(_)` or a typed error
//!     ([`celnet_replog::SnapshotError`] / [`celnet_replog::UpdateError`]).
//!
//! On the `Ok` path of `BookState`/`Snapshot` we assert re-encode soundness, and
//! for `BookState` that the entry count implied by the bytes is bounded by the
//! input length (the `count * 16` guard actually held).
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run replog_snapshot -- -max_total_time=120

#![no_main]

use libfuzzer_sys::fuzz_target;

use celnet_replog::{BookState, BookUpdate, Snapshot};

fuzz_target!(|data: &[u8]| {
    // 1) Whole-snapshot decode (CRC-framed capture of the applied book state).
    match Snapshot::decode(data) {
        Ok(snap) => {
            // Re-encode/decode must be an identity on the accepted path.
            let re = snap.encode();
            let back = Snapshot::decode(&re).expect("re-encoded snapshot must decode");
            assert_eq!(snap, back, "snapshot encode/decode is not an identity");
        }
        Err(_typed) => {} // SnapshotError::Malformed — an honest rejection.
    }

    // 2) The embedded canonical book-state codec (the `count * 16` overflow guard
    //    is the classic length-amplification attack point).
    match BookState::decode(data) {
        Ok(state) => {
            // The decoded book can hold at most `(len-8)/16` entries; allocation is
            // therefore bounded by the input, and the round-trip must be exact.
            let max_entries = data.len().saturating_sub(8) / 16;
            assert!(
                state.len() <= max_entries,
                "decoded {} entries from {} bytes (> {} max)",
                state.len(),
                data.len(),
                max_entries
            );
            let re = state.encode();
            let back = BookState::decode(&re).expect("re-encoded book state must decode");
            assert_eq!(
                state.to_bits(),
                back.to_bits(),
                "book-state codec is not bit-identical on Ok"
            );
        }
        Err(_typed) => {} // UpdateError::Truncated — an honest rejection.
    }

    // 3) A single replicated mutation (the per-entry payload parser).
    match BookUpdate::decode(data) {
        Ok(update) => {
            // Re-encoding a decoded update reproduces a buffer that decodes back to
            // the same logical update (float fields compared by bits for NaN/-0.0).
            let back = BookUpdate::decode(&update.encode()).expect("re-encoded update must decode");
            assert!(
                book_update_bit_eq(&update, &back),
                "book-update codec is not an identity on Ok"
            );
        }
        Err(_typed) => {} // Truncated / UnknownTag — honest rejections.
    }
});

/// Bit-exact equality of two [`BookUpdate`]s — float fields are compared by their
/// raw IEEE-754 bits so a NaN payload or signed zero round-trips correctly.
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
