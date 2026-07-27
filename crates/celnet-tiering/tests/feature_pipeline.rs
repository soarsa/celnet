//! Oracle + property tests for the composable **pricing-feature pipeline**.
//!
//! Every expected value is **hand-computed** from the feature definitions
//! (`bid = mid − h − s`, spread-invariant skews, the shipped tiering worked
//! example) — never by re-running the engine as its own oracle. The final block
//! is a **regression** proving the shipped `TieringConfig`/`quote` path is
//! byte-identical and unaffected by the additive feature engine.

use celnet_tiering::{
    AxeSide, FeatureKind, FeaturePipeline, FeatureSpec, FlatMarkup, Guardrails, InventorySkew,
    PricingCtx, PricingFeature, QuoteCtx, SpreadUnit, StalePolicy, StrategySpec, TieringConfig,
    TieringStrategy, TwoWay, quote,
};

/// f64 arithmetic tolerance — far below any price tick; effectively exact for the
/// decimal price arithmetic under test (not a lowered numerical tolerance).
const EPS: f64 = 1e-12;

fn wide_guards() -> Guardrails {
    Guardrails::new(0.0, 10.0, 5.0, 0.01)
}

fn flat_tiering(half_spread: f64, unit: SpreadUnit) -> TieringConfig {
    TieringConfig {
        unit,
        strategies: vec![StrategySpec::FlatMarkup { half_spread }],
        guardrails: wide_guards(),
        stale_policy: StalePolicy::Suppress,
    }
}

fn ctx(mid: f64) -> PricingCtx {
    PricingCtx::new(QuoteCtx::new(mid))
}

// ── MID SHIFT ────────────────────────────────────────────────────────────────
#[test]
fn mid_shift_bias_moves_both_sides_equally() {
    // Raw 99.90/100.10 (mid 100, half 0.10); bias +0.50 price points.
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    };
    let f = PricingFeature::MidShift {
        shift: 0.50,
        unit: SpreadUnit::PricePoints,
        reference: None,
    };
    let tw = f.apply(raw, &ctx(100.0));
    // Both sides up 0.50; spread unchanged.
    assert!((tw.bid - 100.40).abs() < EPS, "bid = {}", tw.bid);
    assert!((tw.offer - 100.60).abs() < EPS, "offer = {}", tw.offer);
    assert!((tw.offer - tw.bid - 0.20).abs() < EPS);
}

#[test]
fn mid_shift_reference_override_recenters_keeping_spread() {
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    }; // half 0.10
    // Override the reference to 105.0, no extra bias.
    let f = PricingFeature::MidShift {
        shift: 0.0,
        unit: SpreadUnit::PricePoints,
        reference: Some(105.0),
    };
    let tw = f.apply(raw, &ctx(100.0));
    assert!((tw.bid - 104.90).abs() < EPS, "bid = {}", tw.bid);
    assert!((tw.offer - 105.10).abs() < EPS, "offer = {}", tw.offer);
}

#[test]
fn mid_shift_reference_falls_back_to_context_reference_price() {
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    };
    // No feature-level reference ⇒ use the context's reference price 102.0.
    let f = PricingFeature::MidShift {
        shift: 0.0,
        unit: SpreadUnit::PricePoints,
        reference: None,
    };
    let pctx = PricingCtx::new(QuoteCtx::new(100.0)).with_reference_price(102.0);
    let tw = f.apply(raw, &pctx);
    assert!((tw.bid - 101.90).abs() < EPS, "bid = {}", tw.bid);
    assert!((tw.offer - 102.10).abs() < EPS, "offer = {}", tw.offer);
}

// ── TIERING (reuse of the shipped engine) ────────────────────────────────────
#[test]
fn tiering_feature_reproduces_shipped_flat_25bp_worked_example() {
    // LP composite 99.50/99.60 (mid 99.55); flat ±25 price bps ⇒ 99.30/99.80,
    // replacing the LP spread with the tier's 0.50 — the shipped worked example.
    let raw = TwoWay {
        bid: 99.50,
        offer: 99.60,
    };
    let pipeline = FeaturePipeline::new(
        vec![PricingFeature::Tiering {
            config: flat_tiering(25.0, SpreadUnit::PriceBps),
        }],
        wide_guards(),
    );
    let r = pipeline.run(raw, &ctx(99.55));
    assert!(
        (r.outbound.bid - 99.30).abs() < EPS,
        "bid = {}",
        r.outbound.bid
    );
    assert!(
        (r.outbound.offer - 99.80).abs() < EPS,
        "offer = {}",
        r.outbound.offer
    );
    // The tiering margin added = half-spread delta = 0.25 − 0.05 (raw half) = 0.20.
    assert!(
        (r.applied_margin - 0.20).abs() < EPS,
        "margin = {}",
        r.applied_margin
    );
    assert!(r.applied_skew.abs() < EPS);
}

