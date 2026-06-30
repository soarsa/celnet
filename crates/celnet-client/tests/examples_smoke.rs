//! Gate for the runnable SDK quickstart examples (`examples/quote_and_trade.rs`,
//! `examples/multi_dealer_trade.rs`, `examples/stream_blotter.rs`,
//! `examples/price_exotic.rs`, `examples/price_linear.rs`,
//! `examples/price_cross_asset.rs`).
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
    AmericanTerms, AsianTerms, Conventions, FixingSource, ForwardSide, ForwardTerms,
    InstrumentSpec, MarketContext, NdfTerms, Quantity, Side, StreamEvent, StrikeSpec, Subscription,
    SwapTerms,
};
use celnet_types::{CcyPair, OptionType, Tenor};

use celnet_server::Clock;

use common::{
    conventions, eurusd, start_edge_and_authed_client, start_edge_and_client,
    start_panel_edge_and_authed_client,
};

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
        // Authenticated: this path books via the capability-gated `AcceptQuote`
        // (`Execute·FxOptions`), which needs a real session under Enforce. Own the
        // `TempDir` so parallel test edges never race the shared persisted-config path.
        let (edge, client, _data_dir) = start_edge_and_authed_client().await;

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

/// `multi_dealer_trade.rs` — fan one RFQ across the demo edge's LP panel (native
/// maker + 3 synthetic demo dealers, the `demo_edge` default), then BUY the
/// best-offer dealer's pinned line. Asserts a full ranked panel with an uncrossed
/// touch and a booking at exactly the pinned offer — the same guards the example
/// exits non-zero on.
#[tokio::test]
async fn example_multi_dealer_trade_path_ranks_and_books_best_lp() {
    tokio::time::timeout(SMOKE_TEST, async {
        // The same panel breadth `demo_edge` boots with, on an in-process edge
        // (explicit panel — no env mutation). Authenticated: the best-LP booking goes
        // through the capability-gated `AcceptQuote` (`Execute·FxOptions`). Own the
        // `TempDir` so parallel test edges never race the shared persisted-config path.
        let (edge, client, _data_dir) =
            start_panel_edge_and_authed_client(Clock::system(), 3).await;

        let instrument = InstrumentSpec::vanilla(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::TwoWay,
            OptionType::Call,
            StrikeSpec::Absolute(1.12),
        );
        let md = client.request_multi_dealer_quote(instrument, Conventions::major_default());

        let panel = tokio::time::timeout(SMOKE_STEP, md.request())
            .await
            .expect("panel in time")
            .expect("panel ok");
        // The example's runtime guards: a full ranked panel, an uncrossed touch.
        assert_eq!(panel.dealers.len(), 4, "native maker + 3 synthetic dealers");
        let best_bid = panel.best_bid().expect("a liftable bid");
        let best_offer = panel.best_offer().expect("a liftable offer");
        assert!(
            best_offer.price.offer.is_finite() && best_bid.price.bid <= best_offer.price.offer,
            "non-degenerate touch: bid {} offer {}",
            best_bid.price.bid,
            best_offer.price.offer
        );

        let lp = best_offer.lp_id.clone();
        let offer = best_offer.price.offer;
        let execution = tokio::time::timeout(SMOKE_STEP, md.accept_dealer(&panel, Side::Buy, &*lp))
            .await
            .expect("accept in time")
            .expect("accept ok");
        // The example's runtime guards: a real booking at the pinned offer.
        assert!(execution.execution_id >= 1, "a real execution booked");
        assert_eq!(
            execution.traded_premium.to_bits(),
            offer.to_bits(),
            "booked exactly the pinned panel offer"
        );

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
        // Authenticated: the stream `Subscribe` frames need `Stream·FxOptions`, a
        // capability only an authenticated session carries under Enforce. Own the
        // `TempDir` so parallel test edges never race the shared persisted-config path.
        let (edge, client, _data_dir) = start_edge_and_authed_client().await;

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
        let (edge, client, _data_dir) = start_edge_and_client().await;
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

/// `price_linear.rs` — price an FX forward, an FX swap, and an NDF via the SDK
/// linear-book builders. Asserts a fair-struck forward has PV ≈ 0 with no MC
/// std-error, the swap is finite/exact, and the NDF PV equals the hand-derived
/// equal-terms deliverable-forward PV — the same guards the example exits non-zero
/// on.
#[tokio::test]
async fn example_price_linear_path_prices_forward_swap_ndf() {
    tokio::time::timeout(SMOKE_TEST, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        let conv = Conventions::major_default();

        // Forward struck at the fair forward ⇒ PV ≈ 0, exact (no std-error).
        let fx_market = MarketContext {
            spot: 1.10,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.01,
        };
        let fair_forward = 1.10 * f64::exp(0.01);
        let at_fair = InstrumentSpec::fx_forward(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            ForwardTerms::new(fair_forward, 1_000_000.0, ForwardSide::Buy),
        );
        let priced_fair = tokio::time::timeout(SMOKE_STEP, client.price(&at_fair, fx_market, conv))
            .await
            .expect("forward price in time")
            .expect("forward price ok");
        assert!(
            priced_fair.greeks.price.is_finite() && priced_fair.greeks.price.abs() <= 1.0,
            "fair-struck forward PV ~0: {}",
            priced_fair.greeks.price
        );
        assert!(
            priced_fair.price_std_error.is_none(),
            "closed-form forward carries no MC standard error"
        );

        // FX swap: near BUY @ 1.10 + far SELL — finite, exact.
        let swap = InstrumentSpec::fx_swap(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            SwapTerms::new(ForwardTerms::new(1.10, 1_000_000.0, ForwardSide::Buy)),
        );
        let priced_swap = tokio::time::timeout(SMOKE_STEP, client.price(&swap, fx_market, conv))
            .await
            .expect("swap price in time")
            .expect("swap price ok");
        assert!(
            priced_swap.greeks.price.is_finite() && priced_swap.price_std_error.is_none(),
            "closed-form swap finite + no std-error: {}",
            priced_swap.greeks.price
        );

        // NDF on a non-deliverable pair (USDBRL): PV == equal-terms deliverable fwd.
        let usdbrl = CcyPair::parse("USDBRL").expect("USDBRL parses");
        let ndf_market = MarketContext {
            spot: 5.00,
            vol: 0.10,
            r_dom: 0.10,
            r_for: 0.05,
        };
        let ndf = InstrumentSpec::ndf(
            usdbrl,
            Tenor::Months(6),
            0.5,
            Quantity::base(1_000_000.0),
            NdfTerms::new(
                5.10,
                1_000_000.0,
                ForwardSide::Buy,
                FixingSource::BrlPtax,
                "USD",
            ),
        );
        let priced_ndf = tokio::time::timeout(SMOKE_STEP, client.price(&ndf, ndf_market, conv))
            .await
            .expect("ndf price in time")
            .expect("ndf price ok");
        let t = 0.5_f64;
        let notional = 1_000_000.0_f64;
        let fwd = ndf_market.spot * f64::exp((ndf_market.r_dom - ndf_market.r_for) * t);
        let expected_ndf = notional * f64::exp(-ndf_market.r_dom * t) * (fwd - 5.10);
        let scale = priced_ndf
            .greeks
            .price
            .abs()
            .max(expected_ndf.abs())
            .max(1.0);
        assert!(
            priced_ndf.price_std_error.is_none(),
            "closed-form NDF carries no MC standard error"
        );
        assert!(
            (priced_ndf.greeks.price - expected_ndf).abs() <= 1e-9 * scale,
            "NDF PV {} == equal-terms deliverable fwd {expected_ndf}",
            priced_ndf.greeks.price
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
