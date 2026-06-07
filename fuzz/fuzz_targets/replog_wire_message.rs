//! Fuzz target: arbitrary bytes -> `celnet_replog::Message::decode`.
//!
//! `Message` is the Raft RPC frame exchanged between cluster nodes over real
//! `std::net::TcpStream` sockets (AppendEntries / AppendReply / RequestVote /
//! VoteReply / InstallSnapshot / Status…). `Message::decode` parses the body of a
//! length-prefixed frame straight off the socket, so it is the single most
//! exposed untrusted parser in the consensus layer. Its sub-parsers are the most
//! interesting corners:
//!   * AppendEntries carries an entry **count** then per-entry length-prefixed
//!     `LogEntry` bytes — a hostile count/length is the length-amplification point
//!     (the decoder caps the pre-allocation at 1024 and bounds each `take`), and
//!   * InstallSnapshot carries a length-prefixed snapshot payload bounded by the
//!     frame.
//!
//! Contract under any input bytes:
//!   * never panics — the bounds-checked `Cursor` returns `WireError::Malformed`
//!     on any short read, an unknown tag, or a nested entry that fails its own
//!     CRC/length check; no `unwrap` on attacker length, no unbounded allocation,
//!   * returns `Ok(Message)` or `Err(WireError)`.
//!
//! On the `Ok` path we assert re-encode soundness (decode is a true inverse of
//! encode on accepted input) and that any decoded entry list is bounded by the
//! input length.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run replog_wire_message -- -max_total_time=120

#![no_main]

use libfuzzer_sys::fuzz_target;

use celnet_replog::Message;

fuzz_target!(|data: &[u8]| {
    match Message::decode(data) {
        Ok(msg) => {
            // Bounded allocation: a decoded AppendEntries can carry at most one
            // entry per byte of input (each entry framing is > 1 byte), and an
            // InstallSnapshot payload is bounded by the frame.
            match &msg {
                Message::AppendEntries { entries, .. } => {
                    assert!(
                        entries.len() <= data.len(),
                        "decoded {} entries from {} bytes",
                        entries.len(),
                        data.len()
                    );
                }
                Message::InstallSnapshot { snapshot_bytes, .. } => {
                    assert!(
                        snapshot_bytes.len() <= data.len(),
                        "decoded snapshot {} > input {}",
                        snapshot_bytes.len(),
                        data.len()
                    );
                }
                _ => {}
            }
            // Soundness: encode is a deterministic inverse on the accepted path.
            let re = msg.encode();
            let back = Message::decode(&re).expect("re-encoded message must decode");
            assert_eq!(msg, back, "wire encode/decode is not an identity on Ok");
        }
        Err(_typed) => {} // WireError::{Malformed,FrameTooLarge,Io} — honest rejections.
    }
});
