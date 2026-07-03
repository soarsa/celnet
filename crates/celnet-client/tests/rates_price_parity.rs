//! Live-loopback conformance for the linear-rates IRS / FRA / bond SDK price
//! methods (`price_irs` / `price_fra` / `price_bond`) — the client-side parity of
//! the landed `RatesInstrument` arms (server side: `fi-wire-instruments`).
//!
//! Each test boots a real in-process `celnet-server` edge, builds the instrument
//! with the typed SDK spec, and prices it end-to-end through the `PriceRates` wire.
//! That proves the SDK-built `RatesPriceRequest` decodes to the intended arm on the
//! server (a wrong or absent arm would fail to price), and that the priced result
//! satisfies a model-independent identity — never a re-run of the engine as its own
//! oracle. A swap or FRA priced AT its own par (fair) rate is worth exactly zero
//! (`N·(K·A − F) = 0`), so the SDK path is wired to the calibrated curve; a payer is
//! exactly the opposite of a receiver, and a long bond the opposite of a short, so
//! the SDK carries `side` faithfully.
//!
//! All quantities are server-computed and never summed client-side (api-first
//! parity). No mock; every body is hard wall-clock bounded and every network await
//! is itself bounded, so a regression fails fast, never hangs.

mod common;

use celnet_client::{BondSpec, CivilDate, FraSpec, IrsSpec, UsdSofrCurve};

use common::{STEP_DEADLINE, TEST_DEADLINE, start_edge_and_client};

/// A calibrated USD-SOFR curve (whole-year par-OIS pillars) the tests price
/// against — the same known-good market shape the server's own rates tests use
/// (reference 2026-06-25; pillars 1y/2y/5y/10y), so every arm calibrates cleanly.
fn curve() -> UsdSofrCurve {
    UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
        .pillar(1, 0.0420)
        .pillar(2, 0.0410)
        .pillar(5, 0.0405)
        .pillar(10, 0.0415)
}

/// IRS par identity — price a receiver IRS to read its par (fair fixed) rate, then
/// re-price AT that rate: the PV must vanish, independent of the pricing
/// implementation. This is the client-side witness that `price_irs` reaches the
/// same calibrated engine the server's own `irs_priced_at_par_rate_has_zero_pv`
/// proves at 1e-12; over the (bit-exact IEEE-754) wire we allow a tiny margin.
#[tokio::test]
async fn irs_priced_at_its_par_rate_is_worth_zero() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _dir) = start_edge_and_client().await;
        let curve = curve();

        let probe = step(client.price_irs(&curve, &IrsSpec::receive_fixed(5, 0.04))).await;
        assert!(
            probe.par_rate > 0.0 && probe.par_rate < 1.0,
            "sane IRS par rate: {}",
            probe.par_rate
        );

        let at_par =
            step(client.price_irs(&curve, &IrsSpec::receive_fixed(5, probe.par_rate))).await;
        assert!(
            at_par.pv.abs() <= 1e-9,
            "par IRS PV not zero: {}",
            at_par.pv
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// The IRS payer is exactly the opposite of the receiver, and par is
/// side-independent — the SDK carries `side` faithfully to the server.
#[tokio::test]
async fn irs_payer_is_the_opposite_of_the_receiver() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _dir) = start_edge_and_client().await;
        let curve = curve();
        let notional = 50_000_000.0;

        let recv =
            step(client.price_irs(&curve, &IrsSpec::receive_fixed(7, 0.041).notional(notional)))
                .await;
        let pay =
            step(client.price_irs(&curve, &IrsSpec::pay_fixed(7, 0.041).notional(notional))).await;

        assert!(
            celnet_core::is_close(pay.pv, -recv.pv, 1e-12, 1e-6),
            "payer PV {} == -receiver PV {}",
            pay.pv,
            recv.pv
        );
        assert_eq!(pay.par_rate, recv.par_rate, "par is side-independent");

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// FRA par identity — a FRA priced at its own break-even rate is worth zero (the
/// single-period swaplet vanishes), proving `price_fra` reaches the calibrated
/// curve on the FRA arm.
#[tokio::test]
async fn fra_priced_at_its_par_rate_is_worth_zero() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _dir) = start_edge_and_client().await;
        let curve = curve();

        let probe = step(client.price_fra(&curve, &FraSpec::receive_fixed(3, 6, 0.033))).await;
        assert!(
            probe.par_rate > 0.0 && probe.par_rate < 1.0,
            "sane FRA par rate: {}",
            probe.par_rate
        );

        let at_par =
            step(client.price_fra(&curve, &FraSpec::receive_fixed(3, 6, probe.par_rate))).await;
        assert!(
            at_par.pv.abs() <= 1e-9,
            "par FRA PV not zero: {}",
            at_par.pv
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// Bond conformance — a long premium bond (6% coupon well above the ~4% curve)
/// carries a positive dirty price and a sane yield to maturity, and a short is
/// exactly its opposite; the yield is side-independent. This proves `price_bond`
/// reaches the curve-discounted bond engine on the bond arm and carries `side`.
#[tokio::test]
async fn bond_long_is_the_opposite_of_short_and_prices_positive() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _dir) = start_edge_and_client().await;
        let curve = curve();
        let maturity = CivilDate::new(2035, 6, 15);

        let long = step(client.price_bond(&curve, &BondSpec::long(0.06, maturity))).await;
        let short = step(client.price_bond(&curve, &BondSpec::short(0.06, maturity))).await;

        assert!(
            long.pv > 0.0,
            "long bond dirty price should be positive: {}",
            long.pv
        );
        assert!(
            long.par_rate > 0.0 && long.par_rate < 1.0,
            "sane yield to maturity: {}",
            long.par_rate
        );
        assert!(
            celnet_core::is_close(short.pv, -long.pv, 1e-12, 1e-6),
            "short PV {} == -long PV {}",
            short.pv,
            long.pv
        );
        assert_eq!(long.par_rate, short.par_rate, "yield is side-independent");

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// Bound a single rates call so a never-arriving reply fails fast.
async fn step<T>(fut: impl std::future::Future<Output = celnet_client::ClientResult<T>>) -> T {
    tokio::time::timeout(STEP_DEADLINE, fut)
        .await
        .expect("rates call resolves in time")
        .expect("rates call succeeds")
}
