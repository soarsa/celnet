//! End-to-end aggregated-book composite test (D3): push `LpQuote`s over the real
//! `LiquidityFeedService.LpFeed` gRPC ingest from ≥2 `LP-SIM-NN` venues for ≥2
//! instruments into a live per-book engine (created over the admin API, so the
//! reconcile-on-CRUD path stands the engine up), then subscribe to the book over
//! the multiplexed `StreamService.StreamSession` and assert the published
//! `AggregatedBookSnapshot` carries the surviving-member envelope: `best_bid =
//! max(fresh bids)`, `best_offer = min(fresh offers)`, the right per-LP
//! contributions (VenueId + ISIN identity), a confidence in `(0, 1]`, and a stale
//! member dropped out of the best price.
//!
//! Every body is hard wall-clock bounded and every stream await is bounded, so a
//! regression surfaces fast rather than hanging.

mod common;

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use celnet_proto::auth_service_client::AuthServiceClient;
use celnet_proto::liquidity_feed_service_client::LiquidityFeedServiceClient;
use celnet_proto::stream_service_client::StreamServiceClient;
use celnet_proto::{
    AggregatedBookSpec, AggregatedBookSubscribe, AggregationParamsDesc, AggregationScopeMode,
    ClientStreamMessage, CreateAggregatedBookRequest, LpQuote, ServerStreamMessage, SubscriptionId,
    client_stream_message, server_stream_message,
};
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;

use common::{STEP_DEADLINE, TEST_DEADLINE, login_seed_admin, start_ready_edge};

/// Outbound client stream wrapping a bounded mpsc receiver, so the test can push the
/// subscribe onto the bidirectional session and keep it open while reading.
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

/// Current epoch nanoseconds (the valuation instant the fresh quotes are stamped at,
/// against the server's own system clock).
fn epoch_now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    )
    .unwrap_or(i64::MAX)
}