#[test]
fn tiering_feature_is_byte_identical_to_shipped_config_quote() {
    // Degenerate raw exactly at the literal mid so the running centre is bit-exact
    // 99.55 ((x+x)/2 == x in IEEE-754), matching QuoteCtx::new(99.55).
    let config = flat_tiering(25.0, SpreadUnit::PriceBps);
    let shipped = config.quote(&QuoteCtx::new(99.55)).unwrap();

    let feature = PricingFeature::Tiering {
        config: config.clone(),
    };
    let raw = TwoWay {
        bid: 99.55,
        offer: 99.55,
    };
    let via = feature.apply(raw, &ctx(99.55));

    assert_eq!(
        shipped.bid.to_bits(),
        via.bid.to_bits(),
        "bid not byte-identical"
    );
    assert_eq!(
        shipped.offer.to_bits(),
        via.offer.to_bits(),
        "offer not byte-identical"
    );
}

#[test]
fn tiering_suppressed_leaves_running_two_way_unchanged() {
    // A stale context under Suppress ⇒ the tier produces no quote; the feature
    // must leave the running two-way exactly as-is (no margin added).
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    };
    let feature = PricingFeature::Tiering {
        config: flat_tiering(25.0, SpreadUnit::PriceBps),
    };
    let stale = PricingCtx::new(QuoteCtx::new(100.0).stale());
    let tw = feature.apply(raw, &stale);
    assert_eq!(tw, raw);
}

// ── AXE ──────────────────────────────────────────────────────────────────────
#[test]
fn axe_buy_leans_up_axe_sell_leans_down() {
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    };
    let buy = PricingFeature::Axe {
        side: AxeSide::Buy,
        magnitude: 0.05,
        unit: SpreadUnit::PricePoints,
    }
    .apply(raw, &ctx(100.0));
    // Buy axe ⇒ keener bid ⇒ both sides up 0.05; spread invariant.
    assert!((buy.bid - 99.95).abs() < EPS, "bid = {}", buy.bid);
    assert!((buy.offer - 100.15).abs() < EPS, "offer = {}", buy.offer);
    assert!((buy.offer - buy.bid - 0.20).abs() < EPS);

    let sell = PricingFeature::Axe {
        side: AxeSide::Sell,
        magnitude: 0.05,
        unit: SpreadUnit::PricePoints,
    }
    .apply(raw, &ctx(100.0));
    assert!((sell.bid - 99.85).abs() < EPS, "bid = {}", sell.bid);
    assert!((sell.offer - 100.05).abs() < EPS, "offer = {}", sell.offer);
}

// ── POSITION (reuse of InventorySkew) ────────────────────────────────────────
#[test]
fn position_skews_to_shed_inventory_matching_inventory_skew() {
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    };
    let feature = PricingFeature::Position {
        kappa: 0.01,
        s_max: 1.0,
        unit: SpreadUnit::PricePoints,
    };

    // Long q=+20: skew = clamp(0.01·20, ±1) = 0.20 ⇒ BOTH sides down 0.20.
    let long = feature.apply(
        raw,
        &PricingCtx::new(QuoteCtx::new(100.0).with_inventory(20.0)),
    );
    assert!((long.bid - 99.70).abs() < EPS, "bid = {}", long.bid);
    assert!((long.offer - 99.90).abs() < EPS, "offer = {}", long.offer);

    // Short q=−20: skew = −0.20 ⇒ BOTH sides up 0.20.
    let short = feature.apply(
        raw,
        &PricingCtx::new(QuoteCtx::new(100.0).with_inventory(-20.0)),
    );
    assert!((short.bid - 100.10).abs() < EPS, "bid = {}", short.bid);
    assert!(
        (short.offer - 100.30).abs() < EPS,
        "offer = {}",
        short.offer
    );

    // The skew magnitude equals the shipped InventorySkew contribution exactly.
    let qctx = QuoteCtx::new(100.0).with_inventory(20.0);
    let shipped_skew = InventorySkew::new(0.0, 0.01, 1.0, SpreadUnit::PricePoints)
        .adjust(&qctx)
        .skew;
    assert!((shipped_skew - 0.20).abs() < EPS);
}

