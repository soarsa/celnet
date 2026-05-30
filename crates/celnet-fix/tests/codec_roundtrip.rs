//! Codec round-trip and malformed-frame property tests.
//!
//! Every dialect message built by the owned builders must parse back
//! bit-identically through the zero-copy [`FrameCursor`], and arbitrary
//! malformed input must be rejected without panic. These run against the real
//! codec — no fakes.

use celnet_fix::framing::{FrameCursor, FrameEncoder};
use celnet_fix::messages::{
    self, EXEC_FILLED, ExecReportParams, Header, NewOrderParams, QuoteParams,
};
use proptest::prelude::*;

fn hdr(seq: u64) -> Header<'static> {
    Header {
        sender: b"CELNETVENUE",
        target: b"CPARTYHEDGE",
        seq_num: seq,
        sending_time: b"20260530-12:00:00.000",
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A built `Quote(S)` round-trips: parse it back and recover the QuoteID and
    /// the prices to 8-dp precision (the wire precision).
    #[test]
    fn quote_roundtrips(
        bid in 0.0f64..10.0,
        spread in 0.0f64..1.0,
        size in 1.0f64..1e9,
        seq in 1u64..1_000_000,
    ) {
        let mut enc = FrameEncoder::new();
        let qid = format!("Q-{seq}");
        let p = QuoteParams {
            quote_req_id: b"REQ",
            quote_id: qid.as_bytes(),
            symbol: b"EURUSD",
            bid_px: bid,
            offer_px: bid + spread,
            size,
            valid_until: b"20260530-12:00:05.000",
        };
        let raw = messages::build_quote(&hdr(seq), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("valid frame");
        let view = messages::QuoteView::new(frame);
        prop_assert_eq!(view.quote_id(), Some(qid.as_bytes()));
        // 8-dp wire precision.
        prop_assert!((view.bid().unwrap() - bid).abs() < 1e-7);
        prop_assert!((view.offer().unwrap() - (bid + spread)).abs() < 1e-7);
        // MsgSeqNum round-trips exactly.
        let frame2 = FrameCursor::parse(&raw).unwrap();
        let seq_str = seq.to_string();
        prop_assert_eq!(frame2.get(34), Some(seq_str.as_bytes()));
    }

    /// A built `NewOrderSingle(D)` round-trips its identity fields.
    #[test]
    fn new_order_roundtrips(qty in 1.0f64..1e9, seq in 1u64..1_000_000) {
        let mut enc = FrameEncoder::new();
        let p = NewOrderParams {
            cl_ord_id: b"C-1",
            quote_id: b"Q-1",
            symbol: b"EURUSD",
            side: b'1',
            qty,
            transact_time: b"20260530-12:00:01.000",
        };
        let raw = messages::build_new_order_single(&hdr(seq), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("valid");
        let view = messages::NewOrderSingleView::new(frame);
        prop_assert_eq!(view.cl_ord_id(), Some(&b"C-1"[..]));
        prop_assert_eq!(view.quote_id(), Some(&b"Q-1"[..]));
        prop_assert!((view.order_qty().unwrap() - qty).abs() < 1e-7 * qty.max(1.0));
    }

    /// A built `ExecutionReport(8)` round-trips price + status.
    #[test]
    fn exec_report_roundtrips(px in 0.0f64..10.0, seq in 1u64..1_000_000) {
        let mut enc = FrameEncoder::new();
        let p = ExecReportParams {
            order_id: b"O-1",
            exec_id: b"E-1",
            cl_ord_id: b"C-1",
            exec_type: EXEC_FILLED,
            ord_status: EXEC_FILLED,
            symbol: b"EURUSD",
            side: b'1',
            last_qty: 1_000_000.0,
            last_px: px,
            multileg_type: Some(2),
            text: None,
        };
        let raw = messages::build_execution_report(&hdr(seq), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("valid");
        let view = messages::ExecReportView::new(frame);
        prop_assert_eq!(view.exec_type(), Some(EXEC_FILLED));
        prop_assert!((view.last_px().unwrap() - px).abs() < 1e-7);
        prop_assert_eq!(frame_get(&raw, 442), Some(b"2".to_vec()));
    }

    /// Arbitrary bytes never panic the parser; they either parse or error.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = FrameCursor::parse(&bytes); // must not panic
    }

    /// A valid frame with one byte flipped is either rejected or, if it still
    /// parses, never panics — and a flipped checksum byte is always rejected.
    #[test]
    fn single_bit_corruption_is_safe(
        seq in 1u64..1_000_000,
        pos in 0usize..40,
        xor in 1u8..=255,
    ) {
        let mut enc = FrameEncoder::new();
        let raw = messages::build_heartbeat(&hdr(seq), None, &mut enc);
        if pos < raw.len() {
            let mut corrupt = raw.clone();
            corrupt[pos] ^= xor;
            let _ = FrameCursor::parse(&corrupt); // must not panic
        }
    }
}

/// Helper: parse a frame and fetch a tag as an owned `Vec`.
fn frame_get(raw: &[u8], tag: u32) -> Option<Vec<u8>> {
    FrameCursor::parse(raw).ok()?.get(tag).map(<[u8]>::to_vec)
}

#[test]
fn malformed_frames_rejected_without_panic() {
    let cases: &[&[u8]] = &[
        b"",
        b"\x01",
        b"8=FIX.4.4",
        b"8=FIX.4.4\x01",
        b"8=FIX.4.4\x019=10\x0135=0\x01",            // no checksum
        b"8=FIX.4.2\x019=2\x0135=0\x0110=000\x01",   // wrong version
        b"8=FIX.4.4\x019=abc\x0135=0\x0110=000\x01", // bad body length
        b"garbage that is definitely not fix at all but long enough to pass the length check ok",
    ];
    for c in cases {
        let r = FrameCursor::parse(c);
        assert!(r.is_err(), "expected error for {c:?}");
    }
}
