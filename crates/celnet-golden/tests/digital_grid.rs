//! Golden gate: Celnet digital (binary) pricing reproduces the frozen QuantLib
//! oracle, independently for all four flavours.
//!
//! Loads `data/digital_gk.csv` (analytic cash-or-nothing and asset-or-nothing
//! prices from QuantLib 1.42.1's `AnalyticEuropeanEngine` with `CashOrNothingPayoff`
//! / `AssetOrNothingPayoff`) and asserts that [`celnet_exotics::digital_price`]
//! matches it across the whole grid, for both settlement styles × {call, put}.
//!
//! This is the independent oracle that the in-crate digital tests (complementarity
//! and the `−∂C/∂K` finite-difference identity) cannot be: those validate the
//! *relationships* between the closed forms, but only QuantLib's independent
//! implementation pins each flavour's absolute value.
//!
//! The Celnet pricer returns the value per **one payout unit** (one unit of
//! domestic cash for cash-or-nothing, one unit of the foreign asset for
//! asset-or-nothing), so the oracle is scaled by the row's `payout` (a unit cash
//! amount / a single asset unit in this grid).
//!
//! ## Tolerance
//!
//! Both sides evaluate the same Garman-Kohlhagen closed form through different
//! normal-CDF implementations, so agreement is last-bit: a `1e-9` relative leg
//! with a `1e-11` absolute floor (for the deep-OTM rows where a digital collapses
//! toward zero) catches any real flavour or sign error while admitting only
//! genuine rounding noise. Closeness is [`celnet_core::is_close`].

use celnet_core::is_close;
use celnet_exotics::{DigitalKind, digital_price};
use celnet_golden::{DigitalRecord, DigitalSettlement, load_digital};
use celnet_types::{OptionType, VanillaInputs};

/// Map the frozen oracle digital record onto the exotics `DigitalKind`.
fn kind_of(rec: &DigitalRecord) -> DigitalKind {
    match rec.style {
        DigitalSettlement::CashOrNothing => DigitalKind::cash(rec.option_type),
        DigitalSettlement::AssetOrNothing => DigitalKind::asset(rec.option_type),
    }
}

#[test]
fn digital_prices_match_quantlib_all_flavours() {
    let records = load_digital().expect("frozen digital table loads");
    assert_eq!(
        records.len(),
        192,
        "frozen digital grid size changed unexpectedly"
    );

    const REL: f64 = 1e-9;
    const ABS: f64 = 1e-11;

    let mut worst_rel = 0.0f64;
    let mut worst_abs = 0.0f64;
    let mut asserts = 0usize;

    for rec in &records {
        let inputs = VanillaInputs::new(rec.spot, rec.strike, rec.vol, rec.t, rec.r_dom, rec.r_for);
        // celnet prices per one payout unit; the oracle row carries `payout`.
        let celnet = rec.payout * digital_price(kind_of(rec), &inputs);

        let abs_dev = (celnet - rec.price).abs();
        let scale = celnet.abs().max(rec.price.abs());
        if scale > 1e-6 {
            worst_rel = worst_rel.max(abs_dev / scale);
        }
        worst_abs = worst_abs.max(abs_dev);
        asserts += 1;

        assert!(
            is_close(celnet, rec.price, REL, ABS),
            "digital mismatch vs QuantLib: celnet={celnet} oracle={} |diff|={abs_dev} \
             (rel={REL}, abs={ABS}) at {rec:?}",
            rec.price,
        );
    }

    // Every flavour must be present (defence against a truncated oracle).
    for style in [
        DigitalSettlement::CashOrNothing,
        DigitalSettlement::AssetOrNothing,
    ] {
        for opt in [OptionType::Call, OptionType::Put] {
            let present = records
                .iter()
                .any(|r| r.style == style && r.option_type == opt);
            assert!(
                present,
                "flavour {style:?}/{opt:?} missing from digital grid"
            );
        }
    }

    println!(
        "golden digital grid: {asserts} flavour×row assertions; \
         worst relative deviation {worst_rel:.3e}, worst absolute {worst_abs:.3e}"
    );
}
