//! Trader-workflow integration tests for the Phase-1 SDK capabilities: smile-model
//! selection on a surface mark, the multiplexed market-series (TrendMode) feed, and
//! the who's-trading attribution chain on the RFQ and RFS lifecycles.
//!
//! Each drives a real in-process `celnet-server` edge through the typed
//! [`celnet_client`] SDK over the SAME contract the GUI uses, asserting the new
//! calls round-trip end-to-end against the live edge — never a mock. Every body is
//! hard wall-clock bounded and every network/stream await is bounded, so a
//! regression fails fast, never hangs.

mod common;

use std::time::Duration;

use celnet_client::{
    Attribution, BookId, BrokerQuoteSet, Calibration, Observable, Seat, SeriesEvent, Side,
    StreamEvent,
};
use celnet_server::Clock;
use celnet_types::Tenor;

use celnet_client::Client;
use common::{
    STEP_DEADLINE, TEST_DEADLINE, conventions, eurusd, start_edge_and_client, start_ready_edge,
    vanilla_call,
};

/// A market-hedge mark and a stochastic-vol mark of the *same* broker quotes both
/// reprice the ATM pillar but materially change the wings, and each carries its
/// model in the smile's arbitrage note (the contract's provenance channel). Both
/// stamp a fresh pinnable surface version.
#[tokio::test]
async fn smile_model_selection_marks_and_tags_the_model() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        // A skewed five-point broker set so the wings genuinely differ between models.
        let broker = BrokerQuoteSet::five_point(1.0, 0.105, -0.0060, 0.0025, -0.0110, 0.0080);

        // Default mark (market-hedge) via the existing call.
        let hedge = tokio::time::timeout(
            STEP_DEADLINE,
            client.mark_surface(eurusd(), &[broker], conventions()),
        )
        .await
        .expect("mark in time")
        .expect("market-hedge mark succeeds");

        // Explicit stochastic-vol mark of the SAME quotes via the new selector.
        let sv = tokio::time::timeout(
            STEP_DEADLINE,
            client.mark_surface_with(
                eurusd(),
                &[broker],
                conventions(),
                Calibration::StochasticVol,
            ),
        )
        .await
        .expect("mark in time")
        .expect("stochastic-vol mark succeeds");

        // Each mark stamps its own pinnable version; a re-mark advances it.
        assert!(hedge.surface_version >= 1, "market-hedge stamps a version");
        assert!(
            sv.surface_version > hedge.surface_version,
            "the second mark advances the surface version: {} > {}",
            sv.surface_version,
            hedge.surface_version
        );

        let hedge_smile = &hedge.smiles[0];
        let sv_smile = &sv.smiles[0];

        // Both reprice the ATM (50Δ) pillar to the marked ATM vol.
        let hedge_atm = hedge_smile.atm_vol().expect("hedge ATM pillar");
        let sv_atm = sv_smile.atm_vol().expect("sv ATM pillar");
        assert!(
            celnet_core::is_close(hedge_atm, sv_atm, 5e-3, 5e-3),
            "both models pin the ATM pillar: {hedge_atm} vs {sv_atm}"
        );

        // The model materially changes a wing (the fit is not the same curve).
        let hedge_put = hedge_smile.vol_at_delta(-0.10).expect("hedge 10Δ put");
        let sv_put = sv_smile.vol_at_delta(-0.10).expect("sv 10Δ put");
        assert!(
            (hedge_put - sv_put).abs() > 1e-6,
            "model selection changes the 10Δ put wing: hedge {hedge_put} vs sv {sv_put}"
        );

        // The model used is carried in the arbitrage note (the provenance channel).
        assert!(
            sv_smile.arbitrage.note.contains("model="),
            "the smile note carries the model provenance, got {:?}",
            sv_smile.arbitrage.note
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Every smile-model selector round-trips through the SDK against the live edge:
/// each marks successfully and reprices the ATM pillar.
#[tokio::test]
async fn every_calibration_model_round_trips_through_the_sdk() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let broker = BrokerQuoteSet::five_point(1.0, 0.105, -0.0050, 0.0022, -0.0090, 0.0065);

        for model in [
            Calibration::MarketHedge,
            Calibration::StochasticVol,
            Calibration::Parametric,
            Calibration::ParametricSurface,
        ] {
            let marked = tokio::time::timeout(
                STEP_DEADLINE,
                client.mark_surface_with(eurusd(), &[broker], conventions(), model),
            )
            .await
            .expect("mark in time")
            .unwrap_or_else(|e| panic!("{model:?} mark succeeds: {e}"));
            let atm = marked.smiles[0]
                .atm_vol()
                .unwrap_or_else(|| panic!("{model:?} produced a 50Δ pillar"));
            assert!(
                celnet_core::is_close(atm, broker.atm_vol, 1e-2, 1e-2),
                "{model:?} reprices ATM {atm} ~ {}",
                broker.atm_vol
            );
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A market-series subscription (ATM vol) over the multiplexed session yields a
/// real opening snapshot then ≥1 live appended point, strictly sequenced, each a
/// finite observed vol — the server's live observation, not a fabricated value. A
/// spot series multiplexed on the same session proves multiple series coexist.
#[tokio::test]
async fn market_series_emits_real_observed_points_over_one_session() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge(Clock::system()).await;
        let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
            .await
            .expect("connects in time")
            .expect("connects");

        let session = tokio::time::timeout(STEP_DEADLINE, client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");

        // An ATM-vol trend tile and a spot trend tile over ONE connection.
        let mut atm = session
            .subscribe_series(eurusd(), Observable::AtmVol, Some(Tenor::Years(1)), 0, 16)
            .await
            .expect("atm-vol series subscribes");
        let mut spot = session
            .subscribe_series(eurusd(), Observable::Spot, None, 0, 16)
            .await
            .expect("spot series subscribes");
        assert_ne!(atm.id(), spot.id(), "distinct series ids on one session");
        assert_eq!(atm.observable(), Observable::AtmVol);

        // The opening snapshot seeds the series with the observable identity and a
        // baseline history.
        let mut last_seq = match next_series(&mut atm).await {
            SeriesEvent::Snapshot {
                observable,
                points,
                sequence,
                ..
            } => {
                assert!(
                    matches!(observable, Observable::AtmVol),
                    "snapshot echoes the observable"
                );
                assert!(!points.is_empty(), "snapshot seeds at least one point");
                let last = points.last().expect("a seeded point");
                assert!(last.value.is_finite() && last.value > 0.0, "a real vol");
                sequence.max(last.sequence)
            }
            other => panic!("expected a series Snapshot first, got {other:?}"),
        };

        // At least one live appended point, strictly sequenced, each a real vol.
        let mut seen = 0;
        while seen < 2 {
            match next_series(&mut atm).await {
                SeriesEvent::Point(p) => {
                    assert!(
                        p.sequence > last_seq,
                        "series points are strictly sequenced: {} > {last_seq}",
                        p.sequence
                    );
                    assert!(p.value.is_finite() && p.value > 0.0, "a real observed vol");
                    last_seq = p.sequence;
                    seen += 1;
                }
                SeriesEvent::Snapshot { .. } => panic!("only one opening snapshot per series"),
            }
        }

        // The spot series also seeds and emits a real spot rate.
        match next_series(&mut spot).await {
            SeriesEvent::Snapshot {
                observable, points, ..
            } => {
                assert!(matches!(observable, Observable::Spot));
                let last = points.last().expect("a seeded spot point");
                assert!(
                    celnet_core::is_close(last.value, common::FIXTURE_SPOT, 0.1, 0.1),
                    "the seeded spot {} is near the fixture spot {}",
                    last.value,
                    common::FIXTURE_SPOT
                );
            }
            other => panic!("expected a spot Snapshot, got {other:?}"),
        }

        // A clean unsubscribe tears the series down server-side.
        atm.unsubscribe().await.expect("unsubscribe sends");

        drop(atm);
        drop(spot);
        drop(session);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A risk-reversal wing series requires its delta wing (carried on the
/// [`Observable`]); it subscribes and emits a real signed RR vol.
#[tokio::test]
async fn wing_observable_series_carries_its_delta() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let session = tokio::time::timeout(STEP_DEADLINE, client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");

        let mut rr = session
            .subscribe_series(
                eurusd(),
                Observable::RiskReversal { delta: 0.25 },
                Some(Tenor::Years(1)),
                0,
                8,
            )
            .await
            .expect("rr series subscribes");
        assert_eq!(rr.observable(), Observable::RiskReversal { delta: 0.25 });

        match next_series(&mut rr).await {
            SeriesEvent::Snapshot { points, .. } => {
                let last = points.last().expect("a seeded rr point");
                assert!(last.value.is_finite(), "a real RR vol");
            }
            other => panic!("expected an rr Snapshot, got {other:?}"),
        }

        drop(rr);
        drop(session);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An RFQ sent on behalf of a requesting trading seat carries the who's-trading
/// attribution chain end to end: the resolved `Quote.attribution` names the maker
/// that quoted the line, and a booked `Execution.attribution` carries the chain so
/// the trade feeds the risk roll-up. API-first parity with the GUI/Excel contract.
#[tokio::test]
async fn rfq_carries_attribution_to_quote_and_execution() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let requesting = Attribution::quoted_by(BookId::new("EM-VOL-1", Seat::trader("alice")));
        let rfq = client
            .request_quote(vanilla_call(1.12), conventions())
            .with_attribution(requesting);

        let quote = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("quote in time")
            .expect("quote succeeds");

        // The server resolved an attribution chain naming the maker that quoted.
        let attr = quote
            .attribution
            .as_ref()
            .expect("the quote carries a resolved attribution chain");
        match &attr.quoted_by.owner {
            Seat::AutoPricer(id) => assert!(!id.is_empty(), "the maker auto-pricer is named"),
            Seat::Trader(id) => assert!(!id.is_empty(), "the quoting seat is named"),
        }

        // Accept and confirm the booked execution carries the chain too.
        let exec = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&quote, Side::Buy))
            .await
            .expect("accept in time")
            .expect("accept books");
        assert_eq!(exec.quote_id, quote.quote_id);
        assert!(
            exec.attribution.is_some(),
            "the booked execution carries the attribution chain"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An attributed RFS subscription echoes the who's-trading chain on its snapshot, so
/// a blotter's attribution column has its identity (API-first parity with the RFQ
/// path).
#[tokio::test]
async fn rfs_subscription_echoes_attribution_on_snapshot() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let session = tokio::time::timeout(STEP_DEADLINE, client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");

        let requesting =
            Attribution::quoted_by(BookId::new("G10-VOL", Seat::auto_pricer("celnet-strat-7")));
        let mut sub = session
            .subscribe_attributed(
                vanilla_call(1.12),
                conventions(),
                Some(77),
                None,
                requesting,
            )
            .await
            .expect("attributed subscribe opens");

        match next_stream(&mut sub).await {
            StreamEvent::Snapshot {
                correlation_id,
                attribution,
                ..
            } => {
                assert_eq!(
                    correlation_id,
                    Some(77),
                    "snapshot echoes the correlation id"
                );
                let attr = attribution.expect("the snapshot carries the attribution chain");
                match &attr.quoted_by.owner {
                    Seat::AutoPricer(id) => assert!(!id.is_empty(), "the maker is named"),
                    Seat::Trader(id) => assert!(!id.is_empty(), "the quoting seat is named"),
                }
            }
            other => panic!("expected an attributed Snapshot, got {other:?}"),
        }

        drop(sub);
        drop(session);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

// ---- helpers --------------------------------------------------------------

/// Await the next market-series event within the step deadline.
async fn next_series(s: &mut celnet_client::MarketSeries) -> SeriesEvent {
    tokio::time::timeout(STEP_DEADLINE, s.next_event())
        .await
        .expect("a series event arrives before the deadline")
        .expect("the series stays open")
        .expect("the event is well-formed")
}

/// Await the next stream event within the step deadline.
async fn next_stream(sub: &mut celnet_client::Subscription) -> StreamEvent {
    tokio::time::timeout(STEP_DEADLINE, sub.next_event())
        .await
        .expect("a stream event arrives before the deadline")
        .expect("the stream stays open")
        .expect("the event is well-formed")
}
