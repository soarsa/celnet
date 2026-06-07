//! Gate for the runnable SDK quickstart examples (`examples/quote_and_trade.rs`,
//! `examples/stream_blotter.rs`, `examples/price_exotic.rs`).
//!
//! The examples themselves are the canonical onboarding affordance (`cargo run -p
//! celnet-client --example <x>` against a `demo_edge`); booting two OS processes is
//! impractical inside the unit-test sandbox, so this test exercises the **same SDK
//! code paths** (the same `InstrumentSpec` builders + the same `Client` RFQ / stream
//! / price calls) against a real in-process edge and asserts each yields a real,
//! non-empty priced result — exactly the assertion each example makes at runtime via
//! its non-zero exit on a degenerate / empty response.
//!
//! Run the examples for real (out of process) with:
//! ```text
//! cargo run -p celnet-server --example demo_edge          # terminal 1 (gRPC :50551)
//! cargo run -p celnet-client --example quote_and_trade    # terminal 2
//! cargo run -p celnet-client --example stream_blotter
//! cargo run -p celnet-client --example price_exotic
//! ```
//!
//! Every body is hard wall-clock bounded so a regression fails fast, never hangs.

mod common;

use std::time::Duration;

use celnet_client::{
    AmericanTerms, AsianTerms, Conventions, InstrumentSpec, MarketContext, Quantity, Side,
    StreamEvent, StrikeSpec, Subscription,
};
use celnet_types::{OptionType, Tenor};

use common::{conventions, eurusd, start_edge_and_client};

// These smoke tests each boot a fresh REAL edge AND price compute-heavy products
// (e.g. the American PSOR free-boundary FD). Under full-suite parallel-nextest
// contention (1300+ tests, all cores saturated) those round-trips run far slower
// than uncontended, so these are GENEROUS LIVENESS guards — a real hang/regression
// still trips them, but CPU starvation does not. They are NOT performance gates:
// the §1.2 pricing latency is gated uncontended by celnet-bench `bench_gate`.
const SMOKE_STEP: std::time::Duration = std::time::Duration::from_secs(30);
const SMOKE_TEST: std::time::Duration = std::time::Duration::from_secs(90);

/// The market context the `price_exotic` example prices against (kept in lockstep
/// with the example so the gate exercises identical inputs).
fn example_market() -> MarketContext {
    MarketContext {
        spot: 1.10,
        vol: 0.105,
        r_dom: 0.02,
        r_for: 0.01,
    }
}

