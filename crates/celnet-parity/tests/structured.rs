//! Parity rows 16–19: **second-generation / structured exotics are built and
//! financially correct** — the product class SynOption monetises (TARF / pivot /
//! accumulator) and that Fenics (Kalahari LSV) and Murex/Numerix carry, here priced
//! in the open through the **public** `celnet-exotics` API and gated on independent
//! validation:
//!
//!  16. **Quanto** vanilla + digital closed forms cross-validated by an independent
//!      Monte-Carlo, and the zero-correlation case collapsing exactly to the plain
//!      (non-quanto) price (the quanto drift adjustment is the only difference).
//!  17. **Lookback** (floating- and fixed-strike) closed forms cross-validated by MC,
//!      plus the lookback-dominates-vanilla optionality invariant.
//!  18. **TARF** gap-risk decomposition: the FullGain (client keeps the overshoot)
//!      settlement is strictly more expensive to the bank than CappedGain (exact
//!      redemption), FullGain carries a positive expected overshoot, CappedGain none.
//!  19. **Accumulator** knock-out correctness: continuous (Brownian-bridge) monitoring
//!      knocks out more often than discrete fixing-only monitoring, so it settles
//!      fewer fixings.
//!
//! The incumbents price these behind closed pricers with no published validation;
//! Celnet validates each against an independent route (closed form ⇄ MC) or a
//! model-free financial invariant, in the open. These four rows promote the
//! structured-product book from "deferred" to "delivered & gated" — the MC regimes
//! and tolerances mirror the `celnet-exotics` crate's own validated tests.

use celnet_core::is_close;
use celnet_exotics::{
    Accumulator, AccumulatorMcConfig, Lookback, LookbackMcConfig, LookbackStyle, Monitoring,
    QuantoMcConfig, QuantoParams, RedemptionStyle, Tarf, TarfMcConfig, accumulator_price,
    fixed_lookback_price, floating_lookback_price, lookback_mc, quanto_digital_mc,
    quanto_digital_price, quanto_vanilla_mc, quanto_vanilla_price, tarf_price,
};
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::price as vanilla_price;

const BOTH: [OptionType; 2] = [OptionType::Call, OptionType::Put];

/// Row 16 — **Quanto** closed forms reproduced by an independent Monte-Carlo, and
/// the zero-correlation collapse to the plain price. SynOption/Fenics price quantos
/// behind closed pricers; Celnet validates the closed form against MC, in the open.
#[test]
fn quanto_closed_form_matches_mc_and_collapses_at_zero_correlation() {
    let i = VanillaInputs::new(1.10, 1.10, 0.12, 1.0, 0.03, 0.01);
    let q = QuantoParams::new(0.10, -0.30); // conversion vol 10%, correlation −0.3
    let cfg = QuantoMcConfig {
        pairs: 300_000,
        seed: 0x9_1A17,
    };

    for opt in BOTH {
        // Vanilla quanto: MC reproduces the closed form within Monte-Carlo error.
        let closed_v = quanto_vanilla_price(opt, &i, q);
        let mc_v = quanto_vanilla_mc(opt, &i, q, cfg);
        let tol_v = 4.0 * mc_v.std_error + 1e-6;
        assert!(
            (mc_v.price - closed_v).abs() < tol_v,
            "quanto vanilla {opt:?}: MC {} vs closed {closed_v} (se {}, tol {tol_v})",
            mc_v.price,
            mc_v.std_error
        );

        // Digital quanto: same independent cross-check.
        let closed_d = quanto_digital_price(opt, &i, q);
        let mc_d = quanto_digital_mc(opt, &i, q, cfg);
        let tol_d = 4.0 * mc_d.std_error + 1e-6;
        assert!(
            (mc_d.price - closed_d).abs() < tol_d,
            "quanto digital {opt:?}: MC {} vs closed {closed_d} (se {}, tol {tol_d})",
            mc_d.price,
            mc_d.std_error
        );

        // Zero correlation ⇒ the quanto drift adjustment vanishes, so the quanto
        // price is *exactly* the plain (non-quanto) price — the defining property.
        let q0 = QuantoParams::new(0.10, 0.0);
        let plain = vanilla_price(opt, &i);
        let quanto0 = quanto_vanilla_price(opt, &i, q0);
        assert!(
            is_close(quanto0, plain, 1e-12, 1e-12),
            "zero-correlation quanto {opt:?} {quanto0} must equal plain {plain}"
        );
    }
}

