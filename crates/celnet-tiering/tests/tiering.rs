//! Oracle + property tests for `celnet-tiering`.
//!
//! Expected values are **hand-computed** from the model definition
//! (`bid = mid − h − s`, `offer = mid + h − s`) and the unit conversions, never
//! by re-running the engine as its own oracle.

use celnet_tiering::{
    FlatMarkup, Guardrails, InventorySkew, QuoteCtx, ScaledSmoothedSpread, SpreadUnit, StalePolicy,
    StrategySpec, SuppressReason, TieringConfig, TieringError, TieringStrategy, quote, smooth,
};

/// f64 arithmetic tolerance — far below any price tick; effectively exact for
/// the decimal price arithmetic under test (not a lowered numerical tolerance).
const EPS: f64 = 1e-12;

fn wide_guards() -> Guardrails {
    // Wide enough not to bind in the worked examples; a small positive floor.
    Guardrails::new(0.0, 10.0, 5.0, 0.01)
}

// ── Oracle 1: the doc's worked example ───────────────────────────────────────
// Flat ±25 price bps on mid 99.55 → bid 99.30 / offer 99.80, EXACTLY.
#[test]
fn flat_markup_worked_example() {
    let flat = FlatMarkup::new(25.0, SpreadUnit::PriceBps); // 25 price bps = 0.25 price
    let ctx = QuoteCtx::new(99.55);
    let tw = quote(&[&flat], &ctx, &wide_guards(), StalePolicy::Suppress).unwrap();

    // h = 25 · 0.01 = 0.25, s = 0 → bid = 99.55 − 0.25 = 99.30, offer = 99.80.
    assert!((tw.bid - 99.30).abs() < EPS, "bid = {}", tw.bid);
    assert!((tw.offer - 99.80).abs() < EPS, "offer = {}", tw.offer);
    // Spread is exactly 2h = 0.50.
    assert!((tw.offer - tw.bid - 0.50).abs() < EPS);
}

// ── Oracle 2: yield-bps → price-offset conversion (DV01) ─────────────────────
#[test]
fn yield_bps_conversion_via_explicit_dv01() {
    // DV01 = 0.07 price per bp; 25 yield bps → 0.07 · 25 = 1.75 price points.
    let ctx = QuoteCtx::new(100.0).with_dv01(0.07);
    let offset = SpreadUnit::YieldBps.to_price_offset(25.0, &ctx).unwrap();
    assert!((offset - 1.75).abs() < EPS, "offset = {offset}");
}

#[test]
fn yield_bps_conversion_derives_dv01_from_mod_duration() {
    // DV01 = ModDur · Price / 10000 = 7.0 · 100 / 10000 = 0.07 → 25 bps → 1.75.
    let ctx = QuoteCtx::new(100.0).with_mod_duration(7.0);
    let offset = SpreadUnit::YieldBps.to_price_offset(25.0, &ctx).unwrap();
    assert!((offset - 1.75).abs() < EPS, "offset = {offset}");
}

#[test]
fn yield_bps_without_duration_is_missing_duration() {
    let ctx = QuoteCtx::new(100.0); // no dv01, no mod_duration
    assert_eq!(
        SpreadUnit::YieldBps.to_price_offset(25.0, &ctx),
        Err(TieringError::MissingDuration)
    );
}

#[test]
fn price_bps_and_percent_conversions() {
    let ctx = QuoteCtx::new(100.0);
    // 1 price bp = 0.01 price points.
    assert!((SpreadUnit::PriceBps.to_price_offset(1.0, &ctx).unwrap() - 0.01).abs() < EPS);
    // 1 percent of mid = 100/100 = 1.0 price point.
    assert!((SpreadUnit::Percent.to_price_offset(1.0, &ctx).unwrap() - 1.0).abs() < EPS);
    // price points are the offset directly.
    assert!((SpreadUnit::PricePoints.to_price_offset(0.3, &ctx).unwrap() - 0.3).abs() < EPS);
    // Conversion is sign-preserving.
    assert!((SpreadUnit::PriceBps.to_price_offset(-25.0, &ctx).unwrap() + 0.25).abs() < EPS);
}

