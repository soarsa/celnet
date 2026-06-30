//! Multiplexed RFS session integration tests over the `celnet-proto`
//! `StreamService::StreamSession`: click-to-trade (book a streamed line by its
//! stamped `TradableToken`), reject of a forged/stale token, `Modify` in place, and
//! multiplexed subscriptions over ONE bidirectional session.
//!
//! Every body is hard wall-clock bounded and every stream await is bounded, so a
//! regression surfaces as a fast failure, never an infinite hang.

mod common;

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use celnet_proto::stream_service_client::StreamServiceClient;
use celnet_proto::{
    ClientStreamMessage, Execute, Modify, ServerStreamMessage, Side, Subscribe, SubscriptionId,
    TradableToken, client_stream_message, server_stream_message, stream_reject,
};
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;

use common::{STEP_DEADLINE, TEST_DEADLINE, start_ready_edge, vanilla_call, wire_conventions};

/// A minimal outbound client stream wrapping a bounded mpsc receiver, so the test
/// can push control + click-to-trade messages onto the bidirectional session.
struct ClientOutbound {
    rx: mpsc::Receiver<ClientStreamMessage>,
}

impl Stream for ClientOutbound {
    type Item = ClientStreamMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Pull the next server stream message within the step deadline.
async fn next_msg(
    stream: &mut tonic::Streaming<ServerStreamMessage>,
) -> server_stream_message::Message {
    tokio::time::timeout(STEP_DEADLINE, stream.next())
        .await
        .expect("a server message arrives before the deadline")
        .expect("the stream stays open")
        .expect("the message is well-formed")
        .message
        .expect("a non-empty server message")
}

fn subscribe(sub_id: SubscriptionId, strike: f64) -> ClientStreamMessage {
    ClientStreamMessage {
        message: Some(client_stream_message::Message::Subscribe(Subscribe {
            subscription: Some(sub_id),
            instrument: Some(vanilla_call(strike)),
            conventions: Some(wire_conventions()),
            throttle_nanos: 0,
            correlation_id: Some(0x1234),
            surface_version: None,
            attribution: None,
        })),
    }
}

/// A streamed line carries two click-to-trade tokens (SELL@bid, BUY@offer); an
/// `Execute` presenting the live BUY token books an `Executed` at exactly the
/// stamped offer, with the correlation id echoed. A forged token is rejected.
#[tokio::test]
async fn rfs_click_to_trade_books_streamed_line_and_rejects_forged_token() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr, _data_dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            StreamServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let (tx, rx) = mpsc::channel::<ClientStreamMessage>(16);
        let sub_id = SubscriptionId { value: 42 };
        tx.send(subscribe(sub_id, 1.10)).await.unwrap();

        let mut inbound =
            tokio::time::timeout(STEP_DEADLINE, client.stream_session(ClientOutbound { rx }))
                .await
                .expect("stream opens in time")
                .expect("stream opens")
                .into_inner();

        // The baseline snapshot carries the click-to-trade tokens + echoed corr id.
        let snap = match next_msg(&mut inbound).await {
            server_stream_message::Message::Snapshot(s) => s,
            other => panic!("expected a Snapshot first, got {other:?}"),
        };
        assert_eq!(snap.subscription, Some(sub_id));
        assert_eq!(
            snap.correlation_id,
            Some(0x1234),
            "snapshot echoes the open corr id"
        );
        assert!(
            !snap.tradable.is_empty(),
            "the streamed line stamps click-to-trade tokens"
        );
        let buy: &TradableToken = snap
            .tradable
            .iter()
            .find(|t| t.side == Side::Buy as i32)
            .expect("a BUY token (lift the offer)");
        let buy_token = buy.token;
        let buy_premium = buy.premium;

        // A FORGED token is rejected as UnknownToken (books nothing).
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Execute(Execute {
                subscription: Some(sub_id),
                token: 0x0BAD_F00D,
                idempotency_key: "forged".to_owned(),
                correlation_id: Some(0x1234),
            })),
        })
        .await
        .unwrap();

        // Then book the genuine BUY token.
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Execute(Execute {
                subscription: Some(sub_id),
                token: buy_token,
                idempotency_key: "click-buy-1".to_owned(),
                correlation_id: Some(0x1234),
            })),
        })
        .await
        .unwrap();

        // Scan the interleaved stream for the reject and the booking.
        let mut saw_reject = false;
        let mut booked = None;
        for _ in 0..256 {
            match next_msg(&mut inbound).await {
                server_stream_message::Message::StreamReject(r) if r.token == 0x0BAD_F00D => {
                    assert_eq!(r.reason, stream_reject::Reason::UnknownToken as i32);
                    saw_reject = true;
                }
                server_stream_message::Message::Executed(e) if e.token == buy_token => {
                    booked = Some(e);
                    break;
                }
                _ => continue,
            }
        }
        assert!(saw_reject, "the forged token was rejected as unknown");
        let exec = booked.expect("the genuine BUY token booked an Executed");
        assert_eq!(exec.side, Side::Buy as i32, "BUY lifted the offer");
        assert_eq!(
            exec.traded_premium.to_bits(),
            buy_premium.to_bits(),
            "booked at exactly the stamped offer premium"
        );
        assert_eq!(
            exec.correlation_id,
            Some(0x1234),
            "executed echoes the corr id"
        );
        assert!(exec.execution_id >= 1, "a booking id is assigned");

        drop(tx);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A second click on a now-consumed token (a different idempotency key) is rejected