// ── PANIC SKEW ───────────────────────────────────────────────────────────────
#[test]
fn panic_skew_applies_only_when_triggered() {
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    };
    // Triggered, +0.10 ⇒ both sides down 0.10.
    let on = PricingFeature::PanicSkew {
        skew: 0.10,
        unit: SpreadUnit::PricePoints,
        triggered: true,
    }
    .apply(raw, &ctx(100.0));
    assert!((on.bid - 99.80).abs() < EPS, "bid = {}", on.bid);
    assert!((on.offer - 100.00).abs() < EPS, "offer = {}", on.offer);

    // Not triggered ⇒ identity.
    let off = PricingFeature::PanicSkew {
        skew: 0.10,
        unit: SpreadUnit::PricePoints,
        triggered: false,
    }
    .apply(raw, &ctx(100.0));
    assert_eq!(off, raw);
}

// ── ORDER + PROVENANCE WATERFALL: the canonical composed pipeline ─────────────
#[test]
fn full_pipeline_order_and_waterfall_hand_oracle() {
    // RAW 99.90/100.10 (mid 100), then:
    //   MidShift +0.50   → centre 100.50            → 100.40/100.60
    //   Tiering flat 0.25→ 100.50 ∓ 0.25            → 100.25/100.75  (half 0.25)
    //   Axe Buy 0.05     → +0.05 both               → 100.30/100.80
    //   Position q=+20   → skew 0.20 down           → 100.10/100.60
    //   PanicSkew +0.10  → 0.10 down                → 100.00/100.50
    // Guards wide ⇒ outbound == last stage.
    let raw = TwoWay {
        bid: 99.90,
        offer: 100.10,
    };
    let pipeline = FeaturePipeline::new(
        vec![
            PricingFeature::MidShift {
                shift: 0.50,
                unit: SpreadUnit::PricePoints,
                reference: None,
            },
            PricingFeature::Tiering {
                config: flat_tiering(0.25, SpreadUnit::PricePoints),
            },
            PricingFeature::Axe {
                side: AxeSide::Buy,
                magnitude: 0.05,
                unit: SpreadUnit::PricePoints,
            },
            PricingFeature::Position {
                kappa: 0.01,
                s_max: 1.0,
                unit: SpreadUnit::PricePoints,
            },
            PricingFeature::PanicSkew {
                skew: 0.10,
                unit: SpreadUnit::PricePoints,
                triggered: true,
            },
        ],
        wide_guards(),
    );
    let pctx = PricingCtx::new(QuoteCtx::new(100.0).with_inventory(20.0));
    let r = pipeline.run(raw, &pctx);

    // Every stage matches the hand oracle, in order.
    let expect = [
        (FeatureKind::MidShift, 100.40, 100.60),
        (FeatureKind::Tiering, 100.25, 100.75),
        (FeatureKind::Axe, 100.30, 100.80),
        (FeatureKind::Position, 100.10, 100.60),
        (FeatureKind::PanicSkew, 100.00, 100.50),
    ];
    assert_eq!(r.after.len(), expect.len());
    for (i, (kind, bid, offer)) in expect.iter().enumerate() {
        let (k, tw) = r.after[i];
        assert_eq!(k, *kind, "stage {i} kind");
        assert!((tw.bid - bid).abs() < EPS, "stage {i} bid = {}", tw.bid);
        assert!(
            (tw.offer - offer).abs() < EPS,
            "stage {i} offer = {}",
            tw.offer
        );
    }

    // Outbound (guards do not bind).
    assert!(
        (r.outbound.bid - 100.00).abs() < EPS,
        "bid = {}",
        r.outbound.bid
    );
    assert!(
        (r.outbound.offer - 100.50).abs() < EPS,
        "offer = {}",
        r.outbound.offer
    );

    // Attribution: margin = tiering half delta (0.25 − 0.10) = 0.15;
    // skew = Axe(+0.05) + Position(−0.20) + Panic(−0.10) = −0.25.
    assert!(
        (r.applied_margin - 0.15).abs() < EPS,
        "margin = {}",
        r.applied_margin
    );
    assert!(
        (r.applied_skew + 0.25).abs() < EPS,
        "skew = {}",
        r.applied_skew
    );

    // The waterfall reconstructs the price exactly: per-stage deltas telescope.
    let mut bid_sum = 0.0;
    let mut offer_sum = 0.0;
    let mut prev = raw;
    for (_, tw) in &r.after {
        bid_sum += tw.bid - prev.bid;
        offer_sum += tw.offer - prev.offer;
        prev = *tw;
    }
    // Plus the closing guardrail delta (zero here).
    bid_sum += r.outbound.bid - prev.bid;
    offer_sum += r.outbound.offer - prev.offer;
    assert!((bid_sum - (r.outbound.bid - raw.bid)).abs() < EPS);
    assert!((offer_sum - (r.outbound.offer - raw.offer)).abs() < EPS);
}