// ── Oracle 3: inventory-skew direction + magnitude ──────────────────────────
#[test]
fn inventory_skew_direction_and_magnitude() {
    // Base h = 0.10, κ = 0.01 price points per unit inventory, strategy cap 1.0.
    let inv = InventorySkew::new(0.10, 0.01, 1.0, SpreadUnit::PricePoints);
    let guards = wide_guards();

    // Flat (q = 0): symmetric around mid = 100 → 99.90 / 100.10, no skew.
    let flat = quote(
        &[&inv],
        &QuoteCtx::new(100.0).with_inventory(0.0),
        &guards,
        StalePolicy::Suppress,
    )
    .unwrap();
    assert!((flat.bid - 99.90).abs() < EPS);
    assert!((flat.offer - 100.10).abs() < EPS);

    // Long (q = +20): skew = clamp(0.01·20, ±1) = 0.20 → BOTH sides down 0.20.
    let long = quote(
        &[&inv],
        &QuoteCtx::new(100.0).with_inventory(20.0),
        &guards,
        StalePolicy::Suppress,
    )
    .unwrap();
    assert!((long.bid - 99.70).abs() < EPS, "bid = {}", long.bid);
    assert!((long.offer - 99.90).abs() < EPS, "offer = {}", long.offer);
    // Spread unchanged (skew is spread-invariant): still 2h = 0.20.
    assert!((long.offer - long.bid - 0.20).abs() < EPS);

    // Short (q = −20): skew = −0.20 → BOTH sides up 0.20.
    let short = quote(
        &[&inv],
        &QuoteCtx::new(100.0).with_inventory(-20.0),
        &guards,
        StalePolicy::Suppress,
    )
    .unwrap();
    assert!((short.bid - 100.10).abs() < EPS, "bid = {}", short.bid);
    assert!(
        (short.offer - 100.30).abs() < EPS,
        "offer = {}",
        short.offer
    );

    // Extreme long clamps at the strategy cap s_max = 1.0 (0.01·1000 = 10 → 1.0).
    let extreme = quote(
        &[&inv],
        &QuoteCtx::new(100.0).with_inventory(1000.0),
        &guards,
        StalePolicy::Suppress,
    )
    .unwrap();
    assert!((extreme.bid - 98.90).abs() < EPS, "bid = {}", extreme.bid);
    assert!(
        (extreme.offer - 99.10).abs() < EPS,
        "offer = {}",
        extreme.offer
    );
}

// ── Guardrail clamps ────────────────────────────────────────────────────────
#[test]
fn guardrails_clamp_half_spread_and_skew() {
    // Both strategies wildly exceed the guard caps; guards must clamp.
    let flat = FlatMarkup::new(1000.0, SpreadUnit::PricePoints); // h → wants 1000
    let inv = InventorySkew::new(0.0, 1000.0, 1.0e9, SpreadUnit::PricePoints); // s → 1e6
    let guards = Guardrails::new(0.5, 10.0, 5.0, 1.0); // h_max = 10, s_max = 5
    let ctx = QuoteCtx::new(100.0).with_inventory(1000.0);

    let tw = quote(&[&flat, &inv], &ctx, &guards, StalePolicy::Suppress).unwrap();
    // h clamped to h_max = 10 → half-width = 10.
    assert!(((tw.offer - tw.bid) / 2.0 - 10.0).abs() < EPS);
    // s clamped to s_max = 5 → midpoint = mid − s = 95.
    assert!(((tw.bid + tw.offer) / 2.0 - 95.0).abs() < EPS);
    assert!((tw.bid - 85.0).abs() < EPS);
    assert!((tw.offer - 105.0).abs() < EPS);
}