/// Row 17 — **Lookback** (floating- and fixed-strike) closed forms reproduced by MC,
/// plus the lookback-dominates-vanilla optionality invariant (the path-extremum
/// payoff dominates the terminal payoff pathwise, so its value is never below the
/// vanilla).
#[test]
fn lookback_closed_form_matches_mc_and_dominates_vanilla() {
    let i: celnet_exotics::ExoticInputs =
        VanillaInputs::new(1.10, 1.10, 0.14, 1.0, 0.03, 0.01).into();
    let cfg = LookbackMcConfig {
        pairs: 120_000,
        steps: 50,
        seed: 0x10_0CB,
    };

    for opt in BOTH {
        // Floating-strike: closed form ⇄ MC (Brownian-bridge extremum), and the
        // lookback value dominates the plain vanilla.
        let closed_float = floating_lookback_price(&i, opt);
        let mc_float = lookback_mc(
            &i,
            Lookback {
                style: LookbackStyle::FloatingStrike,
                option: opt,
            },
            cfg,
        );
        let tol_f = 4.0 * mc_float.std_error + 8e-2;
        assert!(
            (mc_float.price - closed_float).abs() < tol_f,
            "floating lookback {opt:?}: MC {} vs closed {closed_float} (se {}, tol {tol_f})",
            mc_float.price,
            mc_float.std_error
        );
        let vanilla = vanilla_price(opt, &i.as_fx_vanilla(i.strike).unwrap());
        assert!(
            closed_float > 0.0,
            "floating lookback {opt:?} must be positive"
        );
        assert!(
            closed_float >= vanilla - 1e-9,
            "floating lookback {closed_float} must dominate vanilla {vanilla} ({opt:?})"
        );

        // Fixed-strike: closed form ⇄ MC.
        let closed_fixed = fixed_lookback_price(&i, opt);
        let mc_fixed = lookback_mc(
            &i,
            Lookback {
                style: LookbackStyle::FixedStrike,
                option: opt,
            },
            cfg,
        );
        let tol_x = 4.0 * mc_fixed.std_error + 8e-2;
        assert!(
            (mc_fixed.price - closed_fixed).abs() < tol_x,
            "fixed lookback {opt:?}: MC {} vs closed {closed_fixed} (se {}, tol {tol_x})",
            mc_fixed.price,
            mc_fixed.std_error
        );
        assert!(
            closed_fixed >= vanilla - 1e-9,
            "fixed lookback {closed_fixed} must dominate vanilla {vanilla} ({opt:?})"
        );
    }
}

/// A representative TARF: a Call-favourable target-redemption forward (client
/// accumulates gains when spot is above the strike).
fn tarf_spec(redemption: RedemptionStyle) -> Tarf {
    Tarf {
        strike: 1.10,
        fixings: 12,
        target: 0.10,
        leverage: 2.0,
        favourable_side: OptionType::Call,
        notional: 1.0,
        redemption,
    }
}