// ── SERDE round-trip: every feature kind + the pipeline shape ─────────────────
#[test]
fn feature_pipeline_serde_round_trip_all_kinds() {
    let pipeline = FeaturePipeline::new(
        vec![
            PricingFeature::MidShift {
                shift: -1.5,
                unit: SpreadUnit::PriceBps,
                reference: Some(101.25),
            },
            PricingFeature::Tiering {
                config: flat_tiering(25.0, SpreadUnit::PriceBps),
            },
            PricingFeature::Axe {
                side: AxeSide::Sell,
                magnitude: 3.0,
                unit: SpreadUnit::YieldBps,
            },
            PricingFeature::Position {
                kappa: 0.5,
                s_max: 100.0,
                unit: SpreadUnit::PriceBps,
            },
            PricingFeature::PanicSkew {
                skew: 2.0,
                unit: SpreadUnit::PricePoints,
                triggered: true,
            },
        ],
        wide_guards(),
    );
    let json = serde_json::to_string(&pipeline).unwrap();
    let back: FeaturePipeline = serde_json::from_str(&json).unwrap();
    assert_eq!(pipeline, back);

    // The tag is "kind" (matching StrategySpec) and it carries the full TieringConfig.
    assert!(json.contains("\"kind\":\"MidShift\""));
    assert!(json.contains("\"kind\":\"Tiering\""));

    // FeatureSpec is the pipeline element type (alias of PricingFeature).
    let _: &Vec<FeatureSpec> = &back.features;
}

// ── REGRESSION: the shipped tiering path is unaffected + byte-identical ───────
#[test]
fn shipped_tiering_path_is_unaffected() {
    // The exact shipped worked example, through the shipped free `quote` and the
    // shipped `TieringConfig::quote` — must still be 99.30/99.80.
    let flat = FlatMarkup::new(25.0, SpreadUnit::PriceBps);
    let qctx = QuoteCtx::new(99.55);
    let via_free = quote(&[&flat], &qctx, &wide_guards(), StalePolicy::Suppress).unwrap();
    assert!((via_free.bid - 99.30).abs() < EPS);
    assert!((via_free.offer - 99.80).abs() < EPS);

    let via_config = flat_tiering(25.0, SpreadUnit::PriceBps)
        .quote(&qctx)
        .unwrap();
    assert_eq!(via_free.bid.to_bits(), via_config.bid.to_bits());
    assert_eq!(via_free.offer.to_bits(), via_config.offer.to_bits());
}