#[test]
fn min_spread_floor_widens_below_floor() {
    // Half-spread of 0.01 → 2h = 0.02, but the floor demands ≥ 0.50.
    let flat = FlatMarkup::new(1.0, SpreadUnit::PriceBps); // h wants 0.01
    let guards = Guardrails::new(0.0, 10.0, 5.0, 0.50); // spread_floor = 0.50
    let ctx = QuoteCtx::new(100.0);
    let tw = quote(&[&flat], &ctx, &guards, StalePolicy::Suppress).unwrap();
    assert!(
        (tw.offer - tw.bid - 0.50).abs() < EPS,
        "spread = {}",
        tw.offer - tw.bid
    );
    assert!(tw.bid < tw.offer);
}

// ── Stale handling ──────────────────────────────────────────────────────────
#[test]
fn stale_suppress_returns_no_quote() {
    let flat = FlatMarkup::new(25.0, SpreadUnit::PriceBps);
    let ctx = QuoteCtx::new(99.55).stale();
    let err = quote(&[&flat], &ctx, &wide_guards(), StalePolicy::Suppress).unwrap_err();
    assert_eq!(err.reason, SuppressReason::StaleInputs);
}

#[test]
fn stale_widen_to_max_quotes_at_h_max_zero_skew() {
    let flat = FlatMarkup::new(25.0, SpreadUnit::PriceBps);
    let guards = Guardrails::new(0.0, 10.0, 5.0, 0.01);
    let ctx = QuoteCtx::new(100.0).with_inventory(20.0).stale();
    let tw = quote(&[&flat], &ctx, &guards, StalePolicy::WidenToMax).unwrap();
    // Widened to h_max = 10, skew forced to 0 → symmetric around mid.
    assert!((tw.bid - 90.0).abs() < EPS);
    assert!((tw.offer - 110.0).abs() < EPS);
}

#[test]
fn non_finite_mid_is_suppressed() {
    let flat = FlatMarkup::new(25.0, SpreadUnit::PriceBps);
    let ctx = QuoteCtx::new(f64::NAN);
    let err = quote(&[&flat], &ctx, &wide_guards(), StalePolicy::Suppress).unwrap_err();
    assert_eq!(err.reason, SuppressReason::NonFiniteMid);
}

#[test]
fn yield_bps_strategy_without_duration_is_suppressed() {
    let flat = FlatMarkup::new(25.0, SpreadUnit::YieldBps); // needs DV01
    let ctx = QuoteCtx::new(100.0); // no duration data
    let err = quote(&[&flat], &ctx, &wide_guards(), StalePolicy::Suppress).unwrap_err();
    assert_eq!(err.reason, SuppressReason::MissingConversionInput);
}

#[test]
fn inconsistent_guardrails_are_suppressed() {
    let flat = FlatMarkup::new(1.0, SpreadUnit::PricePoints);
    let ctx = QuoteCtx::new(100.0);
    // h_min > h_max is invalid.
    let bad = Guardrails::new(5.0, 1.0, 5.0, 0.01);
    let err = quote(&[&flat], &ctx, &bad, StalePolicy::Suppress).unwrap_err();
    assert_eq!(err.reason, SuppressReason::InvalidConfig);
}

// ── Config: serde round-trip + end-to-end quote ─────────────────────────────
#[test]
fn config_serde_round_trip_and_quote() {
    let config = TieringConfig {
        unit: SpreadUnit::PriceBps,
        strategies: vec![
            StrategySpec::FlatMarkup { half_spread: 25.0 },
            StrategySpec::InventorySkew {
                half_spread: 0.0,
                kappa: 0.5,
                s_max: 100.0,
            },
        ],
        guardrails: Guardrails::new(0.0, 10.0, 5.0, 0.01),
        stale_policy: StalePolicy::Suppress,
    };

    let json = serde_json::to_string(&config).unwrap();
    let back: TieringConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(config, back);

    // Flat 25 price bps (0.25) + inventory skew: q = 10, κ = 0.5 → 5 price bps
    // → 0.05 price. mid = 99.55 → bid = 99.55 − 0.25 − 0.05 = 99.25.
    let ctx = QuoteCtx::new(99.55).with_inventory(10.0);
    let tw = back.quote(&ctx).unwrap();
    assert!((tw.bid - 99.25).abs() < EPS, "bid = {}", tw.bid);
    assert!((tw.offer - 99.75).abs() < EPS, "offer = {}", tw.offer);
}

