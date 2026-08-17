//! Session FSM driven over a REAL `tokio` loopback TCP socket — our acceptor
//! against our own initiator, no fakes. Proves the full recovery handshake:
//! logon → heartbeat → sequence gap → resend request → gap-fill.
//!
//! Every async body is wrapped in a `tokio::time::timeout` so a protocol hang
//! fails fast rather than wedging the test runner. The listener binds an
//! ephemeral port and is dropped at end of test.

use std::time::Duration;

use celnet_fix::dialect_rates::{self, SubscriptionRequest};
use celnet_fix::framing::{FrameCursor, FrameEncoder};
use celnet_fix::initiator::{Initiator, LiftPolicy};
use celnet_fix::messages::{self, Header};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig, SessionState};
use celnet_fix::transport::{FrameReader, write_frame};
use celnet_proto::{AccrualBasis, BrokenDate, PaymentFrequency, Side};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const T: &[u8] = b"20260530-12:00:00.000";
const DEADLINE: Duration = Duration::from_secs(5);

fn acc_cfg() -> SessionConfig {
    SessionConfig {
        sender: b"VENUE".to_vec(),
        target: b"TAKER".to_vec(),
        heart_bt_int: 30,
        role: Role::Acceptor,
    }
}

fn init_cfg() -> SessionConfig {
    SessionConfig {
        sender: b"TAKER".to_vec(),
        target: b"VENUE".to_vec(),
        heart_bt_int: 30,
        role: Role::Initiator,
    }
}

#[tokio::test]
async fn logon_heartbeat_gap_resend_gapfill_over_socket() {
    let body = async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // --- Acceptor task: drives a session over the accepted socket. ---
        let acceptor = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (rd, mut wr) = tokio::io::split(stream);
            let mut reader = FrameReader::new(rd);
            let mut sess = Session::new(acc_cfg(), InMemoryStore::new());
            let mut saw_resend_request = false;
            // Process inbound frames until the initiator closes.
            while let Ok(Some(frame)) = reader.next_frame().await {
                let action = sess.on_inbound(&frame, T).expect("valid inbound");
                for f in &action.outbound {
                    write_frame(&mut wr, f).await.unwrap();
                    let parsed = FrameCursor::parse(f).unwrap();
                    if parsed.msg_type() == b"2" {
                        saw_resend_request = true;
                    }
                }
            }
            (sess.state(), saw_resend_request, sess.expected_inbound())
        });

        // --- Initiator side: real client socket. ---
        let stream = TcpStream::connect(addr).await.unwrap();
        let (rd, mut wr) = tokio::io::split(stream);
        let mut reader = FrameReader::new(rd);
        let mut sess = Session::new(init_cfg(), InMemoryStore::new());

        // 1. Logon.
        let logon = sess.start_logon(T, false);
        write_frame(&mut wr, &logon).await.unwrap();
        // Receive the acceptor's logon mirror.
        let mirror = reader.next_frame().await.unwrap().unwrap();
        sess.on_inbound(&mirror, T).unwrap();
        assert_eq!(sess.state(), SessionState::Active);

        // 2. Heartbeat (seq 2).
        let hb = sess.send_heartbeat(T);
        write_frame(&mut wr, &hb).await.unwrap();

        // 3. Create a gap: build seq 3 and seq 4 but only SEND seq 4 (seq 3 is
        //    "lost"). The acceptor expects 3, receives 4 → resend request.
        let _hb3 = sess.send_heartbeat(T); // seq 3 (deliberately not sent)
        let hb4 = sess.send_heartbeat(T); // seq 4
        write_frame(&mut wr, &hb4).await.unwrap();

        // 4. Expect a ResendRequest(2) from the acceptor for [3, 0).
        let rr = reader.next_frame().await.unwrap().unwrap();
        let rr_frame = FrameCursor::parse(&rr).unwrap();
        assert_eq!(rr_frame.msg_type(), b"2", "expected resend request");
        // begin should be the acceptor's expected seq (3).
        assert_eq!(rr_frame.get(7), Some(&b"3"[..]));

        // 5. Respond with a gap-fill SequenceReset(4) advancing to seq 5.
        let mut enc = FrameEncoder::new();
        let h = Header {
            sender: b"TAKER",
            target: b"VENUE",
            seq_num: 3,
            sending_time: T,
        };
        let gapfill = messages::build_sequence_reset(&h, 5, true, &mut enc);
        write_frame(&mut wr, &gapfill).await.unwrap();

        // 6. Send seq 5 to confirm recovery (acceptor should now accept it).
        // The initiator session's next_out is already 5.
        let hb5 = sess.send_heartbeat(T);
        write_frame(&mut wr, &hb5).await.unwrap();

        // Give the acceptor a moment to drain, then close.
        drop(wr);
        drop(reader);

        let (state, saw_rr, expected_in) = timeout(DEADLINE, acceptor).await.unwrap().unwrap();
        assert_eq!(state, SessionState::Active);
        assert!(saw_rr, "acceptor must have issued a resend request");
        // After the gap-fill (→5) and the seq-5 heartbeat, the acceptor expects 6.
        assert_eq!(expected_in, 6, "sequence recovered after gap-fill");
    };
    timeout(DEADLINE, body).await.expect("test timed out");
}