// ── Property tests ───────────────────────────────────────────────────────────
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

    fn any_side() -> impl Strategy<Value = AxeSide> {
        prop_oneof![Just(AxeSide::Buy), Just(AxeSide::Sell)]
    }

    fn any_feature() -> impl Strategy<Value = PricingFeature> {
        prop_oneof![
            (
                -2.0f64..2.0,
                any_unit(),
                proptest::option::of(1.0f64..1000.0)
            )
                .prop_map(|(shift, unit, reference)| PricingFeature::MidShift {
                    shift,
                    unit,
                    reference
                }),
            (0.0f64..2.0, any_unit()).prop_map(|(half_spread, unit)| PricingFeature::Tiering {
                config: TieringConfig {
                    unit,
                    strategies: vec![StrategySpec::FlatMarkup { half_spread }],
                    guardrails: Guardrails::new(0.0, 20.0, 5.0, 1.0e-3),
                    stale_policy: StalePolicy::Suppress,
                }
            }),
            (any_side(), 0.0f64..2.0, any_unit()).prop_map(|(side, magnitude, unit)| {
                PricingFeature::Axe {
                    side,
                    magnitude,
                    unit,
                }
            }),
            (-5.0f64..5.0, 0.0f64..5.0, any_unit()).prop_map(|(kappa, s_max, unit)| {
                PricingFeature::Position { kappa, s_max, unit }
            }),
            (-2.0f64..2.0, any_unit(), any::<bool>()).prop_map(|(skew, unit, triggered)| {
                PricingFeature::PanicSkew {
                    skew,
                    unit,
                    triggered,
                }
            }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2048))]
        // The pipeline NEVER crosses the book and always honours the spread floor,
        // for ANY composed feature list, ANY signed inventory, and ANY valid guards.
        #[test]
        fn pipeline_never_crosses_and_respects_floor(
            mid in 1.0f64..1000.0,
            raw_half in 0.0f64..5.0,
            inventory in -1.0e6f64..1.0e6,
            dv01 in 1.0e-3f64..0.5,          // always present ⇒ every unit converts
            features in prop::collection::vec(any_feature(), 0..8),
            h_min in 0.0f64..1.0,
            h_span in 1.0f64..20.0,          // h_max = h_min + h_span ≥ 1 ≥ floor/2
            g_s_max in 0.0f64..10.0,
            spread_floor in 1.0e-3f64..2.0,
        ) {
            let raw = TwoWay { bid: mid - raw_half, offer: mid + raw_half };
            let guards = Guardrails::new(h_min, h_min + h_span, g_s_max, spread_floor);
            let pipeline = FeaturePipeline::new(features, guards);
            let pctx = PricingCtx::new(
                QuoteCtx::new(mid).with_inventory(inventory).with_dv01(dv01),
            );

            let r = pipeline.run(raw, &pctx);

            // Anti-cross + floor: strictly two-sided, above the floor.
            prop_assert!(r.outbound.bid < r.outbound.offer,
                "bid {} >= offer {}", r.outbound.bid, r.outbound.offer);
            let spread = r.outbound.offer - r.outbound.bid;
            prop_assert!(spread >= spread_floor - 1.0e-9,
                "spread {spread} < floor {spread_floor}");

            // Waterfall telescopes to outbound − raw (per-stage deltas + guard delta).
            let mut prev = raw;
            let mut bid_sum = 0.0f64;
            for (_, tw) in &r.after { bid_sum += tw.bid - prev.bid; prev = *tw; }
            bid_sum += r.outbound.bid - prev.bid;
            prop_assert!((bid_sum - (r.outbound.bid - raw.bid)).abs() < 1.0e-6);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1024))]
        // Pure skew / shift features are spread-invariant: only Tiering changes width.
        #[test]
        fn skew_and_shift_features_are_spread_invariant(
            mid in 1.0f64..1000.0,
            raw_half in 1.0e-3f64..5.0,
            inventory in -1.0e6f64..1.0e6,
            magnitude in 0.0f64..2.0,
            shift in -2.0f64..2.0,
            kappa in -5.0f64..5.0,
            s_max in 0.0f64..5.0,
            side in any_side(),
        ) {
            let raw = TwoWay { bid: mid - raw_half, offer: mid + raw_half };
            let pctx = PricingCtx::new(QuoteCtx::new(mid).with_inventory(inventory));
            let width = raw.offer - raw.bid;
            let u = SpreadUnit::PricePoints;

            for f in [
                PricingFeature::MidShift { shift, unit: u, reference: None },
                PricingFeature::Axe { side, magnitude, unit: u },
                PricingFeature::Position { kappa, s_max, unit: u },
                PricingFeature::PanicSkew { skew: shift, unit: u, triggered: true },
            ] {
                let tw = f.apply(raw, &pctx);
                prop_assert!((tw.offer - tw.bid - width).abs() < 1.0e-9,
                    "{:?} changed width", f.kind());
            }
        }
    }
}

// ── PROVENANCE WATERFALL ACCESSORS (design §7) ───────────────────────────────
// `PricedResult::constructed` / `tiered` / `feature_kinds` are what the server
// stamps as execution pricing provenance. Every value is hand-computed from the
// feature definitions, never read back from the engine.

