//! End-to-end SDK gate for the LSV booking-model selector.
//!
//! These tests drive a **real** in-process `celnet-server` edge over the typed
//! [`celnet_client`] SDK and prove the booking-model selector reaches the server
//! across the wire and prices on the LSV engine — `API == SDK`, the api-first
//! parity bar. They are deliberately given a generous deadline: LSV is a heavy
//! model (particle calibration + 2-D ADI PDE + a finite-difference Greek strip),
//! and pricing it honestly takes ~hundreds of ms in release / a few seconds in a
//! debug test build. The fast numeric equality gate (server route == direct
//! `celnet-exotics` LsvModel reprice to ~1e-9) lives in the server's
//! `tests/lsv_model.rs`; here we prove the *transport* and the SDK ergonomics.

mod common;

use std::time::Duration;

use celnet_client::{
    BarrierSide as SdkBarrierSide, InstrumentSpec, PricingModel, Quantity, Side, StrikeSpec,
};
use celnet_exotics::{
    AdiGrid, ImpliedVolSurface, LsvModel, ParticleConfig, VarianceParams,
    WindowBarrier as ExWindowBarrier,
};
use celnet_types::{OptionType, Tenor, VanillaInputs};

use common::{conventions, eurusd, live_market, start_edge_and_client};

/// A long ceiling — LSV pricing is genuinely heavy (calibration + PDE + Greek
/// strip), so the wire round-trip must be given room rather than flake.
const LSV_TEST_DEADLINE: Duration = Duration::from_secs(45);
const LSV_STEP_DEADLINE: Duration = Duration::from_secs(40);

// --- the LSV oracle (the engine the route calls, re-derived by hand) -------

const ORACLE_KAPPA: f64 = 2.0;
const ORACLE_XI: f64 = 0.18;
const ORACLE_RHO: f64 = -0.30;

struct OracleFlatIv {
    sigma: f64,
    spot: f64,
    carry: f64,
}
impl ImpliedVolSurface for OracleFlatIv {
    fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
        self.sigma
    }
    fn forward(&self, t: f64) -> f64 {
        self.spot * (self.carry * t).exp()
    }
}

fn oracle_model(spot: f64, vol: f64, r_dom: f64, r_for: f64) -> LsvModel {
    let v = vol * vol;
    let var = VarianceParams::new(v, ORACLE_KAPPA, v, ORACLE_XI, ORACLE_RHO);
    let carry = r_dom - r_for;
    let iv = OracleFlatIv {
        sigma: vol,
        spot,
        carry,
    };
    let n = 41usize;
    let spot_grid: Vec<f64> = (0..n)
        .map(|k| {
            let x = -0.6 + 1.2 * (k as f64) / ((n - 1) as f64);
            spot * x.exp()
        })
        .collect();
    let inputs = VanillaInputs::new(spot, spot, vol, 1.0, r_dom, r_for);
    let particle = ParticleConfig {
        particles: 30_000,
        steps: 40,
        seed: 0x0001_0CA1,
        ..ParticleConfig::default()
    };
    LsvModel::calibrate(inputs, var, &iv, &spot_grid, particle)
}

fn oracle_price_grid() -> AdiGrid {
    AdiGrid {
        x_steps: 160,
        v_steps: 48,
        time_steps: 100,
        ..AdiGrid::default()
    }
}

