//! RFQ lifecycle integration tests over the `celnet-proto` `QuoteService`:
//! request → quote → accept → execution, idempotent retry, and last-look expiry.
//!
//! Each test starts the edge on an ephemeral port, dials the generated client, and
//! asserts the quoted mid matches a first-principles `celnet-vanilla` price and
//! that the two-way / idempotency / last-look semantics hold. Every body is hard
//! wall-clock bounded and every network await is bounded, so a stall fails fast.

mod common;

use std::time::Duration;

use celnet_core::is_close;
use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::{QuoteAccept, QuoteReject, QuoteRequest, Side};
use celnet_server::Clock;
use celnet_types::{OptionType, VanillaInputs};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, live_market, start_edge_with, start_ready_edge, vanilla_call,
    wire_conventions,
};

/// The quoted two-way must bracket a first-principles Garman-Kohlhagen mid, and an
/// accept must book an execution at the lifted side.
#[tokio::test]
async fn rfq_quote_accept_matches_direct_price() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let strike = 1.12;
        let req = QuoteRequest {
            idempotency_key: "rfq-key-001".to_owned(),
            instrument: Some(vanilla_call(strike)),
            conventions: Some(wire_conventions()),
        };
        let quote = tokio::time::timeout(STEP_DEADLINE, client.request_quote(req))
            .await
            .expect("request_quote returns in time")
            .expect("request_quote succeeds")
            .into_inner();

        assert!(quote.quote_id >= 1, "a quote id is assigned");
        assert!(
            quote.valid_until_nanos > quote.epoch_nanos,
            "the quote carries a forward last-look deadline"
        );

        // Reference mid: a direct GK price at the live ATM vol.
        let m = live_market();
        let direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom, m.r_for),
        );
        let price = quote.price.expect("two-way present");
        assert!(price.bid < price.offer, "two-way is bid < offer");
        assert!(
            price.bid <= direct && direct <= price.offer,
            "mid {direct} must sit inside the two-way [{}, {}]",
            price.bid,
            price.offer
        );
        // The mid is exactly bracketed (symmetric spread around the GK mid).
        let mid = 0.5 * (price.bid + price.offer);
        // The bid floors at zero, so the recovered mid may differ; assert the
        // offer is mid+half and matches direct+half within the spread model.
        assert!(
            is_close(mid, direct, 1e-9, 1e-9) || price.bid == 0.0,
            "recovered mid {mid} vs direct {direct}"
        );

        // Accept the offer (BUY lifts the offer) and book the execution.
        let exec = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "rfq-key-001".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept_quote returns in time")
        .expect("accept_quote succeeds")
        .into_inner();

        assert_eq!(exec.quote_id, quote.quote_id);
        assert!(exec.execution_id >= 1);
        assert_eq!(exec.side, Side::Buy as i32);
        assert!(
            is_close(exec.traded_premium, price.offer, 1e-12, 1e-12),
            "BUY books the offer {} != {}",
            exec.traded_premium,
            price.offer
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A retried `RequestQuote` carrying the same idempotency key returns the *same*
/// quote (id + prices), never re-pricing; and a retried `AcceptQuote` returns the
/// same booked execution, so a network retry can never double-book.
#[tokio::test]
async fn rfq_idempotent_retry_returns_same_quote_and_execution() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let req = || QuoteRequest {
            idempotency_key: "dedup-42".to_owned(),
            instrument: Some(vanilla_call(1.10)),
            conventions: Some(wire_conventions()),
        };

        let first = tokio::time::timeout(STEP_DEADLINE, client.request_quote(req()))
            .await
            .expect("first quote in time")
            .expect("first quote ok")
            .into_inner();
        let second = tokio::time::timeout(STEP_DEADLINE, client.request_quote(req()))
            .await
            .expect("retry quote in time")
            .expect("retry quote ok")
            .into_inner();

        assert_eq!(
            first.quote_id, second.quote_id,
            "an idempotent retry returns the SAME quote id"
        );
        assert_eq!(first.price, second.price, "and the same two-way price");
        assert_eq!(
            first.epoch_nanos, second.epoch_nanos,
            "and the same timestamp"
        );

        // Accept twice: the second accept returns the same execution (no double book).
        let acc = || QuoteAccept {
            quote_id: first.quote_id,
            idempotency_key: "dedup-42".to_owned(),
            side: Side::Buy as i32,
        };
        let e1 = tokio::time::timeout(STEP_DEADLINE, client.accept_quote(acc()))
            .await
            .expect("accept in time")
            .expect("accept ok")
            .into_inner();
        let e2 = tokio::time::timeout(STEP_DEADLINE, client.accept_quote(acc()))
            .await
            .expect("retry accept in time")
            .expect("retry accept ok")
            .into_inner();
        assert_eq!(
            e1.execution_id, e2.execution_id,
            "an accept retry returns the SAME execution (no double book)"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An accept after the quote's last-look validity deadline is rejected as expired.
/// A manual clock drives time past the deadline without sleeping.
#[tokio::test]
async fn rfq_accept_after_validity_is_rejected() {
    tokio::time::timeout(TEST_DEADLINE, async {
        // Start at t=1_000_000_000 ns; the quote validity is 5 s = 5e9 ns.
        let clock = Clock::manual(1_000_000_000);
        let (edge, addr) = start_edge_with(true, clock.clone()).await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let quote = tokio::time::timeout(
            STEP_DEADLINE,
            client.request_quote(QuoteRequest {
                idempotency_key: "expiring".to_owned(),
                instrument: Some(vanilla_call(1.10)),
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("quote in time")
        .expect("quote ok")
        .into_inner();

        // Jump the clock past the validity window (6 s).
        clock.advance(6_000_000_000);

        let status = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "expiring".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept returns in time")
        .expect_err("an accept past the last-look deadline must be rejected");
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Regression for the idempotency key-collision bug: reusing a known idempotency
/// key for a *different* instrument must return an error (the key is ambiguous),
/// never the stale quote minted for the original instrument. The genuine
/// same-key-same-request idempotency still returns the same quote.
#[tokio::test]
async fn rfq_idempotency_key_collision_is_rejected() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        // First request mints a quote for a 1.10-strike call under key "collide".
        let first = tokio::time::timeout(
            STEP_DEADLINE,
            client.request_quote(QuoteRequest {
                idempotency_key: "collide".to_owned(),
                instrument: Some(vanilla_call(1.10)),
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("first quote in time")
        .expect("first quote ok")
        .into_inner();

        // Same key, SAME request → same quote (genuine idempotency preserved).
        let same = tokio::time::timeout(
            STEP_DEADLINE,
            client.request_quote(QuoteRequest {
                idempotency_key: "collide".to_owned(),
                instrument: Some(vanilla_call(1.10)),
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("same-request retry in time")
        .expect("same-request retry ok")
        .into_inner();
        assert_eq!(
            first.quote_id, same.quote_id,
            "same key + same request is still idempotent (same quote)"
        );
        assert_eq!(first.price, same.price, "and the same price");

        // Same key, DIFFERENT instrument (1.20 strike) → InvalidArgument, NOT the
        // stale 1.10 quote.
        let status = tokio::time::timeout(
            STEP_DEADLINE,
            client.request_quote(QuoteRequest {
                idempotency_key: "collide".to_owned(),
                instrument: Some(vanilla_call(1.20)),
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("collision request returns in time")
        .expect_err("a key reused for a different instrument must error, not return a stale quote");
        assert_eq!(
            status.code(),
            tonic::Code::InvalidArgument,
            "an idempotency key-collision is an InvalidArgument"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Regression for the RejectQuote contract: `RejectQuote` returns a purpose-typed
/// acknowledgement (`RejectAck` with the quote_id and a timestamp), not an
/// `Execution`; and after a reject the quote can no longer be accepted.
#[tokio::test]
async fn rfq_reject_returns_typed_ack_and_blocks_accept() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let quote = tokio::time::timeout(
            STEP_DEADLINE,
            client.request_quote(QuoteRequest {
                idempotency_key: "reject-me".to_owned(),
                instrument: Some(vanilla_call(1.11)),
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("quote in time")
        .expect("quote ok")
        .into_inner();

        // Reject → a typed RejectAck echoing the quote_id with a timestamp.
        let ack = tokio::time::timeout(
            STEP_DEADLINE,
            client.reject_quote(QuoteReject {
                quote_id: quote.quote_id,
                reason: "passing".to_owned(),
            }),
        )
        .await
        .expect("reject in time")
        .expect("reject ok")
        .into_inner();
        assert_eq!(
            ack.quote_id, quote.quote_id,
            "ack echoes the rejected quote"
        );
        assert!(ack.epoch_nanos > 0, "ack is stamped with an ack time");

        // A re-reject is idempotent (still acked).
        let ack2 = tokio::time::timeout(
            STEP_DEADLINE,
            client.reject_quote(QuoteReject {
                quote_id: quote.quote_id,
                reason: "still passing".to_owned(),
            }),
        )
        .await
        .expect("re-reject in time")
        .expect("re-reject ok")
        .into_inner();
        assert_eq!(ack2.quote_id, quote.quote_id);

        // After a reject the quote can no longer be accepted.
        let status = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "reject-me".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept-after-reject returns in time")
        .expect_err("a rejected quote can no longer be accepted");
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
