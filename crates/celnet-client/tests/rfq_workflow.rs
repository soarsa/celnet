//! Trader-workflow integration tests for the RFQ lifecycle over the typed SDK.
//!
//! These drive a real in-process `celnet-server` edge through the [`celnet_client`]
//! SDK and assert the client result equals the underlying analytics crate's direct
//! computation:
//!
//! * **Scenario 2 — a taker RFQs a double-no-touch, accepts, and books.** The
//!   quoted two-way must bracket a first-principles `celnet-exotics`
//!   double-no-touch price, and the accept books an execution at the lifted side.
//! * **Scenario 5 — idempotent quote retry.** A retried request under the same
//!   [`celnet_client::Rfq`] handle returns the same quote; a retried accept returns
//!   the same execution — a network retry never double-books.
//!
//! Every body is hard wall-clock bounded and every network await is bounded.

mod common;

use std::time::Duration;

use celnet_client::{
    BarrierKind as SdkBarrierKind, BarrierSide as SdkBarrierSide, BarrierTerms, Conventions,
    DigitalTerms, DoubleBarrierTerms, InstrumentSpec, Quantity, Side, StrikeSpec,
};
use celnet_core::is_close;
use celnet_exotics::{
    BarrierKind as ExBarrierKind, BarrierStyle, DigitalKind, DoubleBarrierKnockOut, DoubleNoTouch,
    RebateTiming, SingleBarrier as ExSingleBarrier, digital_price, double_knock_out_price,
    double_no_touch_price, one_touch_price, single_barrier_price,
};
use celnet_server::Clock;
use celnet_types::{OptionType, Tenor, VanillaInputs};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, conventions, eurusd, live_market, start_edge_and_client,
    start_edge_and_client_with, vanilla_call,
};

/// Build a 1Y EURUSD double-no-touch with a `[lower, upper]` corridor and a
/// rebate, in the typed SDK vocabulary.
fn dnt(lower: f64, upper: f64, rebate: f64) -> InstrumentSpec {
    InstrumentSpec::double_no_touch(
        eurusd(),
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        celnet_client::TouchTerms::double_no_touch(lower, upper, rebate),
    )
}