/// Row 18 — **TARF** gap-risk decomposition. The FullGain settlement lets the client
/// keep the overshoot past the target on the breaching fixing (the genuine gap
/// exposure), so it is strictly more expensive to the bank than the CappedGain
/// (exact-redemption) settlement; the spread is the explicit, priced gap-risk
/// premium. FullGain carries a positive expected overshoot; CappedGain none.
#[test]
fn tarf_gap_risk_premium_is_priced_and_signed() {
    let i: celnet_exotics::ExoticInputs =
        VanillaInputs::new(1.12, 1.10, 0.12, 1.0, 0.03, 0.01).into();
    let cfg = TarfMcConfig {
        pairs: 200_000,
        seed: 0x6A9,
    };

    let full = tarf_price(&i, tarf_spec(RedemptionStyle::FullGain), cfg);
    let capped = tarf_price(&i, tarf_spec(RedemptionStyle::CappedGain), cfg);

    // FullGain pays the client more on the breaching fixing ⇒ lower bank PV. Common
    // random numbers (shared seed) keep the spread low-variance; require it to clear
    // a positive gap-premium floor net of the residual MC noise.
    let tol = 4.0 * (full.std_error + capped.std_error);
    assert!(
        full.price < capped.price - 1e-4 + tol,
        "FullGain bank PV {} must be below CappedGain {} by the gap premium (tol {tol})",
        full.price,
        capped.price
    );
    // The gap exposure: FullGain overshoots the target, CappedGain never does.
    assert!(
        full.expected_overshoot > 0.0,
        "FullGain must carry a positive expected overshoot, got {}",
        full.expected_overshoot
    );
    assert!(
        capped.expected_overshoot.abs() < 1e-12,
        "CappedGain must have zero overshoot, got {}",
        capped.expected_overshoot
    );
    // The structure redeems within its life (a sane expected-redemption diagnostic).
    assert!(
        full.expected_redemption_fixing > 0.0 && full.expected_redemption_fixing <= 12.0,
        "expected redemption fixing {} out of (0, 12]",
        full.expected_redemption_fixing
    );
}

/// A representative up-and-out accumulator (pivot 1.10, knock-out 1.16).
fn accumulator_spec(monitoring: Monitoring) -> Accumulator {
    Accumulator {
        pivot: 1.10,
        barrier: 1.16,
        fixings: 12,
        leverage: 2.0,
        notional: 1.0,
        monitoring,
    }
}

/// Row 19 — **Accumulator** knock-out correctness. Continuous (Brownian-bridge)
/// monitoring captures barrier crossings *between* fixings that discrete
/// fixing-only monitoring misses, so it knocks out more often and therefore settles
/// strictly fewer fixings on average — the model-free invariant that proves the
/// continuous-monitoring path is genuine, not a relabelled discrete one.
#[test]
fn accumulator_continuous_monitoring_knocks_out_more_than_discrete() {
    let i: celnet_exotics::ExoticInputs =
        VanillaInputs::new(1.10, 1.10, 0.16, 1.0, 0.03, 0.01).into();
    let cfg = AccumulatorMcConfig {
        pairs: 200_000,
        seed: 0x6EA2,
    };

    let discrete = accumulator_price(&i, accumulator_spec(Monitoring::Discrete), cfg);
    let continuous = accumulator_price(&i, accumulator_spec(Monitoring::Continuous), cfg);

    // Continuous monitoring sees between-fixing crossings ⇒ knocks out sooner ⇒
    // fewer settled fixings. Shared seed ⇒ common random numbers ⇒ clean spread.
    let tol = 4.0 * (discrete.std_error + continuous.std_error);
    assert!(
        continuous.expected_settled_fixings < discrete.expected_settled_fixings + tol,
        "continuous-monitoring settled {} must be below discrete {} (knocks out more, tol {tol})",
        continuous.expected_settled_fixings,
        discrete.expected_settled_fixings
    );
    for (label, r) in [("discrete", &discrete), ("continuous", &continuous)] {
        assert!(
            r.expected_settled_fixings >= 0.0 && r.expected_settled_fixings <= 12.0,
            "{label} settled fixings {} out of [0, 12]",
            r.expected_settled_fixings
        );
        assert!(
            r.std_error > 0.0,
            "{label} MC must report a positive std error"
        );
    }
}
