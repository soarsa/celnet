//! Integration tests driving the `celnet-vanilla` convention layer from the
//! *real* resolved convention records produced by `celnet-conventions`.
//!
//! This wires the two work-streams together end-to-end: for each `(pair, tenor)`
//! the registry hands back a [`celnet_conventions::ConventionRecord`] naming the
//! delta/ATM/premium conventions, and we verify that the
//! strike↔delta↔strike round-trip, the ATM strike, and the premium re-expression
//! all behave correctly *in the resolved convention* — i.e. exactly how a desk
//! would consume them. Reference geometry follows the standard FX-smile
//! literature (Clark, *Foreign Exchange Option Pricing*, 2011; Reiswich &
//! Wystup, "FX Volatility Smile Construction", 2010).

use celnet_conventions::resolve;
use celnet_core::is_close;
use celnet_types::{
    AtmConvention, CcyPair, DeltaConvention, OptionType, PremiumStyle, Tenor, VanillaInputs,
};
use celnet_vanilla::{atm_strike_from_inputs, convention_delta, premium, strike_from_delta};

fn pair(s: &str) -> CcyPair {
    CcyPair::parse(s).unwrap()
}

/// A representative input set for a pair; vol/rates are plausible market levels.
fn inputs_for(spot: f64) -> VanillaInputs {
    VanillaInputs::new(spot, spot, 0.105, 1.0, 0.025, 0.012)
}

/// For each resolved `(pair, tenor)` record, the 25Δ and 10Δ strikes invert
/// back to the requested delta in that record's *own* delta convention.
#[test]
fn resolved_convention_strike_delta_round_trip() {
    let cases = [
        ("EURUSD", 1.10),
        ("USDJPY", 150.0),
        ("GBPUSD", 1.27),
        ("AUDUSD", 0.66),
        ("USDCHF", 0.88),
        ("USDCAD", 1.36),
        ("NZDUSD", 0.61),
    ];
    let tenors = [Tenor::Months(3), Tenor::Years(1), Tenor::Years(2)];

    for (p, spot) in cases {
        let i = inputs_for(spot);
        for t in tenors {
            let rec = resolve(pair(p), t).record;
            let conv = rec.delta;
            for opt in [OptionType::Call, OptionType::Put] {
                for mag in [0.25_f64, 0.10] {
                    let target = match opt {
                        OptionType::Call => mag,
                        OptionType::Put => -mag,
                    };
                    let k = strike_from_delta(conv, opt, target, &i)
                        .unwrap_or_else(|e| panic!("{p} {t:?} {conv:?} {opt:?}: {e:?}"));
                    let mut probe = i;
                    probe.strike = k;
                    let back = convention_delta(conv, opt, &probe);
                    assert!(
                        is_close(back, target, 1e-7, 1e-9),
                        "{p} {t:?} {conv:?} {opt:?}: Δ*={target} got {back} (K={k})"
                    );
                }
            }
        }
    }
}

/// The resolved ATM strike is delta-neutral / equal to the forward as the
/// record's [`AtmConvention`] requires, with the DNS sign matching the
/// record's premium-adjustment.
#[test]
fn resolved_atm_strike_is_consistent() {
    for (p, spot) in [("EURUSD", 1.10), ("USDJPY", 150.0), ("GBPUSD", 1.27)] {
        let i = inputs_for(spot);
        let rec = resolve(pair(p), Tenor::Months(3)).record;
        let k_atm = atm_strike_from_inputs(rec.atm, rec.delta, &i);
        let f = i.forward();
        match rec.atm {
            AtmConvention::AtmForward => assert!(is_close(k_atm, f, 1e-12, 1e-12), "{p} ATMF"),
            AtmConvention::DeltaNeutralStraddle => {
                // Straddle delta neutral in the record's delta convention.
                let mut probe = i;
                probe.strike = k_atm;
                let straddle = convention_delta(rec.delta, OptionType::Call, &probe)
                    + convention_delta(rec.delta, OptionType::Put, &probe);
                assert!(is_close(straddle, 0.0, 1e-9, 1e-11), "{p} DNS not neutral");
                // Sign of the DNS strike vs forward matches premium-adjustment.
                if rec.is_delta_premium_adjusted() {
                    assert!(k_atm < f, "{p} prem-adj DNS below forward");
                } else {
                    assert!(k_atm > f, "{p} unadjusted DNS above forward");
                }
            }
        }
    }
}

/// EURUSD 3M is premium-adjusted (%FOR); its premium re-expressed in %foreign
/// equals the domestic-pips PV divided by spot — the canonical conversion.
#[test]
fn eurusd_premium_style_round_trip() {
    let rec = resolve(pair("EURUSD"), Tenor::Months(3)).record;
    assert_eq!(rec.premium_style, PremiumStyle::PercentForeign);
    let i = inputs_for(1.10);
    let pct_for = premium(rec.premium_style, OptionType::Call, &i);
    let dpips = premium(PremiumStyle::DomesticPips, OptionType::Call, &i);
    assert!(is_close(pct_for, dpips / i.spot, 1e-12, 1e-12));
}

/// USDJPY switches the delta *axis* with tenor (spot ≤1Y, forward >1Y) while
/// staying premium-adjusted; the solver must round-trip in both axes.
#[test]
fn usdjpy_term_structure_axes_round_trip() {
    let i = VanillaInputs::new(150.0, 150.0, 0.095, 1.0, 0.05, 0.001);
    let short = resolve(pair("USDJPY"), Tenor::Months(6)).record;
    let long = resolve(pair("USDJPY"), Tenor::Years(2)).record;
    assert_eq!(short.delta, DeltaConvention::SpotPremiumAdjusted);
    assert_eq!(long.delta, DeltaConvention::ForwardPremiumAdjusted);

    for rec in [short, long] {
        let k = strike_from_delta(rec.delta, OptionType::Put, -0.25, &i).unwrap();
        let mut probe = i;
        probe.strike = k;
        let back = convention_delta(rec.delta, OptionType::Put, &probe);
        assert!(is_close(back, -0.25, 1e-7, 1e-9), "{:?} axis RT", rec.delta);
    }
}
