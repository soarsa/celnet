//! Surface + scenario integration tests over the `celnet-proto` `SurfaceService`:
//! `MarkSurface` calibrates an arbitrage-checked smile, `GetSmile` reads one, and
//! `Scenario` reprices an instrument across a spot/vol shock grid correctly.

mod common;

use std::time::Duration;

use celnet_core::is_close;
use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::surface_service_client::SurfaceServiceClient;
use celnet_proto::{
    BrokerQuoteSet, CrossGamma, GetSmileRequest, MarkSurfaceRequest, QuoteRequest,
    RiskBucketRequest, ScenarioRequest, ShockAxis, VegaBucket, shock_axis,
};
use celnet_types::{OptionType, VanillaInputs};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, eurusd_pair, live_market, start_ready_edge, vanilla_call,
    wire_conventions,
};

/// A scenario shock grid must reprice the instrument at each node exactly as a
/// direct `celnet-vanilla` price at the shocked market context.
#[tokio::test]
async fn scenario_shock_reprices_correctly() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            SurfaceServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let base = live_market();
        let spot_steps = vec![-0.10, 0.0, 0.10];
        let vol_steps = vec![0.0, 0.50];
        let req = ScenarioRequest {
            instrument: Some(vanilla_call(1.12)),
            base_market: Some(base),
            conventions: Some(wire_conventions()),
            axes: vec![
                ShockAxis {
                    factor: shock_axis::Factor::Spot as i32,
                    relative: true,
                    steps: spot_steps.clone(),
                },
                ShockAxis {
                    factor: shock_axis::Factor::Vol as i32,
                    relative: true,
                    steps: vol_steps.clone(),
                },
            ],
            expiry_years: 1.0,
            risk_buckets: None,
        };
        let resp = tokio::time::timeout(STEP_DEADLINE, client.scenario(req))
            .await
            .expect("scenario returns in time")
            .expect("scenario succeeds")
            .into_inner();

        assert_eq!(
            resp.points.len(),
            spot_steps.len() * vol_steps.len(),
            "the grid is the Cartesian product of the axes"
        );

        // Every node must reprice exactly against a direct GK price at the node's
        // shocked spot/vol.
        for point in &resp.points {
            let shocked = point
                .shocked_market
                .expect("node carries its shocked market");
            let greeks = point.greeks.expect("node carries greeks");
            let direct = celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(
                    shocked.spot,
                    1.12,
                    shocked.vol,
                    1.0,
                    shocked.r_dom,
                    shocked.r_for,
                ),
            );
            assert!(
                is_close(greeks.price, direct, 1e-10, 1e-10),
                "node price {} != direct {direct} at spot {} vol {}",
                greeks.price,
                shocked.spot,
                shocked.vol
            );
        }

        // The unshocked (0,0) node must equal the base price.
        let base_node = resp
            .points
            .iter()
            .find(|p| p.applied_shocks == vec![0.0, 0.0])
            .expect("the (0,0) node exists");
        let base_direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(base.spot, 1.12, base.vol, 1.0, base.r_dom, base.r_for),
        );
        assert!(is_close(
            base_node.greeks.unwrap().price,
            base_direct,
            1e-12,
            1e-12
        ));

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// `MarkSurface` calibrates a per-tenor smile from a broker quote set and returns
/// an arbitrage report; the calibrated smile reproduces the ATM vol at the 50Δ
/// pillar and stamps a surface version.
#[tokio::test]
async fn mark_surface_calibrates_arbitrage_checked_smile() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            SurfaceServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let broker = BrokerQuoteSet {
            tenor_years: 1.0,
            atm_vol: 0.105,
            rr_25: -0.0040,
            bf_25: 0.0020,
            rr_10: 0.0,
            bf_10: 0.0,
            has_ten_delta: false,
        };
        let resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.mark_surface(MarkSurfaceRequest {
                pair: Some(eurusd_pair()),
                broker_quotes: vec![broker],
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("mark_surface returns in time")
        .expect("mark_surface succeeds")
        .into_inner();

        assert!(resp.surface_version >= 1, "a surface version is stamped");
        assert_eq!(resp.smiles.len(), 1, "one smile per broker quote set");
        let smile = &resp.smiles[0];
        assert!(is_close(smile.tenor_years, 1.0, 1e-12, 1e-12));
        assert_eq!(smile.points.len(), 5, "delta-axis pillars reported");
        let arb = smile.arbitrage.as_ref().expect("arbitrage report present");
        assert!(
            arb.butterfly_arbitrage_free,
            "a mild EURUSD smile is butterfly-arbitrage-free"
        );

        // The 50Δ (ATM) pillar reproduces the marked ATM vol.
        let atm_point = smile
            .points
            .iter()
            .find(|p| (p.delta - 0.50).abs() < 1e-9)
            .expect("a 50Δ pillar exists");
        assert!(
            is_close(atm_point.vol, broker.atm_vol, 5e-3, 5e-3),
            "ATM pillar vol {} ~ marked {}",
            atm_point.vol,
            broker.atm_vol
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// `GetSmile` always returns a calibrated, arbitrage-checked delta-axis smile from
/// the maker's live market (no stub).
#[tokio::test]
async fn get_smile_returns_calibrated_slice() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            SurfaceServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let smile = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_smile(GetSmileRequest {
                pair: Some(eurusd_pair()),
                tenor_years: 1.0,
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("get_smile returns in time")
        .expect("get_smile succeeds")
        .into_inner();

        assert!(is_close(smile.tenor_years, 1.0, 1e-12, 1e-12));
        assert_eq!(smile.points.len(), 5);
        // Every reported vol is a sane positive number.
        for p in &smile.points {
            assert!(p.vol > 0.0 && p.vol < 1.0, "vol {} in range", p.vol);
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// `surface_version` pinning: a `MarkSurface` stamps a version; a subsequent
/// `RequestQuote` pinned to that version prices against the marked surface and
/// echoes the pinned version on the `Quote`. An unknown pinned version is refused.
#[tokio::test]
async fn quote_pinned_to_marked_surface_version_is_honoured_and_echoed() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut surface = tokio::time::timeout(
            STEP_DEADLINE,
            SurfaceServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("surface client connects in time")
        .expect("surface client connects");
        let mut quotes = tokio::time::timeout(
            STEP_DEADLINE,
            QuoteServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("quote client connects in time")
        .expect("quote client connects");

        // Mark a EURUSD 1Y surface → a fresh version.
        let mark = tokio::time::timeout(
            STEP_DEADLINE,
            surface.mark_surface(MarkSurfaceRequest {
                pair: Some(eurusd_pair()),
                broker_quotes: vec![BrokerQuoteSet {
                    tenor_years: 1.0,
                    atm_vol: 0.105,
                    rr_25: -0.0040,
                    bf_25: 0.0020,
                    rr_10: 0.0,
                    bf_10: 0.0,
                    has_ten_delta: false,
                }],
                conventions: Some(wire_conventions()),
            }),
        )
        .await
        .expect("mark_surface returns in time")
        .expect("mark_surface succeeds")
        .into_inner();
        let version = mark.surface_version;
        assert!(version >= 1, "a surface version is stamped");

        // A quote pinned to that version is honoured and echoes the version.
        let quote = tokio::time::timeout(
            STEP_DEADLINE,
            quotes.request_quote(QuoteRequest {
                idempotency_key: "pinned-1".to_owned(),
                instrument: Some(vanilla_call(1.12)),
                conventions: Some(wire_conventions()),
                correlation_id: Some(0xABCD),
                surface_version: Some(version),
            }),
        )
        .await
        .expect("pinned quote returns in time")
        .expect("pinned quote succeeds")
        .into_inner();
        assert_eq!(
            quote.surface_version,
            Some(version),
            "the quote echoes the pinned surface version"
        );
        assert_eq!(
            quote.correlation_id,
            Some(0xABCD),
            "the quote echoes the corr id"
        );

        // A quote pinned to an UNKNOWN version is refused (the pin cannot be honoured).
        let status = tokio::time::timeout(
            STEP_DEADLINE,
            quotes.request_quote(QuoteRequest {
                idempotency_key: "pinned-bad".to_owned(),
                instrument: Some(vanilla_call(1.12)),
                conventions: Some(wire_conventions()),
                correlation_id: None,
                surface_version: Some(version + 9_999),
            }),
        )
        .await
        .expect("bad-pin quote returns in time")
        .expect_err("an unknown pinned surface version must be refused");
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A book-shaped `Scenario` with a theta-roll axis + a `RiskBucketRequest` returns
/// per-node rolled expiries, bucketed vega for the matching tenor, a cross-gamma
/// term, and the theta roll over the requested horizons.
#[tokio::test]
async fn scenario_book_shaped_risk_theta_roll_buckets_and_cross_gamma() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            SurfaceServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let base = live_market();
        let day = 1.0 / 365.0;
        let req = ScenarioRequest {
            instrument: Some(vanilla_call(1.12)),
            base_market: Some(base),
            conventions: Some(wire_conventions()),
            axes: vec![
                ShockAxis {
                    factor: shock_axis::Factor::Spot as i32,
                    relative: true,
                    steps: vec![-0.05, 0.0, 0.05],
                },
                // The theta-roll axis: roll calendar time forward (absolute years).
                ShockAxis {
                    factor: shock_axis::Factor::Time as i32,
                    relative: false,
                    steps: vec![0.0, day, 3.0 * day],
                },
            ],
            expiry_years: 1.0,
            risk_buckets: Some(RiskBucketRequest {
                vega_pillars: vec![
                    // The matching-tenor (1Y) ATM pillar carries the structure's vega.
                    VegaBucket {
                        tenor_years: 1.0,
                        delta: 0.50,
                        vega: 0.0,
                    },
                    // A non-matching tenor pillar has no exposure (flat-vol scenario).
                    VegaBucket {
                        tenor_years: 0.25,
                        delta: 0.25,
                        vega: 0.0,
                    },
                ],
                cross_gamma_pairs: vec![CrossGamma {
                    factor_a: shock_axis::Factor::Spot as i32,
                    factor_b: shock_axis::Factor::Vol as i32,
                    value: 0.0,
                }],
                roll_horizons_years: vec![day, 3.0 * day],
            }),
        };
        let resp = tokio::time::timeout(STEP_DEADLINE, client.scenario(req))
            .await
            .expect("scenario returns in time")
            .expect("scenario succeeds")
            .into_inner();

        // The grid is the Cartesian product (3 spot × 3 time = 9 nodes).
        assert_eq!(resp.points.len(), 9);
        // Each node reports its (theta-) rolled expiry: a 3-day roll prices at
        // 1.0 - 3/365.
        let rolled = resp
            .points
            .iter()
            .find(|p| p.applied_shocks == vec![0.0, 3.0 * day])
            .expect("the (0, 3d-roll) node exists");
        assert!(
            is_close(rolled.expiry_years, 1.0 - 3.0 * day, 1e-12, 1e-12),
            "the theta-roll node prices at the rolled expiry, got {}",
            rolled.expiry_years
        );

        let risk = resp.bucketed_risk.expect("book-shaped risk present");
        // Bucketed vega: the matching 1Y pillar carries the structure's vega; the
        // non-matching 0.25Y pillar is zero (no exposure on a flat scenario vol).
        let v_1y = risk
            .vega_buckets
            .iter()
            .find(|b| is_close(b.tenor_years, 1.0, 1e-9, 1e-9))
            .expect("the 1Y vega bucket");
        assert!(
            v_1y.vega > 0.0,
            "the matching-tenor pillar carries vega {}",
            v_1y.vega
        );
        let v_3m = risk
            .vega_buckets
            .iter()
            .find(|b| is_close(b.tenor_years, 0.25, 1e-9, 1e-9))
            .expect("the 0.25Y vega bucket");
        assert!(
            is_close(v_3m.vega, 0.0, 1e-12, 1e-12),
            "a non-matching tenor pillar has no exposure, got {}",
            v_3m.vega
        );

        // Cross-gamma spot×vol is the book-level vanna aggregate: present and finite.
        assert_eq!(risk.cross_gammas.len(), 1);
        assert!(
            risk.cross_gammas[0].value.is_finite(),
            "cross-gamma is finite"
        );

        // The theta roll: two values, one per requested horizon, both finite and
        // strictly less than the unrolled value (an option decays as time passes).
        assert_eq!(risk.theta_roll.len(), 2);
        assert_eq!(risk.roll_horizons_years.len(), 2);
        let base_value = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(base.spot, 1.12, base.vol, 1.0, base.r_dom, base.r_for),
        );
        for &rolled_value in &risk.theta_roll {
            assert!(rolled_value.is_finite());
            assert!(
                rolled_value < base_value,
                "a rolled (shorter-dated) value {rolled_value} decays below the base {base_value}"
            );
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