// ── Scaled Smoothed Spread: the PDF's worked output-spread oracle ────────────
// c=.0002, e=.00008, d=.00004, m=.0008, f=1.2 — every value hand-derived from
// the formula Oₙ = min(m, c·(1 + f·Dₙ/e)), NOT from re-running the engine.
#[test]
fn scaled_smoothed_output_spread_worked_oracle() {
    let s = ScaledSmoothedSpread::new(0.00008, 0.00004, 0.0002, 0.0008, 1.2);

    // Dₙ ≤ d (dead-band) ⇒ Pₙ = 0 ⇒ Oₙ = c = .0002.
    // Sₙ = e ⇒ Dₙ = 0 ≤ d.
    assert!((s.output_spread(Some(0.00008)) - 0.0002).abs() < EPS);

    // Dₙ just above d: Sₙ = e + (d + tiny). Dₙ ≈ d = .00004 ⇒ Pₙ = 1.2·.00004/.00008
    // = 0.6 ⇒ Oₙ = .0002·1.6 = .00032. Use Sₙ = e + d + ε.
    let just_over: f64 = 0.00008 + 0.00004 + 1e-12;
    let d_over = (just_over - 0.00008).abs(); // ≈ .00004
    let p_over = 1.2 * d_over / 0.00008;
    let o_over = 0.0002 * (1.0 + p_over);
    assert!((s.output_spread(Some(just_over)) - o_over).abs() < EPS);
    assert!((o_over - 0.00032).abs() < 1e-9, "≈ .00032, got {o_over}");

    // Dₙ = e = .00008 (Sₙ = 2e): Pₙ = 1.2 ⇒ Oₙ = .0002·2.2 = .00044.
    assert!((s.output_spread(Some(0.00016)) - 0.00044).abs() < EPS);

    // Dₙ = .001 (Sₙ = e + .001): Pₙ = 1.2·.001/.00008 = 15 ⇒ c·16 = .0032, capped
    // at m = .0008. (The PDF prints Pₙ=30; that is its arithmetic slip — the
    // formula gives 15, and the cap makes the output .0008 either way.)
    assert!((s.output_spread(Some(0.00008 + 0.001)) - 0.0008).abs() < EPS);
}

// ── Scaled Smoothed Spread: half-spread + two-way through the pipeline ───────
#[test]
fn scaled_smoothed_two_way_symmetric_no_skew() {
    let s = ScaledSmoothedSpread::new(0.00008, 0.00004, 0.0002, 0.0008, 1.2);
    // Guards wide enough not to bind on these tiny spreads (floor below c).
    let guards = Guardrails::new(0.0, 1.0, 1.0, 1e-6);
    let mid = 1.10000;
    // Sₙ = 2e ⇒ Oₙ = .00044 ⇒ half = .00022, bid/offer = mid ∓ .00022, no skew.
    let ctx = QuoteCtx::new(mid).with_smoothed_spread(0.00016);
    let tw = quote(&[&s], &ctx, &guards, StalePolicy::Suppress).unwrap();
    assert!((tw.bid - (mid - 0.00022)).abs() < EPS, "bid {}", tw.bid);
    assert!(
        (tw.offer - (mid + 0.00022)).abs() < EPS,
        "offer {}",
        tw.offer
    );
    // Symmetric about mid (no skew): midpoint == mid.
    assert!(((tw.bid + tw.offer) / 2.0 - mid).abs() < EPS);
}

