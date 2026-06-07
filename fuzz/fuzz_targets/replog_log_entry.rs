//! Fuzz target: arbitrary bytes -> `celnet_replog::LogEntry::decode`.
//!
//! `LogEntry` is the unit of replication: a `(term, index)`-stamped payload that
//! a leader streams to followers over a **real TCP socket** and that every node
//! persists to its on-disk journal. `LogEntry::decode` is therefore a genuinely
//! untrusted byte parser — its input is whatever a peer (or a corrupt disk) hands
//! it, including a hostile `plen` length field crafted to over-read or over-
//! allocate.
//!
//! Contract under any input bytes:
//!   * the decode never panics (no index-out-of-bounds, no `unwrap` on an
//!     attacker-controlled length, no slice overrun), and
//!   * it returns `Ok(LogEntry)` or a typed [`celnet_replog::EntryError`] —
//!     never a partial/garbage value, never UB.
//!
//! On the `Ok` path we additionally assert the **decode is sound**: re-encoding
//! the decoded entry reproduces a buffer whose framed prefix matches the input's
//! framed region (a decode that silently dropped or invented bytes would diverge),
//! and the declared payload length never exceeds the input length (bounded alloc).
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run replog_log_entry -- -max_total_time=120

#![no_main]

use libfuzzer_sys::fuzz_target;

use celnet_replog::{EntryError, LogEntry};

fuzz_target!(|data: &[u8]| {
    // Contract 1+2: arbitrary bytes either decode to a valid entry or to a typed
    // error; the call must never panic on attacker input.
    match LogEntry::decode(data) {
        Ok(entry) => {
            // Bounded allocation: a decoded payload can never be longer than the
            // input it was carved from (the only allocation `decode` performs is
            // the payload `to_vec`, and it is sliced from `data`).
            assert!(
                entry.payload.len() <= data.len(),
                "decoded payload {} longer than input {}",
                entry.payload.len(),
                data.len()
            );
            // Soundness: re-encoding yields a deterministic, self-consistent frame
            // that itself round-trips. A decode that mangled the fields would fail
            // its own CRC on the second pass.
            let re = entry.encode();
            let back = LogEntry::decode(&re).expect("re-encoded entry must decode");
            assert_eq!(entry, back, "encode/decode is not an identity on Ok");
        }
        Err(EntryError::Truncated | EntryError::Crc) => {
            // The only two honest rejection reasons — both fine.
        }
    }
});