#[test]
fn provenance_tiering_only_constructed_is_raw_tiered_is_after_margin() {
    // Design worked example: a group Flat ±0.25 price-points on a 99.55 raw mid.
    // No MID SHIFT stage ⇒ constructed = RAW (the prior stage); tiered = after Flat.
    let raw = TwoWay {
        bid: 99.55,
        offer: 99.55, // zero raw spread, so applied_margin = the full 0.25 half-spread
    };
    let pipeline = FeaturePipeline::new(
        vec![FeatureSpec::Tiering {
            config: flat_tiering(0.25, SpreadUnit::PricePoints),
        }],
        wide_guards(),
    );
    let r = pipeline.run(raw, &ctx(99.55));

    assert_eq!(r.constructed(), raw, "no MID SHIFT ⇒ constructed = RAW");
    let tiered = r.tiered();
    assert!(
        (tiered.bid - 99.30).abs() < EPS,
        "tiered.bid = {}",
        tiered.bid
    );
    assert!(
        (tiered.offer - 99.80).abs() < EPS,
        "tiered.offer = {}",
        tiered.offer
    );
    assert_eq!(r.feature_kinds(), vec![FeatureKind::Tiering]);
    assert!(
        (r.applied_margin - 0.25).abs() < EPS,
        "applied_margin = {}",
        r.applied_margin
    );
    // Waterfall reconstructs: raw → constructed → tiered, with the margin explaining
    // the half-spread growth from constructed to tiered.
    let constructed_half = 0.5 * (r.constructed().offer - r.constructed().bid);
    let tiered_half = 0.5 * (tiered.offer - tiered.bid);
    assert!((tiered_half - constructed_half - r.applied_margin).abs() < EPS);
}

#[test]
fn provenance_mid_shift_then_tiering_stages_are_each_captured() {
    // RAW 99.50/99.60 (mid 99.55, half 0.05) ▸ MID SHIFT +0.10 ▸ Flat ±0.25.
    let raw = TwoWay {
        bid: 99.50,
        offer: 99.60,
    };
    let pipeline = FeaturePipeline::new(
        vec![
            FeatureSpec::MidShift {
                shift: 0.10,
                unit: SpreadUnit::PricePoints,
                reference: None,
            },
            FeatureSpec::Tiering {
                config: flat_tiering(0.25, SpreadUnit::PricePoints),
            },
        ],
        wide_guards(),
    );
    let r = pipeline.run(raw, &ctx(99.55));

    // Constructed: mid 99.55 + 0.10 = 99.65, half 0.05 preserved ⇒ 99.60/99.70.
    let c = r.constructed();
    assert!((c.bid - 99.60).abs() < EPS, "constructed.bid = {}", c.bid);
    assert!(
        (c.offer - 99.70).abs() < EPS,
        "constructed.offer = {}",
        c.offer
    );
    // Tiered: Flat ±0.25 around constructed mid 99.65 ⇒ 99.40/99.90.
    let t = r.tiered();
    assert!((t.bid - 99.40).abs() < EPS, "tiered.bid = {}", t.bid);
    assert!((t.offer - 99.90).abs() < EPS, "tiered.offer = {}", t.offer);
    assert_eq!(
        r.feature_kinds(),
        vec![FeatureKind::MidShift, FeatureKind::Tiering]
    );
    // MID SHIFT is desk construction (no margin); the Flat adds 0.20 half-spread
    // (0.25 − the constructed 0.05).
    assert!(
        (r.applied_margin - 0.20).abs() < EPS,
        "margin = {}",
        r.applied_margin
    );
}

#[test]
fn provenance_no_mid_shift_no_tiering_falls_back_to_prior_stage() {
    // A skew-only pipeline: constructed = RAW, tiered = constructed (both prior).
    let raw = TwoWay {
        bid: 99.50,
        offer: 99.60,
    };
    let pipeline = FeaturePipeline::new(
        vec![FeatureSpec::Axe {
            side: AxeSide::Buy,
            magnitude: 0.20,
            unit: SpreadUnit::PricePoints,
        }],
        wide_guards(),
    );
    let r = pipeline.run(raw, &ctx(99.55));
    assert_eq!(r.constructed(), raw, "no MID SHIFT ⇒ constructed = RAW");
    assert_eq!(
        r.tiered(),
        r.constructed(),
        "no TIERING ⇒ tiered = constructed"
    );
    assert_eq!(r.feature_kinds(), vec![FeatureKind::Axe]);
    assert!((r.applied_margin).abs() < EPS, "skew adds no margin");
}
