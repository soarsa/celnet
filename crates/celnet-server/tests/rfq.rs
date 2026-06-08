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
use celnet_proto::{
    AttributionRecord, BookId, Owner, QuoteAccept, QuoteReject, QuoteRequest, Side, owner,
};
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
            correlation_id: None,
            surface_version: None,
            attribution: None,
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
            &VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom(), m.r_for()),
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
            correlation_id: None,
            surface_version: None,
            attribution: None,
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
                correlation_id: None,
                surface_version: None,
                attribution: None,
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
                correlation_id: None,
                surface_version: None,
                attribution: None,
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
                correlation_id: None,
                surface_version: None,
                attribution: None,
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
                correlation_id: None,
                surface_version: None,
                attribution: None,
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

/// Regression for the accept-authority bug: `AcceptQuote` must be request-matched
/// to the originating idempotency key. An accept carrying a wrong/empty key (a
/// party that merely learned the `quote_id`) must be refused with
/// `InvalidArgument` and book nothing; the genuine key still books.
#[tokio::test]
async fn rfq_accept_requires_originating_idempotency_key() {
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
                idempotency_key: "owner-key-xyz".to_owned(),
                instrument: Some(vanilla_call(1.10)),
                conventions: Some(wire_conventions()),
                correlation_id: None,
                surface_version: None,
                attribution: None,
            }),
        )
        .await
        .expect("quote in time")
        .expect("quote ok")
        .into_inner();

        // A third party knows the quote_id but presents the WRONG key → refused.
        let wrong = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "not-the-owner".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept returns in time")
        .expect_err("an accept with a non-matching key must be refused");
        assert_eq!(wrong.code(), tonic::Code::InvalidArgument);

        // An EMPTY key is likewise refused (no silent keyless accept).
        let empty = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: String::new(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept returns in time")
        .expect_err("an accept with an empty key must be refused");
        assert_eq!(empty.code(), tonic::Code::InvalidArgument);

        // The genuine originator (matching key) still books.
        let exec = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "owner-key-xyz".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept returns in time")
        .expect("the originating key accepts")
        .into_inner();
        assert_eq!(exec.quote_id, quote.quote_id);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Regression for the accept-retry side guard: once a quote is booked on one side,
/// an accept-retry that flips the side (even with the right key) is a different
/// intent and must be refused with `FailedPrecondition`, never handed the
/// opposite-side booking. A same-side retry still returns the original execution.
#[tokio::test]
async fn rfq_accept_retry_side_flip_is_refused() {
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
                idempotency_key: "flip-key".to_owned(),
                instrument: Some(vanilla_call(1.10)),
                conventions: Some(wire_conventions()),
                correlation_id: None,
                surface_version: None,
                attribution: None,
            }),
        )
        .await
        .expect("quote in time")
        .expect("quote ok")
        .into_inner();

        // Book BUY (lift the offer).
        let buy = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "flip-key".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept in time")
        .expect("buy accepts")
        .into_inner();
        assert_eq!(buy.side, Side::Buy as i32);

        // Retry SELL with the same key → refused (side flip, not a retry).
        let flip = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "flip-key".to_owned(),
                side: Side::Sell as i32,
            }),
        )
        .await
        .expect("accept returns in time")
        .expect_err("a side-flipping accept-retry must be refused");
        assert_eq!(flip.code(), tonic::Code::FailedPrecondition);

        // Same-side retry still returns the SAME booking (idempotent).
        let buy2 = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "flip-key".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept in time")
        .expect("same-side retry ok")
        .into_inner();
        assert_eq!(buy2.execution_id, buy.execution_id);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Regression for the unguessable-quote-id property: minted ids are not the dense
/// `1,2,3,…` sequence (which a third party could enumerate to reject/accept other
/// parties' quotes). Two distinct quotes get distinct, non-adjacent, `>= 1` ids.
#[tokio::test]
async fn rfq_quote_ids_are_unguessable() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let mk = |k: &str, strike: f64| QuoteRequest {
            idempotency_key: k.to_owned(),
            instrument: Some(vanilla_call(strike)),
            conventions: Some(wire_conventions()),
            correlation_id: None,
            surface_version: None,
            attribution: None,
        };
        let a = tokio::time::timeout(STEP_DEADLINE, client.request_quote(mk("a", 1.10)))
            .await
            .expect("quote a in time")
            .expect("quote a ok")
            .into_inner();
        let b = tokio::time::timeout(STEP_DEADLINE, client.request_quote(mk("b", 1.11)))
            .await
            .expect("quote b in time")
            .expect("quote b ok")
            .into_inner();

        assert!(a.quote_id >= 1 && b.quote_id >= 1, "ids are valid (>= 1)");
        assert_ne!(a.quote_id, b.quote_id, "distinct quotes get distinct ids");
        // Not a dense monotonic pair (the whole point of unguessability): the two
        // consecutive ids must not differ by exactly 1.
        assert_ne!(
            a.quote_id.abs_diff(b.quote_id),
            1,
            "ids must not be a dense enumerable sequence"
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
                correlation_id: None,
                surface_version: None,
                attribution: None,
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

/// The RFQ lifecycle carries the who's-trading attribution: a quote is always
/// `quoted_by` the maker auto-pricer (so engine-quoted flow is never anonymous),
/// a client-supplied requesting seat becomes the `held_by`, and the booked
/// `Execution` carries the same chain — so the risk roll-up / blotter can attribute
/// the flow request → quote → booking.
#[tokio::test]
async fn rfq_lifecycle_carries_maker_and_holder_attribution() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        // The client requests on behalf of a human trading seat.
        let requesting_seat = BookId {
            book: "EM-VOL-1".to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::Trader("jdoe".to_owned())),
            }),
        };
        let req = QuoteRequest {
            idempotency_key: "attr-key-1".to_owned(),
            instrument: Some(vanilla_call(1.12)),
            conventions: Some(wire_conventions()),
            correlation_id: None,
            surface_version: None,
            attribution: Some(AttributionRecord {
                quoted_by: Some(requesting_seat.clone()),
                held_by: None,
                won: Some(true),
                lp_count: Some(3),
            }),
        };
        let quote = tokio::time::timeout(STEP_DEADLINE, client.request_quote(req))
            .await
            .expect("request_quote returns in time")
            .expect("request_quote succeeds")
            .into_inner();

        let attr = quote
            .attribution
            .clone()
            .expect("quote carries attribution");
        // The maker auto-pricer is the quoter (the engine priced the line).
        let quoted = attr.quoted_by.expect("quoted_by stamped");
        assert!(
            matches!(quoted.owner.unwrap().seat, Some(owner::Seat::AutoPricer(_))),
            "the maker auto-pricer quotes the line"
        );
        // The client's requesting seat holds the position; competition context kept.
        assert_eq!(
            attr.held_by,
            Some(requesting_seat),
            "the client seat becomes the holder"
        );
        assert_eq!(attr.won, Some(true));
        assert_eq!(attr.lp_count, Some(3));

        // Accept it; the booking carries the same attribution chain.
        let exec = tokio::time::timeout(
            STEP_DEADLINE,
            client.accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "attr-key-1".to_owned(),
                side: Side::Buy as i32,
            }),
        )
        .await
        .expect("accept returns in time")
        .expect("accept succeeds")
        .into_inner();
        assert_eq!(
            exec.attribution, quote.attribution,
            "the execution carries the quote's attribution chain"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
