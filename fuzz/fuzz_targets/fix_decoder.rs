//! Fuzz target: arbitrary bytes -> `celnet_fix::framing::FrameCursor::parse`.
//!
//! `celnet-fix` is the only hand-rolled external-bytes parser in the codebase that
//! did not yet have a fuzz target (verification-contract clause f, W6 spec §4).
//! The FIX framing decoder parses bytes that arrive over a real TCP socket from
//! a FIX counterparty — a genuinely untrusted source — and must therefore satisfy
//! a hard contract on **arbitrary** bytes:
//!
//!   * **No panic** — no index-out-of-bounds, no integer-overflow panic, no slice
//!     over-read behind a crafted `BodyLength` field;
//!   * **Ok-or-typed-Err** — every result is a valid parsed frame or one of the
//!     crate's own [`FrameError`] variants; never a partial/garbage state or an
//!     unreachable;
//!   * **Bounded work** — the parser does no allocation and visits the buffer at most
//!     twice (one pass for `BodyLength` / `CheckSum` location, one for `MsgType`);
//!     a huge crafted `BodyLength` must surface as `BodyLengthMismatch`, never cause
//!     the parser to scan beyond the buffer.
//!
//! Additional contracts asserted on a successful parse (`Ok(cursor)`):
//!   * `cursor.msg_type()` is non-empty (guaranteed by the mandatory tag-35
//!     presence check inside [`FrameCursor::parse`]);
//!   * re-iterating `cursor.fields()` never panics and yields only fields whose
//!     tags are non-zero;
//!   * `cursor.raw()` is the same slice as the input buffer (zero-copy borrow).
//!
//! The coverage-guided engine naturally discovers the `8=FIX.4.4\x01` prologue
//! (10 bytes), then the `9=<n>\x01` body-length field, then the `10=<cs>\x01`
//! trailer, quickly driving the fully-valid and all the one-off-invalid branches.
//!
//! A stable proptest mirror is in
//!   `crates/celnet-fix/tests/frame_fuzz.rs`
//! so this property gates the merge on the stable toolchain too.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run fix_decoder -- -max_total_time=120

#![no_main]

use libfuzzer_sys::fuzz_target;

use celnet_fix::framing::FrameCursor;

fuzz_target!(|data: &[u8]| {
    // Contract: parse never panics; returns Ok or a typed FrameError.
    match FrameCursor::parse(data) {
        Ok(cursor) => {
            // Contract: msg_type is non-empty on a successfully-parsed frame.
            assert!(
                !cursor.msg_type().is_empty(),
                "parsed frame must have a non-empty MsgType"
            );

            // Contract: raw() is the same byte range as the input.
            assert_eq!(
                cursor.raw().as_ptr(),
                data.as_ptr(),
                "cursor.raw() must borrow from the input slice"
            );
            assert_eq!(
                cursor.raw().len(),
                data.len(),
                "cursor.raw() length must equal input length"
            );

            // Contract: iterating fields never panics and yields well-formed tags.
            for field in cursor.fields() {
                // Every tag must be non-zero (a tag of 0 is not a valid FIX tag).
                assert!(
                    field.tag > 0,
                    "field tag must be non-zero, got {}",
                    field.tag
                );
                // Value slice must be a sub-slice of the original buffer.
                let data_start = data.as_ptr() as usize;
                let val_start = field.value.as_ptr() as usize;
                assert!(
                    val_start >= data_start && val_start + field.value.len() <= data_start + data.len(),
                    "field value must be a sub-slice of the input buffer"
                );
            }

            // Contract: `get` for the mandatory MsgType field returns the same
            // bytes as `msg_type()`.
            if let Some(mt_via_get) = cursor.get(35) {
                assert_eq!(
                    mt_via_get,
                    cursor.msg_type(),
                    "cursor.get(35) must return the same bytes as msg_type()"
                );
            }
        }
        Err(_e) => {
            // A typed FrameError is the correct outcome for malformed input.
            // The specific variant is not constrained here — any FrameError variant
            // is acceptable (the parser is free to return whichever is most
            // informative). We only require no panic.
        }
    }
});