#[tokio::test]
async fn test_request_heartbeat_over_socket() {
    let body = async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let acceptor = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (rd, mut wr) = tokio::io::split(stream);
            let mut reader = FrameReader::new(rd);
            let mut sess = Session::new(acc_cfg(), InMemoryStore::new());
            while let Ok(Some(frame)) = reader.next_frame().await {
                let action = sess.on_inbound(&frame, T).unwrap();
                for f in &action.outbound {
                    write_frame(&mut wr, f).await.unwrap();
                }
            }
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let (rd, mut wr) = tokio::io::split(stream);
        let mut reader = FrameReader::new(rd);
        let mut sess = Session::new(init_cfg(), InMemoryStore::new());

        let logon = sess.start_logon(T, false);
        write_frame(&mut wr, &logon).await.unwrap();
        let mirror = reader.next_frame().await.unwrap().unwrap();
        sess.on_inbound(&mirror, T).unwrap();

        // Send a TestRequest; expect a Heartbeat echoing the TestReqID.
        let tr = sess.send_test_request(T, b"PING-42");
        write_frame(&mut wr, &tr).await.unwrap();
        let resp = reader.next_frame().await.unwrap().unwrap();
        let frame = FrameCursor::parse(&resp).unwrap();
        assert_eq!(frame.msg_type(), b"0");
        assert_eq!(frame.get(112), Some(&b"PING-42"[..]));

        drop(wr);
        drop(reader);
        drop(sess);
        timeout(DEADLINE, acceptor).await.unwrap().unwrap();
    };
    timeout(DEADLINE, body).await.expect("test timed out");
}

/// A market-data cycle that schedules a LIFT against a venue which publishes **nothing**
/// must still end at its hold deadline.
///
/// Regression for the spin found on UAT 2026-08-17: `collect_md_stream` selected `lift_at`
/// as its wake instant whenever a lift was still pending, without checking that the instant
/// was in the future. On a silent venue no snapshot ever arrives, so `last_symbol` stays
/// `None`, the lift cannot fire, and `pending_lift` stays true — pinning the wake to an
/// instant already in the past. `timeout_at` on a past instant returns `Elapsed`
/// immediately, and the `wake >= deadline` break cannot trigger (because
/// `lift_at < deadline`), so the cycle spun at ~100% CPU forever and never advanced to the
/// next instrument. The RFS and ESP simulator legs wedged this way for 2h35m at ~87% CPU
/// each, contributing 4 pricing events against the RFQ leg's 4,662.
///
/// The venue here logs on and then deliberately answers the `MarketDataRequest(35=V)` with
/// silence — exactly the unquotable-instrument case (a bond no liquidity provider prices).
#[tokio::test]
async fn md_stream_with_a_pending_lift_ends_on_a_silent_venue() {
    let body = async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Venue: complete the logon handshake, then publish NOTHING. Session-level outbound
        // (the logon mirror, heartbeats) still flows; no application data ever does.
        let acceptor = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (rd, mut wr) = tokio::io::split(stream);
            let mut reader = FrameReader::new(rd);
            let mut sess = Session::new(acc_cfg(), InMemoryStore::new());
            while let Ok(Some(frame)) = reader.next_frame().await {
                let Ok(action) = sess.on_inbound(&frame, T) else {
                    continue;
                };
                for f in &action.outbound {
                    if write_frame(&mut wr, f).await.is_err() {
                        return;
                    }
                }
            }
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        // A FIRM lift policy: `lift_after` is only honoured when the policy is not Observe,
        // so an Observe policy would not reproduce the spin.
        let mut initiator = Initiator::new(
            Session::new(init_cfg(), InMemoryStore::new()),
            LiftPolicy::LiftOffer,
        );
        let mut sess = initiator.open(stream, T).await.unwrap();

        let md_req_id = b"MD-SILENT-1".to_vec();
        let symbol = b"912797UA3".to_vec();
        let params = dialect_rates::BondMarketDataRequestParams {
            md_req_id: &md_req_id,
            symbol: &symbol,
            coupon_rate: 0.0,
            coupon_frequency: PaymentFrequency::SemiAnnual,
            day_count: AccrualBasis::Act365Fixed,
            maturity: BrokenDate {
                year: 2026,
                month: 6,
                day: 30,
            },
            redemption: 100.0,
            notional: 1_000_000.0,
            side: Side::TwoWay,
            subscription: SubscriptionRequest::Subscribe,
        };

        // The lift falls due HALF-WAY through the hold — the arrangement the ESP/RFS legs
        // use (`lift_after = hold / 2`), and the one that spun.
        let hold = Duration::from_millis(300);
        let started = std::time::Instant::now();
        let outcome = sess
            .md_stream(
                T,
                dialect_rates::SEC_TYPE_BOND,
                |hdr, enc| dialect_rates::build_bond_market_data_request(hdr, &params, enc),
                hold,
                Some(hold / 2),
            )
            .await
            .expect("md_stream must return, not wedge");
        let elapsed = started.elapsed();

        // It ended on its own hold deadline rather than spinning.
        assert!(
            elapsed < Duration::from_secs(2),
            "md_stream spun past its {hold:?} hold on a silent venue (took {elapsed:?})"
        );
        assert!(
            elapsed >= Duration::from_millis(250),
            "md_stream returned before its hold elapsed ({elapsed:?}) — it should wait the \
             window out, not bail early"
        );
        // Nothing was published, so nothing was streamed and no order can have gone out:
        // a lift with no liftable line must NOT invent one.
        assert_eq!(outcome.updates, 0, "a silent venue publishes no snapshots");
        assert!(!outcome.result.lifted, "no line was ever liftable");
        assert!(!outcome.result.filled, "nothing can fill without a lift");

        drop(sess);
        let _ = timeout(DEADLINE, acceptor).await;
    };
    timeout(DEADLINE, body).await.expect("test timed out");
}