// ── Scaled Smoothed Spread: observed-unavailable ⇒ Max Output Spread m ───────
#[test]
fn scaled_smoothed_observed_unavailable_quotes_at_max_output() {
    let s = ScaledSmoothedSpread::new(0.00008, 0.00004, 0.0002, 0.0008, 1.2);
    // No smoothed_spread on the context ⇒ indicative fallback Oₙ = m = .0008.
    assert!((s.output_spread(None) - 0.0008).abs() < EPS);
    let guards = Guardrails::new(0.0, 1.0, 1.0, 1e-6);
    let ctx = QuoteCtx::new(1.10000); // smoothed_spread == None
    let tw = quote(&[&s], &ctx, &guards, StalePolicy::Suppress).unwrap();
    // half = m/2 = .0004.
    assert!(
        (tw.offer - tw.bid - 0.0008).abs() < EPS,
        "spread {}",
        tw.offer - tw.bid
    );
}

// ── EWMA smoothing recurrence: seed, w=1 disables, geometric decay ───────────
#[test]
fn smoothing_recurrence_and_disable() {
    // Seed: S₀ = R₀ regardless of w.
    assert!((smooth(None, 0.0003, 0.25) - 0.0003).abs() < EPS);
    // w = 1 ⇒ smoothing off: Sₙ = Rₙ.
    assert!((smooth(Some(0.0009), 0.0003, 1.0) - 0.0003).abs() < EPS);
    // One step: Sₙ = w·R + (1−w)·S₋₁ = 0.25·0.0003 + 0.75·0.0009 = 0.00075.
    assert!((smooth(Some(0.0009), 0.0003, 0.25) - 0.00075).abs() < EPS);

    // Geometric decay at rate (1−w): after a step to a constant raw r from a level
    // p, (Sₙ − r) = (1−w)ⁿ (p − r). Check the first three iterates.
    let (w, r) = (0.25, 0.0003);
    let mut s = 0.0009_f64;
    let start_gap = s - r;
    for n in 1..=3 {
        s = smooth(Some(s), r, w);
        let expected_gap = (1.0 - w).powi(n) * start_gap;
        assert!((s - r - expected_gap).abs() < EPS, "n={n}");
    }
}

// ── Config: SCALE_SMOOTH serde round-trip + smoothing_weight accessor ────────
#[test]
fn scaled_smoothed_config_round_trip_and_weight() {
    let config = TieringConfig {
        unit: SpreadUnit::PricePoints,
        strategies: vec![StrategySpec::ScaledSmoothedSpread {
            smoothing_weight: 0.3,
            expected_spread: 0.00008,
            max_divergence: 0.00004,
            core_spread: 0.0002,
            max_output_spread: 0.0008,
            spread_scale_factor: 1.2,
        }],
        guardrails: Guardrails::new(0.0, 1.0, 1.0, 1e-6),
        stale_policy: StalePolicy::Suppress,
    };
    let json = serde_json::to_string(&config).unwrap();
    let back: TieringConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(config, back);
    assert_eq!(back.smoothing_weight(), Some(0.3));

    // Feed a smoothed Sₙ = 2e; end-to-end Oₙ = .00044 around mid 1.10.
    let ctx = QuoteCtx::new(1.10).with_smoothed_spread(0.00016);
    let tw = back.quote(&ctx).unwrap();
    assert!((tw.offer - tw.bid - 0.00044).abs() < EPS);

    // A config with no SCALE_SMOOTH strategy reports no smoothing weight.
    let flat_only = TieringConfig {
        unit: SpreadUnit::PriceBps,
        strategies: vec![StrategySpec::FlatMarkup { half_spread: 25.0 }],
        guardrails: Guardrails::new(0.0, 10.0, 5.0, 0.01),
        stale_policy: StalePolicy::Suppress,
    };
    assert_eq!(flat_only.smoothing_weight(), None);
}

// ── Property test: the anti-cross invariant holds for ANY config ────────────
mod props {
    use super::*;
    use proptest::prelude::*;