/// Scenario 2: a taker requests a quote on a double-no-touch, the quoted two-way
/// brackets the first-principles `celnet-exotics` DNT price, and accepting the
/// offer books an execution at the lifted side.
#[tokio::test]
async fn taker_rfqs_dnt_accepts_and_books() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;

        let lower = 1.00;
        let upper = 1.20;
        let rebate = 250_000.0;
        let rfq = client.request_quote(dnt(lower, upper, rebate), conventions());

        let quote = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("request returns in time")
            .expect("request succeeds");

        assert!(quote.quote_id >= 1, "a quote id is assigned");
        assert!(
            quote.valid_until_nanos > quote.epoch_nanos,
            "the quote carries a forward last-look deadline"
        );
        assert!(
            quote.price.bid < quote.price.offer,
            "two-way is bid < offer: {:?}",
            quote.price
        );

        // Reference: a direct `celnet-exotics` DNT price at the live ATM market.
        let m = live_market();
        let direct = double_no_touch_price(
            &(&VanillaInputs::new(m.spot, lower, m.vol, 1.0, m.r_dom, m.r_for)).into(),
            DoubleNoTouch::new(lower, upper, rebate),
        );
        assert!(
            quote.price.bid <= direct && direct <= quote.price.offer,
            "DNT mid {direct} must sit inside the two-way [{}, {}]",
            quote.price.bid,
            quote.price.offer
        );

        // Accept the offer (BUY lifts the offer) and book the execution.
        let exec = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&quote, Side::Buy))
            .await
            .expect("accept returns in time")
            .expect("accept succeeds");

        assert_eq!(exec.quote_id, quote.quote_id);
        assert!(exec.execution_id >= 1);
        assert_eq!(exec.side, Side::Buy);
        assert!(
            is_close(exec.traded_premium, quote.price.offer, 1e-12, 1e-12),
            "BUY books the offer {} != {}",
            exec.traded_premium,
            quote.price.offer
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A vanilla RFQ's quoted mid equals the first-principles Garman-Kohlhagen price
/// (the SDK surfaces the maker's deterministic price verbatim), and the resolved
/// strike echoes the requested absolute strike.
#[tokio::test]
async fn vanilla_rfq_mid_equals_direct_garman_kohlhagen() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;

        let strike = 1.12;
        let rfq = client.request_quote(vanilla_call(strike), conventions());
        let quote = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("request in time")
            .expect("request ok");

        let m = live_market();
        let direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom, m.r_for),
        );
        // The two-way is symmetric around the GK mid; the recovered mid matches
        // direct unless the bid floored at zero.
        let recovered = quote.price.mid();
        assert!(
            is_close(recovered, direct, 1e-9, 1e-9) || quote.price.bid == 0.0,
            "recovered mid {recovered} vs direct {direct}"
        );
        assert!(
            is_close(quote.resolved_strike, strike, 1e-12, 1e-12),
            "resolved strike {} echoes the requested {strike}",
            quote.resolved_strike
        );
        // The full Greek set arrives typed and matches the direct GK greeks.
        let direct_greeks = celnet_vanilla::greeks(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom, m.r_for),
        );
        assert!(is_close(quote.greeks.vega, direct_greeks.vega, 1e-9, 1e-9));
        assert!(is_close(
            quote.greeks.delta_spot,
            direct_greeks.delta_spot,
            1e-9,
            1e-9
        ));

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Scenario 5: an idempotent quote retry. A retried `request` under the same
/// [`celnet_client::Rfq`] handle returns the same quote (id + prices + timestamp),
/// and a retried `accept` returns the same execution — a network retry can never
/// re-price or double-book. The SDK relays the caller's single idempotency key.
#[tokio::test]
async fn idempotent_quote_retry_never_double_books() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;

        let rfq = client.request_quote(vanilla_call(1.10), conventions());
        // The handle owns one stable key reused on every call.
        let key = rfq.idempotency_key().to_owned();
        assert!(!key.is_empty(), "the SDK minted an idempotency key");

        let first = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("first quote in time")
            .expect("first quote ok");
        // Simulate a timeout-then-retry: re-issue the SAME request handle.
        let retry = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("retry quote in time")
            .expect("retry quote ok");

        assert_eq!(
            first.quote_id, retry.quote_id,
            "an idempotent retry returns the SAME quote id"
        );
        assert_eq!(first.price.bid, retry.price.bid, "and the same two-way bid");
        assert_eq!(
            first.price.offer, retry.price.offer,
            "and the same two-way offer"
        );
        assert_eq!(
            first.epoch_nanos, retry.epoch_nanos,
            "and the same publication timestamp (never re-priced)"
        );
        assert_eq!(retry.idempotency_key, key, "the key is echoed back");

        // Accept twice under the same handle: the second accept returns the same
        // execution (no double book).
        let e1 = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&first, Side::Buy))
            .await
            .expect("accept in time")
            .expect("accept ok");
        let e2 = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&first, Side::Buy))
            .await
            .expect("retry accept in time")
            .expect("retry accept ok");
        assert_eq!(
            e1.execution_id, e2.execution_id,
            "an accept retry returns the SAME execution (no double book)"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An accept after the quote's last-look validity window is rejected by the maker;
/// the SDK surfaces it as a `deadline_exceeded` status the caller can branch on. A
/// manual clock drives time past the deadline without sleeping.
#[tokio::test]
async fn accept_after_last_look_is_rejected() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let clock = Clock::manual(1_000_000_000);
        let (edge, client, _data_dir) = start_edge_and_client_with(clock.clone()).await;

        let rfq = client.request_quote(vanilla_call(1.10), conventions());
        let quote = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("quote in time")
            .expect("quote ok");

        // Jump the clock past the 5-second validity window.
        clock.advance(6_000_000_000);

        let err = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&quote, Side::Buy))
            .await
            .expect("accept returns in time")
            .expect_err("an accept past the last-look deadline must be rejected");
        match err {
            celnet_client::ClientError::Status(s) => {
                assert_eq!(s.code(), tonic::Code::DeadlineExceeded);
            }
            other => panic!("expected a status error, got {other:?}"),
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Sanity: two RFQ handles minted on the same client carry distinct idempotency
/// keys, so independent requests never collide.
#[tokio::test]
async fn distinct_rfqs_have_distinct_keys() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        let a = client.request_quote(vanilla_call(1.10), Conventions::major_default());
        let b = client.request_quote(vanilla_call(1.12), Conventions::major_default());
        assert_ne!(
            a.idempotency_key(),
            b.idempotency_key(),
            "distinct RFQ handles get distinct keys"
        );
        // Both quote independently.
        let qa = tokio::time::timeout(STEP_DEADLINE, a.request())
            .await
            .expect("a in time")
            .expect("a ok");
        let qb = tokio::time::timeout(STEP_DEADLINE, b.request())
            .await
            .expect("b in time")
            .expect("b ok");
        assert_ne!(qa.quote_id, qb.quote_id, "distinct quotes get distinct ids");

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

// Keep the `StrikeSpec` import meaningful even if a future refactor drops its
// only direct use here (the helpers in `common` use it); referencing it documents
// the strike vocabulary the RFQ tests speak.
const _: fn() = || {
    let _ = StrikeSpec::Absolute(1.0);
};

/// Reject coherence end-to-end: `Rfq::reject` returns a typed [`RejectAck`]
/// echoing the quote_id (not an `Execution`), and after a reject the quote can no
/// longer be accepted — the SDK surfaces the maker's `failed_precondition`.
#[tokio::test]
async fn reject_returns_typed_ack_and_quote_cannot_be_accepted() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;

        let rfq = client.request_quote(vanilla_call(1.10), conventions());
        let quote = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("quote in time")
            .expect("quote ok");

        // Reject returns a typed acknowledgement carrying the quote_id + a time.
        let ack = tokio::time::timeout(STEP_DEADLINE, rfq.reject(&quote, "passing"))
            .await
            .expect("reject in time")
            .expect("reject ok");
        assert_eq!(
            ack.quote_id, quote.quote_id,
            "ack echoes the rejected quote"
        );
        assert!(ack.epoch_nanos > 0, "ack is stamped with an ack time");

        // After a reject the quote can no longer be accepted.
        let err = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&quote, Side::Buy))
            .await
            .expect("accept-after-reject returns in time")
            .expect_err("a rejected quote can no longer be accepted");
        match err {
            celnet_client::ClientError::Status(status) => {
                assert_eq!(status.code(), tonic::Code::FailedPrecondition);
            }
            other => panic!("expected a FailedPrecondition status, got {other:?}"),
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// W4 Track C — api-first close for the originally-contracted barrier / digital /
/// touch products: the SDK now exposes ergonomic [`InstrumentSpec`] constructors
/// for them, and a request built through those constructors and priced over a real
/// edge equals the first-principles `celnet-exotics` reference — the same closed
/// form the server's own pricer evaluates. This proves the contracted-but-formerly-
/// unbuildable products are now reachable through the typed SDK with no contract or
/// server change (field numbers single_barrier=9 / double_barrier=10 / digital=11 /
/// touch=12 are unchanged on the wire).
///
/// Coverage: a single-barrier EURUSD up-and-out call, a double-barrier knock-out
/// call, a cash-or-nothing digital call, and a one-touch — each through its own
/// ergonomic constructor, each reconciled to the independent `celnet-exotics` oracle
/// to machine precision.
#[tokio::test]
async fn sdk_ctors_price_barriers_digital_touch_equal_exotics_reference() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;

        let m = live_market();
        // The headline `greeks.price` is per 1 unit of base notional — the same raw
        // per-unit quantity the `celnet-exotics` closed forms return — so the
        // reconciliation is direct, with the notional left on the `Instrument`.
        let inputs = |strike: f64| VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom, m.r_for);

        // ---- single-barrier: EURUSD 1Y up-and-out call, strike 1.10, H = 1.30 ----
        let strike = 1.10;
        let barrier = 1.30;
        let rebate = 0.0;
        let up_and_out = InstrumentSpec::single_barrier(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            BarrierTerms::new(
                OptionType::Call,
                StrikeSpec::Absolute(strike),
                SdkBarrierKind::KnockOut,
                SdkBarrierSide::Up,
                barrier,
            )
            .rebate(rebate),
        );
        let priced =
            tokio::time::timeout(STEP_DEADLINE, client.price(&up_and_out, m, conventions()))
                .await
                .expect("single-barrier price returns in time")
                .expect("single-barrier price succeeds");
        let sb_ref = single_barrier_price(
            &(&inputs(strike)).into(),
            ExSingleBarrier {
                kind: ExBarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike,
                barrier,
                rebate,
            },
        );
        assert!(
            is_close(priced.greeks.price, sb_ref, 1e-12, 1e-12),
            "up-and-out call SDK price {} != celnet-exotics reference {sb_ref}",
            priced.greeks.price
        );
        assert!(
            is_close(priced.resolved_strike, strike, 1e-12, 1e-12),
            "single-barrier echoes the requested strike"
        );

        // ---- double-barrier: 1Y knock-out call, K = 1.10, corridor [0.95, 1.25] ----
        let lower = 0.95;
        let upper = 1.25;
        let dbarrier = InstrumentSpec::double_barrier(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            DoubleBarrierTerms::new(
                OptionType::Call,
                StrikeSpec::Absolute(strike),
                SdkBarrierKind::KnockOut,
                lower,
                upper,
            ),
        );
        let priced_db =
            tokio::time::timeout(STEP_DEADLINE, client.price(&dbarrier, m, conventions()))
                .await
                .expect("double-barrier price returns in time")
                .expect("double-barrier price succeeds");
        let db_ref = double_knock_out_price(
            &(&inputs(strike)).into(),
            DoubleBarrierKnockOut::new(OptionType::Call, strike, lower, upper),
        );
        assert!(
            is_close(priced_db.greeks.price, db_ref, 1e-12, 1e-12),
            "double knock-out call SDK price {} != celnet-exotics reference {db_ref}",
            priced_db.greeks.price
        );

        // ---- digital: 1Y cash-or-nothing call, K = 1.12, payout = 1.0 -------------
        let dig_strike = 1.12;
        let payout = 1.0;
        let digital = InstrumentSpec::digital(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            DigitalTerms::cash_or_nothing(OptionType::Call, dig_strike, payout),
        );
        let priced_dig =
            tokio::time::timeout(STEP_DEADLINE, client.price(&digital, m, conventions()))
                .await
                .expect("digital price returns in time")
                .expect("digital price succeeds");
        let dig_ref = payout
            * digital_price(
                DigitalKind::cash(OptionType::Call),
                &(&inputs(dig_strike)).into(),
            );
        assert!(
            is_close(priced_dig.greeks.price, dig_ref, 1e-12, 1e-12),
            "cash-or-nothing digital SDK price {} != celnet-exotics reference {dig_ref}",
            priced_dig.greeks.price
        );

        // ---- one-touch: 1Y, barrier 1.25 above spot, rebate 1.0 ------------------
        let touch_barrier = 1.25;
        let touch_rebate = 1.0;
        let one_touch = InstrumentSpec::one_touch(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            touch_barrier,
            touch_rebate,
        );
        let priced_ot =
            tokio::time::timeout(STEP_DEADLINE, client.price(&one_touch, m, conventions()))
                .await
                .expect("one-touch price returns in time")
                .expect("one-touch price succeeds");
        // A touch has no strike; the server prices on inputs with strike = barrier.
        let ot_ref = one_touch_price(
            &(&inputs(touch_barrier)).into(),
            touch_barrier,
            touch_rebate,
            RebateTiming::AtHit,
        );
        assert!(
            is_close(priced_ot.greeks.price, ot_ref, 1e-12, 1e-12),
            "one-touch SDK price {} != celnet-exotics reference {ot_ref}",
            priced_ot.greeks.price
        );

        // Structural sanity: an up-and-out call is worth strictly less than a plain
        // vanilla call of the same strike (the knock-out can only extinguish value).
        let vanilla_ref = celnet_vanilla::price(OptionType::Call, &inputs(strike));
        assert!(
            sb_ref < vanilla_ref,
            "up-and-out call {sb_ref} must be cheaper than the vanilla {vanilla_ref}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
