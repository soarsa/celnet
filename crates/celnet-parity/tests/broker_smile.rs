//! Parity rows 10–11: **broker → smile reprices the market strangle** — the
//! documented "#1 production bug" (`docs/CAPABILITIES-VS-COMPETITION.md`
//! §"Volatility surface"). The naive desk uses the arithmetic smile-butterfly
//! `BF = ½(σc+σp) − σ_ATM` directly; the *correct* construction reprices the
//! **broker (market) strangle** — a single vol `σ_ATM + BF_broker` on both wing
//! strikes. Building the smile off the arithmetic butterfly mismarks the wings,
//! worst in high-risk-reversal emerging-market crosses where SynOption / Fenics /
//! OVML are opaque about which convexity they use.
//!
//! The non-vacuous check is twofold: (1) evaluate the *calibrated* smile **at
//! the broker strangle strikes** (which are *not* smile pillars) and require the
//! two vanillas, priced at the smile's interpolated vols there, to sum back to
//! the broker strangle price; and (2) build the **naive arithmetic-butterfly
//! smile** (the production bug: σ_ATM ± ½RR + BF with no calibration) and show it
//! *mismarks* the same broker strangle by far more than the calibrated residual.
//! Asserting the naive misprice **exceeds a threshold** while the calibrated one
//! reprices within tolerance proves the calibration is a real correction, not a
//! tautology. We prove it on a benign G10 slice and on a high-RR EM slice.

use celnet_conventions::resolve;
use celnet_core::{Smile, is_close};
use celnet_surface::{MarketContext, MarketHedgeSmile, MarketQuotes, build_smile, market_strangle};
use celnet_types::{CcyPair, OptionType, Tenor};
use celnet_vanilla::price;

/// Build a per-slice context from a pair / tenor / market state.
fn ctx(pair: &str, tenor: Tenor, spot: f64, r_dom: f64, r_for: f64, t: f64) -> MarketContext {
    let conv = resolve(CcyPair::parse(pair).unwrap(), tenor).record;
    MarketContext::new(spot, r_dom, r_for, t, conv)
}

/// Core assertion: the constructed smile, evaluated at the broker strangle
/// strikes, reprices the broker strangle to tolerance. Returns the number of
/// repriced legs gated (2).
fn assert_reprices_broker_strangle(c: &MarketContext, q: &MarketQuotes) -> usize {
    let smile = build_smile(c, q).expect("smile builds from broker quotes");
    let f = c.forward();

    // The broker (market) strangle: one vol on both wings.
    let ms = market_strangle(c, q.atm_vol, q.inner).expect("market strangle builds");

    // Reprice it through the *smile's own* interpolated vols at those strikes
    // (the broker strikes are not smile anchors — this exercises interpolation).
    let vol_call = smile.implied_vol(ms.call_strike, f, c.t).0;
    let vol_put = smile.implied_vol(ms.put_strike, f, c.t).0;
    let call = price(OptionType::Call, &c.template(ms.call_strike, vol_call));
    let put = price(OptionType::Put, &c.template(ms.put_strike, vol_put));
    let repriced = call + put;

    let calibrated_err = (repriced - ms.price).abs();
    assert!(
        is_close(repriced, ms.price, 1e-8, 1e-10),
        "smile @ broker strikes reprices {repriced} but broker strangle is {} (|diff|={calibrated_err})",
        ms.price,
    );

    // --- Non-triviality: the naive arithmetic-butterfly smile MISPRICES ---
    //
    // The "#1 production bug" is using the quoted butterfly as the *smile*
    // strangle directly (no calibration): wing vols σ_ATM ± ½RR + BF placed at
    // the strikes those vols imply, with NO correction so the resulting smile
    // reprices the broker strangle. We build exactly that naive smile and show it
    // mismarks the broker strangle by FAR more than the calibrated smile's
    // residual — proving the calibration is a real, non-trivial correction, not a
    // tautology. Only meaningful when the butterfly is non-degenerate.
    if q.inner.butterfly.abs() > 1e-4 {
        let rr = q.inner.risk_reversal;
        let bf = q.inner.butterfly; // arithmetic butterfly used as σ_ss (the bug)
        let pillar = q.inner.pillar;
        // Naive wing vols: arithmetic σ_ATM ± ½RR + BF (no calibrated correction).
        let naive_call_vol = q.atm_vol + bf + 0.5 * rr;
        let naive_put_vol = q.atm_vol + bf - 0.5 * rr;
        let naive_call_k = c
            .strike_at_delta(OptionType::Call, pillar, naive_call_vol)
            .expect("naive call strike");
        let naive_put_k = c
            .strike_at_delta(OptionType::Put, pillar, naive_put_vol)
            .expect("naive put strike");
        let k_atm = c.atm_strike(q.atm_vol);

        // Build the naive vanna-volga smile through the arithmetic pillars.
        let naive_smile = MarketHedgeSmile::try_new(
            [naive_put_k, k_atm, naive_call_k],
            [naive_put_vol, q.atm_vol, naive_call_vol],
            f,
            c.t,
        )
        .expect("naive arithmetic smile builds");

        // Reprice the broker strangle through the NAIVE smile's interpolated vols
        // at the broker strikes (the same operation the calibrated smile passes).
        let nv_call = naive_smile.implied_vol(ms.call_strike, f, c.t).0;
        let nv_put = naive_smile.implied_vol(ms.put_strike, f, c.t).0;
        let naive_repriced = price(OptionType::Call, &c.template(ms.call_strike, nv_call))
            + price(OptionType::Put, &c.template(ms.put_strike, nv_put));
        let naive_err = (naive_repriced - ms.price).abs();

        // Threshold: the naive misprice must EXCEED a desk-meaningful amount AND
        // dwarf the calibrated residual. Scale the threshold by the strangle
        // price so it is dimensionless across G10 and EM regimes.
        let rel_naive = naive_err / ms.price;
        assert!(
            rel_naive > 1e-6,
            "naive arithmetic-butterfly smile should MISPRICE the broker strangle \
             but reprices it too well: naive |diff|={naive_err} (rel {rel_naive}), \
             broker strangle {}",
            ms.price
        );
        assert!(
            naive_err > 50.0 * calibrated_err.max(f64::MIN_POSITIVE),
            "calibration must be non-trivial: naive misprice {naive_err} must dwarf \
             the calibrated residual {calibrated_err} (broker strangle {})",
            ms.price
        );

        // And the calibrated smile still reprices the SAME broker strangle to
        // tolerance (the contrast is the whole point).
        assert!(
            is_close(repriced, ms.price, 1e-8, 1e-10),
            "calibrated smile must reprice within tolerance: |diff|={calibrated_err}"
        );
    }
    2
}