    fn any_unit() -> impl Strategy<Value = SpreadUnit> {
        prop_oneof![
            Just(SpreadUnit::PriceBps),
            Just(SpreadUnit::YieldBps),
            Just(SpreadUnit::PricePoints),
            Just(SpreadUnit::Percent),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2048))]
        #[test]
        fn pipeline_never_crosses_and_respects_floor(
            mid in 1.0f64..1000.0,
            inventory in -1.0e6f64..1.0e6,
            dv01 in 1.0e-3f64..0.5,           // always provided so every unit converts
            unit in any_unit(),
            flat_h in 0.0f64..5.0,
            inv_h in 0.0f64..5.0,
            kappa in -10.0f64..10.0,
            inv_s_max in 0.0f64..5.0,
            h_min in 0.0f64..1.0,
            h_span in 1.0f64..20.0,           // h_max = h_min + h_span ≥ 1 ≥ spread_floor/2
            g_s_max in 0.0f64..10.0,
            spread_floor in 1.0e-3f64..2.0,
        ) {
            let flat = FlatMarkup::new(flat_h, unit);
            let inv = InventorySkew::new(inv_h, kappa, inv_s_max, unit);
            let guards = Guardrails::new(h_min, h_min + h_span, g_s_max, spread_floor);
            let ctx = QuoteCtx::new(mid).with_inventory(inventory).with_dv01(dv01);

            let strategies: [&dyn TieringStrategy; 2] = [&flat, &inv];
            let tw = quote(&strategies, &ctx, &guards, StalePolicy::Suppress)
                .expect("valid guards + convertible unit must quote");

            // Core invariants (§4): strictly two-sided and above the floor.
            prop_assert!(tw.bid < tw.offer, "bid {} >= offer {}", tw.bid, tw.offer);
            let spread = tw.offer - tw.bid;
            prop_assert!(
                spread >= spread_floor - 1.0e-9,
                "spread {spread} < floor {spread_floor}"
            );

            // Half-spread respects [max(h_min, floor/2), h_max].
            let half = spread / 2.0;
            let h_max = h_min + h_span;
            prop_assert!(half <= h_max + 1.0e-9);
            prop_assert!(half >= (h_min.max(spread_floor / 2.0)) - 1.0e-9);

            // Skew (mid − midpoint) respects the guardrail |s| ≤ s_max.
            let skew = mid - (tw.bid + tw.offer) / 2.0;
            prop_assert!(skew.abs() <= g_s_max + 1.0e-9, "|skew| {} > s_max {g_s_max}", skew.abs());
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2048))]
        // Scaled Smoothed Spread never crosses the book and stays symmetric about
        // mid (no skew), for any smoothed level and any valid params. The output
        // spread is bounded by [c (floored), m] before guardrails; guards then only
        // widen (floor) or cap, never invert.
        #[test]
        fn scaled_smoothed_never_crosses_and_is_symmetric(
            mid in 0.5f64..2000.0,
            smoothed in 0.0f64..0.01,
            expected in 1.0e-6f64..0.005,   // e > 0
            max_div in 0.0f64..0.005,       // d ≥ 0
            core in 0.0f64..0.005,          // c ≥ 0
            extra_out in 0.0f64..0.01,      // m = c + extra ≥ c
            scale in 0.0f64..5.0,           // f ≥ 0
            h_span in 1.0f64..20.0,
            spread_floor in 1.0e-4f64..0.5,
        ) {
            let m = core + extra_out;
            let s = ScaledSmoothedSpread::new(expected, max_div, core, m, scale);
            let guards = Guardrails::new(0.0, h_span, 5.0, spread_floor);
            let ctx = QuoteCtx::new(mid).with_smoothed_spread(smoothed);
            let tw = quote(&[&s], &ctx, &guards, StalePolicy::Suppress)
                .expect("PricePoints unit always converts; valid guards must quote");

            prop_assert!(tw.bid < tw.offer, "bid {} >= offer {}", tw.bid, tw.offer);
            prop_assert!(tw.offer - tw.bid >= spread_floor - 1.0e-9);
            // No skew: strictly symmetric about mid.
            prop_assert!(((tw.bid + tw.offer) / 2.0 - mid).abs() <= 1.0e-9);
            // The pure output spread never exceeds m (before the floor may widen it).
            let o = s.output_spread(Some(smoothed));
            prop_assert!(o <= m + 1.0e-12, "O {o} > m {m}");
        }
    }
}
