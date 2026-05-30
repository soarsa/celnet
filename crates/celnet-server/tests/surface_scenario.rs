//! Surface + scenario integration tests over the `celnet-proto` `SurfaceService`:
//! `MarkSurface` calibrates an arbitrage-checked smile, `GetSmile` reads one, and
//! `Scenario` reprices an instrument across a spot/vol shock grid correctly.

mod common;

use std::time::Duration;

use celnet_core::is_close;
use celnet_proto::surface_service_client::SurfaceServiceClient;
use celnet_proto::{
    BrokerQuoteSet, GetSmileRequest, MarkSurfaceRequest, ScenarioRequest, ShockAxis, shock_axis,
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