/// `quote_and_trade.rs` — request a vanilla two-way, then BUY (lift the offer) and
/// book. Asserts a finite priced two-way and a real booking — the same guards the
/// example exits non-zero on.
#[tokio::test]
async fn example_quote_and_trade_path_quotes_and_books() {
    tokio::time::timeout(SMOKE_TEST, async {
        let (edge, client) = start_edge_and_client().await;

        let instrument = InstrumentSpec::vanilla(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::TwoWay,
            OptionType::Call,
            StrikeSpec::Absolute(1.12),
        );
        let rfq = client.request_quote(instrument, Conventions::major_default());

        let quote = tokio::time::timeout(SMOKE_STEP, rfq.request())
            .await
            .expect("quote in time")
            .expect("quote ok");
        // The example's runtime guard: a finite, ordered priced two-way.
        assert!(
            quote.price.offer.is_finite() && quote.price.offer >= quote.price.bid,
            "non-degenerate quote: bid {} offer {}",
            quote.price.bid,
            quote.price.offer
        );

        let execution = tokio::time::timeout(SMOKE_STEP, rfq.accept(&quote, Side::Buy))
            .await
            .expect("accept in time")
            .expect("accept ok");
        // The example's runtime guard: a real booking id.
        assert!(execution.execution_id >= 1, "a real execution booked");
        assert_eq!(execution.side, Side::Buy);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// `stream_blotter.rs` — open ONE session, subscribe three vanilla calls, consume
/// each baseline snapshot then at least one strictly-advancing live tick. Asserts a
/// real, moving blotter — the same guard the example exits non-zero on.
#[tokio::test]
async fn example_stream_blotter_path_streams_moving_lines() {
    tokio::time::timeout(SMOKE_TEST, async {
        let (edge, client) = start_edge_and_client().await;

        let strikes = [1.08_f64, 1.12, 1.16];
        let session = tokio::time::timeout(SMOKE_STEP, client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");

        let mut subs: Vec<Subscription> = Vec::new();
        for (i, &k) in strikes.iter().enumerate() {
            let instrument = InstrumentSpec::vanilla(
                eurusd(),
                Tenor::Years(1),
                1.0,
                Quantity::base(1_000_000.0),
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(k),
            );
            let sub = tokio::time::timeout(
                SMOKE_STEP,
                session.subscribe(instrument, conventions(), Some(1000 + i as u64), None),
            )
            .await
            .expect("subscribe in time")
            .expect("subscribe ok");
            subs.push(sub);
        }
        assert_eq!(subs.len(), 3, "three subscriptions over one session");

        let mut total_ticks = 0_usize;
        for sub in &mut subs {
            // Baseline snapshot.
            let last_seq = match next_event(sub).await {
                StreamEvent::Snapshot { line, .. } => {
                    assert!(line.price.offer.is_finite(), "finite snapshot offer");
                    line.sequence
                }
                other => panic!("expected a Snapshot first, got {other:?}"),
            };
            // At least one strictly-advancing tick (the example's moving-blotter guard).
            let mut seen = false;
            let mut prev = last_seq;
            while !seen {
                match next_event(sub).await {
                    StreamEvent::Tick(line) => {
                        assert!(line.sequence > prev, "ticks strictly advance");
                        prev = line.sequence;
                        assert!(line.price.offer.is_finite());
                        seen = true;
                        total_ticks += 1;
                    }
                    StreamEvent::Heartbeat { .. } => {}
                    other => panic!("unexpected stream event: {other:?}"),
                }
            }
        }
        assert!(total_ticks >= 3, "every line streamed a live tick");

        drop(subs);
        drop(session);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// `price_exotic.rs` — price an Asian (closed-form) and an American (FD) via the
/// SDK vocabulary builders. Asserts the Asian price is finite-positive with no MC
/// standard error (closed form) and the American is never worth less than the
/// European of the same strike — the same guards the example exits non-zero on.
#[tokio::test]
async fn example_price_exotic_path_prices_asian_and_american() {
    tokio::time::timeout(SMOKE_TEST, async {
        let (edge, client) = start_edge_and_client().await;
        let market = example_market();
        let conv = Conventions::major_default();

        // Asian: arithmetic-average-rate call, 12 fixings, closed-form Curran.
        let asian = InstrumentSpec::asian_option(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::TwoWay,
            AsianTerms::fresh_discrete(OptionType::Call, 1.10, 12),
        );
        let priced_asian = tokio::time::timeout(SMOKE_STEP, client.price(&asian, market, conv))
            .await
            .expect("asian price in time")
            .expect("asian price ok");
        assert!(
            priced_asian.greeks.price.is_finite() && priced_asian.greeks.price > 0.0,
            "asian price finite-positive: {}",
            priced_asian.greeks.price
        );
        assert!(
            priced_asian.price_std_error.is_none(),
            "closed-form Asian carries no MC standard error"
        );

        // American put vs the European reference.
        let american = InstrumentSpec::american(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::TwoWay,
            AmericanTerms::american(OptionType::Put, 1.10),
        );
        let priced_american =
            tokio::time::timeout(SMOKE_STEP, client.price(&american, market, conv))
                .await
                .expect("american price in time")
                .expect("american price ok");

        let european = InstrumentSpec::vanilla(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::TwoWay,
            OptionType::Put,
            StrikeSpec::Absolute(1.10),
        );
        let priced_european =
            tokio::time::timeout(SMOKE_STEP, client.price(&european, market, conv))
                .await
                .expect("european price in time")
                .expect("european price ok");

        let american_px = priced_american.greeks.price;
        let european_px = priced_european.greeks.price;
        assert!(
            american_px.is_finite() && american_px >= european_px - 1e-9,
            "American put {american_px} must be >= European {european_px}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Await the next event on a subscription within the step deadline.
async fn next_event(sub: &mut Subscription) -> StreamEvent {
    tokio::time::timeout(SMOKE_STEP, sub.next_event())
        .await
        .expect("a stream event arrives before the deadline")
        .expect("the stream stays open")
        .expect("the event is well-formed")
}