fn lpq(lp: &str, instrument: &str, bid: f64, offer: f64, ts: i64) -> LpQuote {
    LpQuote {
        lp_name: lp.to_string(),
        instrument_id: instrument.to_string(),
        bid,
        offer,
        bid_size: 1_000_000.0,
        offer_size: 2_000_000.0,
        ts_nanos: ts,
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[tokio::test]
async fn lp_feed_flows_through_to_aggregated_book_subscribers() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, addr, _data_dir) = start_ready_edge().await;
        let base = format!("http://{addr}");
        let token = login_seed_admin(&base).await;

        // --- 1. Create the aggregated book over the real admin API. This commits the
        // definition AND re-reconciles the engine hub, so the book is live. Scope is
        // the two SEEDED reference-data instruments (so their ISIN identity resolves).
        let mut auth = AuthServiceClient::connect(base.clone())
            .await
            .expect("auth client connects");
        let created = tokio::time::timeout(
            STEP_DEADLINE,
            auth.create_aggregated_book(CreateAggregatedBookRequest {
                session_token: token.clone(),
                spec: Some(AggregatedBookSpec {
                    id: String::new(),
                    name: "UST Composite".to_string(),
                    member_connection_ids: vec![
                        "LP-SIM-01".to_string(),
                        "LP-SIM-02".to_string(),
                        "LP-SIM-03".to_string(),
                    ],
                    scope_mode: AggregationScopeMode::Explicit as i32,
                    instrument_ids: vec!["ust-2y-note".to_string(), "acme-5y-corp".to_string()],
                    params: Some(AggregationParamsDesc {
                        staleness_tau_ms: 30_000,
                        max_quote_age_ms: 5_000,
                        divergence_gating: false,
                        min_contributors: 1,
                        depth_levels: 1,
                    }),
                    enabled: true,
                }),
                correlation_id: Some(1),
            }),
        )
        .await
        .expect("create resolves in time")
        .expect("create succeeds")
        .into_inner()
        .book
        .expect("created book echoed");
        let book_id = created.id;

        // --- 2. Push LP quotes from three venues for two instruments over the real
        // gRPC LpFeed ingest. On `ust-2y-note` all three are fresh. On `acme-5y-corp`
        // LP-SIM-02 posts the best two-way but is stamped 60 s stale (> the 5 s hard
        // cutoff), so it must drop out of the composite best price.
        let now = epoch_now();
        let stale = now - 60 * 1_000_000_000;
        let quotes = vec![
            lpq("LP-SIM-01", "ust-2y-note", 99.90, 100.10, now),
            lpq("LP-SIM-02", "ust-2y-note", 99.95, 100.05, now), // sets best bid + offer
            lpq("LP-SIM-03", "ust-2y-note", 99.80, 100.20, now),
            lpq("LP-SIM-01", "acme-5y-corp", 98.00, 98.50, now),
            lpq("LP-SIM-02", "acme-5y-corp", 98.20, 98.30, stale), // best if fresh — but stale
            lpq("LP-SIM-03", "acme-5y-corp", 98.10, 98.40, now),
        ];
        let mut feed = LiquidityFeedServiceClient::connect(base.clone())
            .await
            .expect("liquidity feed client connects");
        let ack = tokio::time::timeout(
            STEP_DEADLINE,
            feed.lp_feed(tonic::Request::new(futures_util::stream::iter(quotes))),
        )
        .await
        .expect("lp_feed resolves in time")
        .expect("lp_feed succeeds")
        .into_inner();
        assert_eq!(ack.accepted, 6, "all six quotes routed into the book");

        // --- 3. Subscribe to the book's composite over the multiplexed session. The
        // ingest RPC has returned, so the sink is populated: the baseline snapshot
        // already carries the consolidated composite.
        let mut sc = StreamServiceClient::connect(base.clone())
            .await
            .expect("stream client connects");
        let (tx, rx) = mpsc::channel::<ClientStreamMessage>(16);
        let sub_id = SubscriptionId { value: 1 };
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::AggregatedBookSubscribe(
                AggregatedBookSubscribe {
                    subscription: Some(sub_id),
                    book_id: book_id.clone(),
                    throttle_nanos: 0,
                    correlation_id: Some(9),
                },
            )),
        })
        .await
        .expect("subscribe enqueued");

        let mut inbound =
            tokio::time::timeout(STEP_DEADLINE, sc.stream_session(ClientOutbound { rx }))
                .await
                .expect("stream opens in time")
                .expect("stream opens")
                .into_inner();

        // Read frames until a non-empty composite arrives (baseline or first delta).
        let composite = loop {
            match next_msg(&mut inbound).await {
                server_stream_message::Message::AggregatedBookStreamSnapshot(s) => {
                    let book = s.book.expect("snapshot carries a book");
                    if !book.instruments.is_empty() {
                        assert_eq!(s.sequence, 1, "baseline snapshot is sequence 1");
                        assert_eq!(s.correlation_id, Some(9), "correlation echoed");
                        break book;
                    }
                }
                server_stream_message::Message::AggregatedBookStreamUpdate(u) => {
                    let book = u.book.expect("update carries a book");
                    if !book.instruments.is_empty() {
                        break book;
                    }
                }
                other => panic!("unexpected frame on the composite line: {other:?}"),
            }
        };

        assert_eq!(composite.book_id, book_id);
        assert_eq!(composite.instruments.len(), 2, "both in-scope instruments");
        // Instruments are ordered by instrument_id: "acme-5y-corp" < "ust-2y-note".
        let acme = &composite.instruments[0];
        let ust = &composite.instruments[1];

        // ust-2y-note: all fresh ⇒ best bid = max(99.95), best offer = min(100.05).
        assert_eq!(ust.instrument_id, "ust-2y-note");
        assert_eq!(
            ust.isin, "US91282CKM23",
            "ISIN resolved from reference data"
        );
        assert!(
            close(ust.best_bid, 99.95),
            "best bid = max fresh bid, got {}",
            ust.best_bid
        );
        assert!(
            close(ust.best_offer, 100.05),
            "best offer = min fresh offer, got {}",
            ust.best_offer
        );
        assert_eq!(ust.contributions.len(), 3, "three members reported");
        assert!(
            ust.contributions.iter().all(|c| !c.stale),
            "all ust members are fresh"
        );
        assert!(
            ust.confidence > 0.0 && ust.confidence <= 1.0,
            "confidence in (0,1], got {}",
            ust.confidence
        );

        // acme-5y-corp: LP-SIM-02 is stale ⇒ survivors LP-1 / LP-3 set the envelope:
        // best bid = 98.10 (LP-3), best offer = 98.40 (LP-3) — NOT LP-2's tighter
        // 98.20 / 98.30. Validate against the surviving-member envelope, never the
        // crossed would-be BBO.
        assert_eq!(acme.instrument_id, "acme-5y-corp");
        assert_eq!(
            acme.isin, "US000402AA77",
            "ISIN resolved from reference data"
        );
        assert!(
            close(acme.best_bid, 98.10),
            "stale LP-2's 98.20 bid did not set the composite, got {}",
            acme.best_bid
        );
        assert!(
            close(acme.best_offer, 98.40),
            "stale LP-2's 98.30 offer did not set the composite, got {}",
            acme.best_offer
        );
        let lp2 = acme
            .contributions
            .iter()
            .find(|c| c.lp_name == "LP-SIM-02")
            .expect("LP-SIM-02 is still reported (as excluded)");
        assert!(lp2.stale, "the aged-out member is flagged stale");
        assert!(
            acme.contributions.iter().filter(|c| !c.stale).count() == 2,
            "exactly the two fresh members contribute"
        );
    })
    .await
    .expect("the end-to-end composite test completes before the deadline");
}