/// Row 10 — a benign G10 (EURUSD-style) 1Y slice: the smile reprices the broker
/// strangle exactly. Mild RR, small BF — the case every vendor *should* get
/// right, gated so a regression in the calibration surfaces immediately.
#[test]
fn smile_reprices_broker_strangle() {
    let mut rows = 0usize;
    // Sweep several benign G10 slices (different ATM/RR/BF and tenors).
    let cases = [
        (
            ctx("EURUSD", Tenor::Years(1), 1.10, 0.02, 0.01, 1.0),
            MarketQuotes::three_point(0.105, 0.015, 0.0035),
        ),
        (
            ctx("EURUSD", Tenor::Months(6), 1.10, 0.02, 0.01, 0.5),
            MarketQuotes::five_point(0.10, 0.010, 0.0030, 0.018, 0.0090),
        ),
        (
            ctx("GBPUSD", Tenor::Years(1), 1.27, 0.04, 0.045, 1.0),
            MarketQuotes::three_point(0.085, -0.012, 0.0025),
        ),
    ];
    for (c, q) in &cases {
        rows += assert_reprices_broker_strangle(c, q);
    }
    assert!(rows >= 6, "benign broker-reprice rows under-gated: {rows}");
}

/// Row 11 — a **high-risk-reversal EM** slice (USDTRY-style: high ATM vol, large
/// negative RR, fat BF). This is precisely where the arithmetic-butterfly
/// shortcut mismarks the wings and where vendor opacity bites; Celnet's
/// broker-strangle calibration reprices it exactly. Gating this is the literal
/// "#1 production bug" regression test.
#[test]
fn high_rr_em_case_reprices() {
    let mut rows = 0usize;
    // USDTRY-like 3M: ~25 ATM vol, strong skew (RR ≈ 4 vol), fat butterfly.
    // (Falls back to default conventions for an unmapped EM pair.)
    let c = ctx("USDTRY", Tenor::Months(3), 32.0, 0.45, 0.05, 0.25);
    let q = MarketQuotes::five_point(0.25, 0.040, 0.012, 0.070, 0.030);
    rows += assert_reprices_broker_strangle(&c, &q);

    // USDBRL-like 6M: high vol, large RR.
    let c2 = ctx("USDBRL", Tenor::Months(6), 5.0, 0.10, 0.05, 0.5);
    let q2 = MarketQuotes::three_point(0.18, 0.030, 0.008);
    rows += assert_reprices_broker_strangle(&c2, &q2);

    assert!(rows >= 4, "EM broker-reprice rows under-gated: {rows}");
}