/// as AlreadyConsumed — a token books at most once (no double-execution).
#[tokio::test]
async fn rfs_double_click_on_consumed_token_is_rejected() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr, _data_dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            StreamServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let (tx, rx) = mpsc::channel::<ClientStreamMessage>(16);
        let sub_id = SubscriptionId { value: 7 };
        tx.send(subscribe(sub_id, 1.10)).await.unwrap();
        let mut inbound =
            tokio::time::timeout(STEP_DEADLINE, client.stream_session(ClientOutbound { rx }))
                .await
                .expect("stream opens in time")
                .expect("stream opens")
                .into_inner();

        let snap = match next_msg(&mut inbound).await {
            server_stream_message::Message::Snapshot(s) => s,
            other => panic!("expected a Snapshot, got {other:?}"),
        };
        let token = snap
            .tradable
            .iter()
            .find(|t| t.side == Side::Buy as i32)
            .expect("a BUY token")
            .token;

        // First click books.
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Execute(Execute {
                subscription: Some(sub_id),
                token,
                idempotency_key: "first".to_owned(),
                correlation_id: None,
            })),
        })
        .await
        .unwrap();
        // Second click on the same token with a different key is AlreadyConsumed.
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Execute(Execute {
                subscription: Some(sub_id),
                token,
                idempotency_key: "second".to_owned(),
                correlation_id: None,
            })),
        })
        .await
        .unwrap();

        let mut saw_exec = false;
        let mut saw_consumed = false;
        for _ in 0..256 {
            match next_msg(&mut inbound).await {
                server_stream_message::Message::Executed(e) if e.token == token => saw_exec = true,
                server_stream_message::Message::StreamReject(r) if r.token == token => {
                    assert_eq!(r.reason, stream_reject::Reason::AlreadyConsumed as i32);
                    saw_consumed = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(saw_exec, "the first click booked");
        assert!(
            saw_consumed,
            "the second click on the consumed token was rejected"
        );

        drop(tx);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A `Modify` re-baselines a live subscription in place: the server answers with a
/// fresh `Snapshot` at the next sequence carrying the new structure, without
/// tearing the subscription down. Multiplexed: a second subscription on the SAME
/// session is unaffected.
#[tokio::test]
async fn rfs_modify_rebaselines_in_place_on_multiplexed_session() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr, _data_dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            StreamServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let (tx, rx) = mpsc::channel::<ClientStreamMessage>(16);
        let sub_a = SubscriptionId { value: 100 };
        let sub_b = SubscriptionId { value: 200 };
        // Two subscriptions multiplexed over ONE session.
        tx.send(subscribe(sub_a, 1.10)).await.unwrap();
        tx.send(subscribe(sub_b, 1.20)).await.unwrap();
        let mut inbound =
            tokio::time::timeout(STEP_DEADLINE, client.stream_session(ClientOutbound { rx }))
                .await
                .expect("stream opens in time")
                .expect("stream opens")
                .into_inner();

        // Collect both baseline snapshots (order not guaranteed across subs).
        let mut seen_a = false;
        let mut seen_b = false;
        for _ in 0..32 {
            if let server_stream_message::Message::Snapshot(s) = next_msg(&mut inbound).await {
                if s.subscription == Some(sub_a) {
                    seen_a = true;
                }
                if s.subscription == Some(sub_b) {
                    seen_b = true;
                }
            }
            if seen_a && seen_b {
                break;
            }
        }
        assert!(
            seen_a && seen_b,
            "both multiplexed subscriptions got a baseline"
        );

        // Modify sub_a in place to a different strike.
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Modify(Modify {
                subscription: Some(sub_a),
                instrument: Some(vanilla_call(1.05)),
                conventions: Some(wire_conventions()),
                throttle_nanos: 0,
                surface_version: None,
            })),
        })
        .await
        .unwrap();

        // The next snapshot for sub_a is the re-baseline at a higher sequence.
        let mut rebaselined = false;
        for _ in 0..256 {
            if let server_stream_message::Message::Snapshot(s) = next_msg(&mut inbound).await
                && s.subscription == Some(sub_a)
                && s.sequence >= 2
            {
                rebaselined = true;
                break;
            }
        }
        assert!(
            rebaselined,
            "modify answered with a fresh snapshot at the next sequence (in-place rebaseline)"
        );

        drop(tx);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