#[tokio::test]
async fn lsv_single_barrier_prices_through_the_wire() {
    tokio::time::timeout(LSV_TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let m = live_market();

        let strike = 1.10;
        let barrier = 0.95; // down-and-out
        // Build the barrier via the dedicated ctor, then select the LSV booking
        // model fluently with `.with_lsv()`.
        let barrier_spec = InstrumentSpec::single_barrier(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            celnet_client::BarrierTerms::new(
                OptionType::Call,
                StrikeSpec::Absolute(strike),
                celnet_client::BarrierKind::KnockOut,
                SdkBarrierSide::Down,
                barrier,
            ),
        )
        .with_lsv();
        assert_eq!(barrier_spec.pricing_model, PricingModel::LocalStochVol);

        let priced = tokio::time::timeout(
            LSV_STEP_DEADLINE,
            client.price(&barrier_spec, m, conventions()),
        )
        .await
        .expect("LSV barrier price returns in time")
        .expect("LSV barrier price succeeds");

        // The PDE-priced LSV barrier carries no MC std-error.
        assert!(priced.price_std_error.is_none());

        // Independent oracle: the same LSV engine, re-derived by hand.
        let model = oracle_model(m.spot, m.vol, m.r_dom, m.r_for);
        let oracle = model.price_barrier_pde(
            OptionType::Call,
            strike,
            barrier,
            false,
            oracle_price_grid(),
        );
        assert!(
            (priced.greeks.price - oracle).abs() < 1e-9,
            "SDK LSV barrier {} vs direct LsvModel reprice {} (diff {:e})",
            priced.greeks.price,
            oracle,
            (priced.greeks.price - oracle).abs()
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

#[tokio::test]
async fn lsv_window_barrier_prices_through_the_wire() {
    tokio::time::timeout(LSV_TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let m = live_market();

        let strike = 1.10;
        let barrier = 1.30; // up-and-out
        let spec = InstrumentSpec::window_barrier(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            OptionType::Call,
            strike,
            barrier,
            SdkBarrierSide::Up,
            0.5,
            1.0,
            0, // ADI PDE (exact, no std-error)
            0,
            0,
        );
        // The window-barrier ctor pre-selects the LSV model (it has no closed form).
        assert_eq!(spec.pricing_model, PricingModel::LocalStochVol);

        let priced = tokio::time::timeout(LSV_STEP_DEADLINE, client.price(&spec, m, conventions()))
            .await
            .expect("LSV window price returns in time")
            .expect("LSV window price succeeds");
        assert!(priced.price_std_error.is_none());

        let model = oracle_model(m.spot, m.vol, m.r_dom, m.r_for);
        let oracle = model.price_window_barrier_pde(
            ExWindowBarrier {
                option: OptionType::Call,
                strike,
                barrier,
                up: true,
                start: 0.5,
                end: 1.0,
            },
            oracle_price_grid(),
        );
        assert!(
            (priced.greeks.price - oracle).abs() < 1e-9,
            "SDK LSV window {} vs direct reprice {} (diff {:e})",
            priced.greeks.price,
            oracle,
            (priced.greeks.price - oracle).abs()
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

#[tokio::test]
async fn lsv_on_unsupported_product_is_invalid_argument() {
    tokio::time::timeout(LSV_TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let m = live_market();

        // A vanilla spec routed through LSV is supported; but selecting LSV on a
        // double-barrier (an unsupported product) must be a clear INVALID_ARGUMENT,
        // never a silent fallback.
        let unsupported = InstrumentSpec::double_barrier(
            eurusd(),
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            celnet_client::DoubleBarrierTerms::new(
                OptionType::Call,
                StrikeSpec::Absolute(1.10),
                celnet_client::BarrierKind::KnockOut,
                0.95,
                1.25,
            ),
        )
        .with_lsv();

        let err = tokio::time::timeout(
            LSV_STEP_DEADLINE,
            client.price(&unsupported, m, conventions()),
        )
        .await
        .expect("error returns in time")
        .expect_err("LSV on a double-barrier must be rejected, never a silent fallback");
        match err {
            celnet_client::ClientError::Status(s) => {
                assert_eq!(s.code(), tonic::Code::InvalidArgument);
                assert!(
                    s.message().contains("LOCAL_STOCH_VOL")
                        && s.message().contains("double_barrier"),
                    "clear unsupported-model message: {}",
                    s.message()
                );
            }
            other => panic!("expected a Status error, got {other:?}"),
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
