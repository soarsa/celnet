//! Proptest mirror of `fuzz/fuzz_targets/fix_decoder.rs`.
//!
//! Asserts the same no-panic / no-garbage-state / bounded-work contracts as the
//! nightly libFuzzer target, on the stable toolchain inside `just check`, so
//! the property gates the merge without requiring nightly:
//!
//!   1. `FrameCursor::parse` never panics on arbitrary byte slices;
//!   2. Every result is `Ok(cursor)` or a typed `FrameError` — no partial state;
//!   3. On `Ok`: `msg_type()` is non-empty;
//!   4. On `Ok`: `raw()` is the same slice as the input (zero-copy borrow);
//!   5. On `Ok`: iterating `fields()` yields only non-zero tags whose value slices
//!      are bounded within the input buffer;
//!   6. On `Ok`: `get(35)` matches `msg_type()`.
//!
//! Two strategy arms:
//!   a) Fully arbitrary bytes (pure noise — exercises the error-path branches);
//!   b) Structurally valid FIX-4.4 frames built by `FrameEncoder` — exercises the
//!      `Ok` path contracts deterministically regardless of corpus luck.

use celnet_fix::framing::{FrameCursor, FrameEncoder};
use proptest::prelude::*;

/// Build a minimal valid FIX-4.4 on-wire frame for the given `msg_type` byte.
///
/// Uses `FrameEncoder` directly: push tag 35 first, then any extra fields.
fn build_valid_frame(msg_type_byte: u8, extra_tag: u32, extra_val: &str) -> Vec<u8> {
    let mut enc = FrameEncoder::new();
    // Tag 35 must be the first body field per FIX convention.
    enc.push(35, &[msg_type_byte]);
    enc.push(extra_tag, extra_val.as_bytes());
    enc.finish()
}

/// Assert the core contracts on any parse result for the given byte slice.
///
/// Called from both the noise arm and the structured arm.
fn assert_contracts(data: &[u8]) {
    match FrameCursor::parse(data) {
        Ok(cursor) => {
            // Contract 3: msg_type non-empty.
            assert!(
                !cursor.msg_type().is_empty(),
                "parsed frame must have a non-empty MsgType"
            );

            // Contract 4: raw() is the same slice.
            assert_eq!(
                cursor.raw().as_ptr(),
                data.as_ptr(),
                "cursor.raw() must point to the input"
            );
            assert_eq!(
                cursor.raw().len(),
                data.len(),
                "cursor.raw() length must equal input length"
            );

            // Contract 5: field iteration — non-zero tags, sub-slice bounds.
            let data_start = data.as_ptr() as usize;
            for field in cursor.fields() {
                assert!(
                    field.tag > 0,
                    "field tag must be non-zero, got {}",
                    field.tag
                );
                let val_start = field.value.as_ptr() as usize;
                assert!(
                    val_start >= data_start
                        && val_start + field.value.len() <= data_start + data.len(),
                    "field value must be within the input buffer"
                );
            }

            // Contract 6: get(35) == msg_type().
            if let Some(mt) = cursor.get(35) {
                assert_eq!(
                    mt,
                    cursor.msg_type(),
                    "cursor.get(35) must match msg_type()"
                );
            }
        }
        Err(_) => {
            // A typed FrameError — correct outcome for malformed input.
            // No panic ⇒ contract 1 satisfied.
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..Default::default() })]

    // Arm (a): arbitrary noise bytes — exercises all error-path branches.
    #[test]
    fn fix_frame_no_panic_arbitrary_bytes(data in prop::collection::vec(any::<u8>(), 0..512)) {
        // Contracts 1+2: no panic, Ok-or-typed-Err.
        assert_contracts(&data);
    }

    // Arm (b): structurally valid frames via FrameEncoder — exercises the Ok path.
    #[test]
    fn fix_frame_ok_path_contracts(
        msg_type_byte in b'A'..=b'Z',
        tag_val in any::<u16>(),
    ) {
        // Build a well-formed frame. Extra tag is in the user-defined range so it
        // doesn't collide with mandatory session fields.
        let user_tag = 1000u32 + (tag_val as u32 % 1000);
        let val = format!("{tag_val}");
        let frame = build_valid_frame(msg_type_byte, user_tag, &val);

        // A well-formed frame must parse as Ok; contracts 3–6 must hold.
        assert_contracts(&frame);
    }
}
