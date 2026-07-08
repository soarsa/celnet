//! Differential byte-identity harness for the descriptor-driven WS encoder
//! (arch item G — `ws-codec-from-proto`, increment 2).
//!
//! Proves, for concrete non-default instances, that the **generated**
//! descriptor-driven encoder ([`celnet_server::ws::generated_codec`]) is
//! **byte-for-byte identical** to the hand codec ([`celnet_server::ws::codec`])
//! before `handle_unary` is ever swapped onto the generated path (the final
//! increment). Two comparison modes, both against the hand codec as the reference:
//!
//! 1. **Encode byte-identity** (`assert_bytes_eq`) — for messages the hand codec
//!    *encodes* (`CcyPair`, `Underlying`, `MarketContext`, `Greeks`,
//!    `RateSensitivities`): the two encoders must serialize to the identical JSON
//!    text. (`serde_json::Value` is `BTreeMap`-backed here — `preserve_order` is
//!    off — so the serialized text is key-order-independent; the assertion is on
//!    the exact serialized bytes regardless.)
//!
//! 2. **Round-trip through the hand decoder** (`assert_roundtrip_*`) — for the
//!    decode-only messages `Tenor` and `Strategy`, which the hand codec never
//!    *encodes* (there is no `tenor_to_json` / `strategy_to_json`), so there are
//!    no hand-encoded reference bytes to diff against. Instead the generated
//!    encoder's output must round-trip through the hand *decoder* to the identical
//!    proto — which fails loudly if the generated keys/casing diverge (e.g. if it
//!    emitted snake_case `broken_date` the hand decoder reads `brokenDate` and the
//!    field would silently vanish, breaking the round-trip). This exercises the
//!    same override table (quirk d, the `strike`/`delta` oneof body) via the hand
//!    codec's own contract.
//!
//! Covers all four FX-legacy divergences — (a) `Underlying`, (b) `MarketContext`,
//! (c) `Greeks` flat rhos, (d) `Tenor.brokenDate` — plus representative simple /
//! nested / repeated / oneof messages (`CcyPair`, `Strategy`, `Greeks`,
//! `Underlying`, `RateSensitivities`).

use celnet_proto::{
    BrokenDate, CcyPair, Greeks, Leg, MarketContext, MetalPair, RateSensitivities, Strategy,
    StrategyKind, StrikeOrDelta, Tenor, Underlying,
};
use celnet_proto::{OptionType, Side, rate_sensitivities, strike_or_delta, tenor};
use celnet_server::ws::codec::diff_support as hand;
use celnet_server::ws::generated_codec as generated;
use serde_json::{Value, json};

/// Serialize to the exact JSON text — the byte string the WS mirror puts on the
/// wire. Byte-for-byte equality of these strings is the byte-identity contract.
fn bytes(v: &Value) -> String {
    serde_json::to_string(v).expect("serialize to JSON text")
}

/// Assert the generated encoding and the hand encoding serialize byte-for-byte
/// identically, and print the shared bytes for the record.
#[track_caller]
fn assert_bytes_eq(label: &str, generated: &Value, hand: &Value) {
    let g = bytes(generated);
    let h = bytes(hand);
    assert_eq!(
        g, h,
        "[{label}] generated encoder diverged from the hand codec\n  generated: {g}\n  hand:      {h}"
    );
}

// ---------------------------------------------------------------------------
// (simple) CcyPair
// ---------------------------------------------------------------------------

#[test]
fn ccy_pair_is_byte_identical() {
    let p = CcyPair {
        base: "EUR".to_owned(),
        quote: "USD".to_owned(),
    };
    assert_bytes_eq(
        "CcyPair",
        &generated::encode_ccy_pair(&p),
        &hand::hand_ccy_pair(&p),
    );
}

// ---------------------------------------------------------------------------
// (FX-legacy a) Underlying → legacy {base, quote} pair
// ---------------------------------------------------------------------------

#[test]
fn underlying_fx_is_byte_identical() {
    let u = Underlying::fx(CcyPair {
        base: "GBP".to_owned(),
        quote: "JPY".to_owned(),
    });
    let g = generated::encode_underlying(&u);
    // Confirm the FX-legacy projection: the `{base, quote}` pair body, NOT the
    // `{fx: {..}}` oneof shape / `settlement_ccy`.
    assert_eq!(g.get("base").and_then(Value::as_str), Some("GBP"));
    assert_eq!(g.get("quote").and_then(Value::as_str), Some("JPY"));
    assert!(g.get("fx").is_none(), "no `ref` oneof arm on the wire");
    assert!(g.get("settlement_ccy").is_none(), "no settlement_ccy leak");
    assert_bytes_eq("Underlying(fx)", &g, &hand::hand_underlying(&u));
}

#[test]
fn underlying_non_fx_is_null_like_the_hand_codec() {
    // A non-FX underlying never reaches this FX-only WS surface; both encoders
    // project it to JSON null.
    let u = Underlying::metal(MetalPair {
        metal: 0,
        quote: "USD".to_owned(),
    });
    let g = generated::encode_underlying(&u);
    assert!(g.is_null());
    assert_bytes_eq("Underlying(metal)", &g, &hand::hand_underlying(&u));
}

// ---------------------------------------------------------------------------
// (FX-legacy b) MarketContext → {spot, vol, r_dom, r_for}
// ---------------------------------------------------------------------------

#[test]
fn market_context_is_byte_identical() {
    let m = MarketContext::fx(1.082_53, 0.091_25, 0.042_10, 0.018_70);
    let g = generated::encode_market_context(&m);
    // The FX-legacy accessors, not the generalized {discount_rate, carry}.
    assert!(g.get("r_dom").is_some() && g.get("r_for").is_some());
    assert!(g.get("discount_rate").is_none() && g.get("carry").is_none());
    assert_bytes_eq("MarketContext", &g, &hand::hand_market_context(&m));
}

// ---------------------------------------------------------------------------
// (oneof, nested) RateSensitivities — both arms
// ---------------------------------------------------------------------------

#[test]
fn rate_sensitivities_fx_arm_is_byte_identical() {
    let rs = RateSensitivities::fx(1_234.5, -678.25);
    assert_bytes_eq(
        "RateSensitivities(fx)",
        &generated::encode_rate_sensitivities(&rs),
        &hand::hand_rate_sensitivities(&rs),
    );
}

#[test]
fn rate_sensitivities_carry_arm_is_byte_identical() {
    let rs = RateSensitivities {
        sensitivities: Some(rate_sensitivities::Sensitivities::Carry(
            rate_sensitivities::CarryRho {
                discount_rho: 91.5,
                carry_rho: -42.25,
            },
        )),
    };
    assert_bytes_eq(
        "RateSensitivities(carry)",
        &generated::encode_rate_sensitivities(&rs),
        &hand::hand_rate_sensitivities(&rs),
    );
}

// ---------------------------------------------------------------------------
// (FX-legacy c, nested) Greeks — flat rhos synthesized beside the oneof
// ---------------------------------------------------------------------------

/// A fully-populated Greeks strip with non-default values on every field so the
/// comparison is meaningful; `rate_sensitivities` set to `arm`.
fn greeks_with(arm: Option<RateSensitivities>) -> Greeks {
    Greeks {
        price: 0.021_4,
        delta_spot: 0.512,
        delta_forward: 0.498,
        gamma: 3.71,
        vega: 0.089_2,
        theta: -0.004_5,
        rate_sensitivities: arm,
        vanna: 0.013_3,
        volga: 0.204,
        charm: -0.001_2,
        speed: 0.000_9,
        zomma: -0.005_6,
        color: 0.000_3,
    }
}

#[test]
fn greeks_with_fx_rhos_is_byte_identical() {
    let g = greeks_with(Some(RateSensitivities::fx(880.5, -410.25)));
    let generated = generated::encode_greeks(&g);
    // The flat FX-legacy rhos are emitted beside the carry-tagged oneof.
    assert!(generated.get("rho_dom").is_some() && generated.get("rho_for").is_some());
    assert!(generated.get("rate_sensitivities").is_some());
    assert_bytes_eq("Greeks(fx)", &generated, &hand::hand_greeks(&g));
}

#[test]
fn greeks_with_carry_rhos_is_byte_identical() {
    // Cross-asset carry arm: rho_dom/rho_for are the lossless flat projection.
    let g = greeks_with(Some(RateSensitivities {
        sensitivities: Some(rate_sensitivities::Sensitivities::Carry(
            rate_sensitivities::CarryRho {
                discount_rho: 120.0,
                carry_rho: -55.0,
            },
        )),
    }));
    assert_bytes_eq(
        "Greeks(carry)",
        &generated::encode_greeks(&g),
        &hand::hand_greeks(&g),
    );
}

#[test]
fn greeks_without_rate_sensitivities_is_byte_identical() {
    // Absent strip: rho_dom/rho_for are 0.0 and rate_sensitivities is JSON null.
    let g = greeks_with(None);
    let generated = generated::encode_greeks(&g);
    assert_eq!(generated.get("rate_sensitivities"), Some(&Value::Null));
    assert_bytes_eq("Greeks(none)", &generated, &hand::hand_greeks(&g));
}

// ---------------------------------------------------------------------------
// (FX-legacy d, round-trip) Tenor.brokenDate camelCase
// ---------------------------------------------------------------------------

#[test]
fn tenor_broken_date_roundtrips_through_the_hand_decoder() {
    let t = Tenor {
        unit: tenor::Unit::BrokenDate as i32,
        count: 0,
        broken_date: Some(BrokenDate {
            year: 2026,
            month: 9,
            day: 18,
        }),
    };
    let encoded = generated::encode_tenor(&t);
    // Quirk (d): camelCase `brokenDate` on the wire, never snake_case.
    assert!(
        encoded.get("brokenDate").is_some(),
        "camelCase brokenDate key must be present: {encoded}"
    );
    assert!(
        encoded.get("broken_date").is_none(),
        "snake_case broken_date must NOT appear: {encoded}"
    );
    let decoded =
        hand::hand_tenor_from_json(&encoded).expect("hand decoder accepts generated JSON");
    assert_eq!(decoded, t, "Tenor round-trips byte-identically");
}

#[test]
fn tenor_without_broken_date_roundtrips() {
    let t = Tenor {
        unit: tenor::Unit::Months as i32,
        count: 3,
        broken_date: None,
    };
    let encoded = generated::encode_tenor(&t);
    assert!(
        encoded.get("brokenDate").is_none(),
        "absent optional brokenDate is omitted: {encoded}"
    );
    let decoded =
        hand::hand_tenor_from_json(&encoded).expect("hand decoder accepts generated JSON");
    assert_eq!(decoded, t);
}

// ---------------------------------------------------------------------------
// (repeated, nested oneof, round-trip) Strategy
// ---------------------------------------------------------------------------

#[test]
fn strategy_roundtrips_through_the_hand_decoder() {
    let s = Strategy {
        kind: StrategyKind::RiskReversal as i32,
        legs: vec![
            Leg {
                option_type: OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    // The delta arm of the strike oneof.
                    spec: Some(strike_or_delta::Spec::Delta(0.25)),
                }),
                side: Side::Buy as i32,
                ratio: 1.0,
            },
            Leg {
                option_type: OptionType::Put as i32,
                strike: Some(StrikeOrDelta {
                    // The absolute-strike arm of the strike oneof.
                    spec: Some(strike_or_delta::Spec::Strike(1.082_53)),
                }),
                side: Side::Sell as i32,
                ratio: 2.0,
            },
        ],
    };
    let encoded = generated::encode_strategy(&s);
    // The repeated legs array + the per-leg strike/delta oneof bodies.
    let legs = encoded
        .get("legs")
        .and_then(Value::as_array)
        .expect("legs array");
    assert_eq!(legs.len(), 2);
    // Each leg's strike/delta oneof body is nested under the leg's `strike` key.
    assert!(
        legs[0].get("strike").and_then(|s| s.get("delta")).is_some(),
        "leg 0 carries the delta arm inside its strike body: {}",
        legs[0]
    );
    assert!(
        legs[1]
            .get("strike")
            .and_then(|s| s.get("strike"))
            .is_some(),
        "leg 1 carries the absolute-strike arm inside its strike body: {}",
        legs[1]
    );
    let decoded =
        hand::hand_strategy_from_json(&encoded).expect("hand decoder accepts generated JSON");
    assert_eq!(decoded, s, "Strategy round-trips byte-identically");
}

// ===========================================================================
// DECODE byte-identity (increment 3): the descriptor-driven decoder produces the
// byte-identical proto message the hand decoder produces, over the request-side
// leaf bodies of the Instrument / Price family.
// ===========================================================================
//
// Equality of the DECODED proto messages IS the decode byte-identity contract:
// two decoders agree on a body iff they build the same message — which re-encodes
// to the identical protobuf bytes. Each assertion checks BOTH: `PartialEq` of the
// messages AND equality of their re-serialized protobuf bytes (the literal wire
// form), so a divergence in any field — value, presence, or oneof arm — fails
// loudly. The inputs are the exact JSON bodies a browser / Excel client sends
// (the `ws_mirror` conformance corpus shapes).

use prost::Message;

/// Assert the generated decoder and the hand decoder both accept `label`'s body and
/// build the byte-identical proto message (equal by `PartialEq` AND re-encoding to
/// identical protobuf bytes).
#[track_caller]
fn assert_decode_eq<T, E>(label: &str, generated: Result<T, E>, hand: Result<T, E>)
where
    T: Message + PartialEq + Default,
    E: std::fmt::Display,
{
    let g = generated.unwrap_or_else(|e| panic!("[{label}] generated decode failed: {e}"));
    let h = hand.unwrap_or_else(|e| panic!("[{label}] hand decode failed: {e}"));
    assert_eq!(
        g, h,
        "[{label}] generated decoder built a different message than the hand codec"
    );
    assert_eq!(
        g.encode_to_vec(),
        h.encode_to_vec(),
        "[{label}] decoded protobuf bytes differ (not byte-identical)"
    );
}

#[test]
fn ccy_pair_decode_is_byte_identical() {
    let v = json!({ "base": "EUR", "quote": "USD" });
    assert_decode_eq(
        "CcyPair",
        generated::decode_ccy_pair(&v),
        hand::hand_ccy_pair_from_json(&v),
    );
}

#[test]
fn underlying_fx_decode_is_byte_identical() {
    // The FX-legacy `{base, quote}` pair → the FX `Underlying` arm (quirk a).
    let v = json!({ "base": "GBP", "quote": "JPY" });
    assert_decode_eq(
        "Underlying(fx)",
        generated::decode_underlying_fx(&v),
        hand::hand_underlying_from_json(&v),
    );
}

#[test]
fn underlying_object_arms_decode_are_byte_identical() {
    // Every cross-asset `underlying` oneof arm decodes to the identical proto.
    let cases = [
        ("fx", json!({ "fx": { "base": "EUR", "quote": "USD" } })),
        ("metal", json!({ "metal": { "metal": 1, "quote": "USD" } })),
        (
            "equity",
            json!({ "equity": { "symbol": { "ticker": "AAPL", "venue": "XNAS" }, "currency": "USD" } }),
        ),
        (
            "commodity",
            json!({ "commodity": { "symbol": { "ticker": "CL", "venue": "NYMEX" }, "currency": "USD" } }),
        ),
        (
            "digital_asset",
            json!({ "digital_asset": { "base": "BTC", "quote": "USDT" } }),
        ),
    ];
    for (arm, v) in cases {
        assert_decode_eq(
            &format!("Underlying(object/{arm})"),
            generated::decode_underlying_object(&v),
            hand::hand_underlying_object_from_json(&v),
        );
    }
}

#[test]
fn market_context_decode_is_byte_identical() {
    // The FX `r_dom`/`r_for` keys → the byte-identical `fx` constructor (quirk b).
    let v = json!({ "spot": 1.082_53, "vol": 0.091_25, "r_dom": 0.042_10, "r_for": 0.018_70 });
    assert_decode_eq(
        "MarketContext",
        generated::decode_market_context(&v),
        hand::hand_market_context_from_json(&v),
    );
    // r_dom / r_for absent ⇒ proto3 default 0.0 on both paths.
    let v0 = json!({ "spot": 1.10, "vol": 0.08 });
    assert_decode_eq(
        "MarketContext(no rates)",
        generated::decode_market_context(&v0),
        hand::hand_market_context_from_json(&v0),
    );
}

#[test]
fn tenor_decode_is_byte_identical() {
    // Plain tenor.
    let months = json!({ "unit": 3, "count": 1 });
    assert_decode_eq(
        "Tenor(months)",
        generated::decode_tenor(&months),
        hand::hand_tenor_from_json(&months),
    );
    // camelCase `brokenDate` (quirk d) decoded identically.
    let broken =
        json!({ "unit": 8, "count": 0, "brokenDate": { "year": 2026, "month": 9, "day": 18 } });
    assert_decode_eq(
        "Tenor(brokenDate)",
        generated::decode_tenor(&broken),
        hand::hand_tenor_from_json(&broken),
    );
}

#[test]
fn conventions_decode_is_byte_identical() {
    let v = json!({
        "delta_convention": 1, "atm_convention": 2, "premium_style": 1,
        "cut": 3, "day_count": 1, "settlement": 1
    });
    assert_decode_eq(
        "Conventions",
        generated::decode_conventions(&v),
        hand::hand_conventions_from_json(&v),
    );
    // All-absent ⇒ all proto3-zero on both paths.
    let z = json!({});
    assert_decode_eq(
        "Conventions(default)",
        generated::decode_conventions(&z),
        hand::hand_conventions_from_json(&z),
    );
}

#[test]
fn quantity_and_solve_decode_are_byte_identical() {
    let q = json!({ "notional": 1_000_000.0, "base_ccy": true });
    assert_decode_eq(
        "Quantity",
        generated::decode_quantity(&q),
        hand::hand_quantity_from_json(&q),
    );
    let s = json!({ "target": 1, "target_premium": 0.021_4 });
    assert_decode_eq(
        "Solve",
        generated::decode_solve(&s),
        hand::hand_solve_from_json(&s),
    );
}

#[test]
fn strike_or_delta_decode_arms_are_byte_identical() {
    let strike = json!({ "strike": 1.082_53 });
    assert_decode_eq(
        "StrikeOrDelta(strike)",
        generated::decode_strike_or_delta(&strike),
        hand::hand_strike_or_delta_from_json(&strike),
    );
    let delta = json!({ "delta": 0.25 });
    assert_decode_eq(
        "StrikeOrDelta(delta)",
        generated::decode_strike_or_delta(&delta),
        hand::hand_strike_or_delta_from_json(&delta),
    );
}

#[test]
fn vanilla_and_strategy_decode_are_byte_identical() {
    let vanilla = json!({ "option_type": 0, "strike": { "strike": 1.12 } });
    assert_decode_eq(
        "Vanilla",
        generated::decode_vanilla(&vanilla),
        hand::hand_vanilla_from_json(&vanilla),
    );
    let strategy = json!({
        "kind": 1,
        "legs": [
            { "option_type": 0, "strike": { "delta": 0.25 }, "side": 1, "ratio": 1.0 },
            { "option_type": 1, "strike": { "strike": 1.082_53 }, "side": 2, "ratio": 2.0 }
        ]
    });
    assert_decode_eq(
        "Strategy",
        generated::decode_strategy(&strategy),
        hand::hand_strategy_from_json(&strategy),
    );
}

/// The exact instrument leaf bodies from the `ws_mirror` conformance corpus
/// (`vanilla_call_json` / `conventions_json`) decode byte-identically through the
/// generated path — the browser/Excel wire shapes, not synthetic values.
#[test]
fn ws_mirror_corpus_leaf_bodies_decode_byte_identical() {
    let pair = json!({ "base": "EUR", "quote": "USD" });
    assert_decode_eq(
        "corpus/pair",
        generated::decode_ccy_pair(&pair),
        hand::hand_ccy_pair_from_json(&pair),
    );
    let tenor = json!({ "unit": 3, "count": 1 });
    assert_decode_eq(
        "corpus/tenor",
        generated::decode_tenor(&tenor),
        hand::hand_tenor_from_json(&tenor),
    );
    let quantity = json!({ "notional": 1_000_000.0, "base_ccy": true });
    assert_decode_eq(
        "corpus/quantity",
        generated::decode_quantity(&quantity),
        hand::hand_quantity_from_json(&quantity),
    );
    let vanilla = json!({ "option_type": 0, "strike": { "strike": 1.12 } });
    assert_decode_eq(
        "corpus/vanilla",
        generated::decode_vanilla(&vanilla),
        hand::hand_vanilla_from_json(&vanilla),
    );
    let conventions = json!({
        "delta_convention": 0, "atm_convention": 0, "premium_style": 0,
        "cut": 0, "day_count": 0, "settlement": 0
    });
    assert_decode_eq(
        "corpus/conventions",
        generated::decode_conventions(&conventions),
        hand::hand_conventions_from_json(&conventions),
    );
}

// ===========================================================================
// INCREMENT 4 — the Instrument-consuming Price family: the full `Instrument`
// (24-arm `product` oneof) + the `PriceRequest` / `RatesPriceRequest` /
// `PriceXvaRequest` request envelopes (decode, byte-identical to the hand codec)
// and the `PriceResponse` / `RatesPriceResponse` / `PriceXvaResponse` /
// `Conventions` / `ArbReport` response surface (encode, byte-identical). This is
// the exact surface `handle_unary`'s price / price_rates / price_xva arms are
// swapped onto — proving the generated descriptor-driven codec is the byte-identical
// replacement over the price/rates/xva conformance corpus BEFORE (and continuously
// after) the swap.
// ===========================================================================

use celnet_proto::{
    ArbReport, Conventions, PriceResponse, PriceXvaResponse, RatesPriceResponse,
    RatesPricingResult, SmileModel, XvaResult,
};

/// Wrap a product body under its oneof key into a full instrument object carrying
/// the common leaf fields (the FX `pair` underlying, tenor, expiry, quantity, side,
/// solve, and the booking-model / settlement-style selectors) — the exact shape a
/// browser / Excel client sends. `decode_instrument` and the hand
/// `instrument_from_json` must build the byte-identical `Instrument` from it.
fn instrument_with(product_key: &str, product_body: Value) -> Value {
    let mut o = json!({
        "pair": { "base": "EUR", "quote": "USD" },
        "tenor": { "unit": 3, "count": 3 },
        "expiry_years": 0.25,
        "quantity": { "notional": 1_000_000.0, "base_ccy": true },
        "side": 1,
        "solve": { "target": 1, "target_premium": 0.021_4 },
        "pricing_model": 1,
        "settlement_style": 0,
    });
    o.as_object_mut()
        .expect("instrument object")
        .insert(product_key.to_owned(), product_body);
    o
}

/// One representative body per product arm, every field set to a non-default value
/// so the byte-identity comparison is meaningful across all 24 arms (the same wire
/// shapes the wave1/2/3 + linear + payoff conformance corpora exercise).
fn product_bodies() -> Vec<(&'static str, Value)> {
    let strike_or_delta = || json!({ "strike": { "strike": 1.082_53 } });
    let schedule = || json!({ "fixing_years": [0.25, 0.5, 0.75], "fixing_notional": 1_000.0 });
    vec![
        (
            "vanilla",
            json!({ "option_type": 0, "strike": { "strike": 1.1 } }),
        ),
        (
            "strategy",
            json!({
                "kind": 1,
                "legs": [
                    { "option_type": 0, "strike": { "delta": 0.25 }, "side": 1, "ratio": 1.0 },
                    { "option_type": 1, "strike": { "strike": 1.082_53 }, "side": 2, "ratio": 2.0 }
                ]
            }),
        ),
        (
            "single_barrier",
            json!({ "vanilla": strike_or_delta(), "kind": 1, "side": 1,
                    "barrier": 1.2, "rebate": 0.01, "monitoring": 1 }),
        ),
        (
            "double_barrier",
            json!({ "vanilla": strike_or_delta(), "kind": 1, "lower_barrier": 0.9,
                    "upper_barrier": 1.3, "rebate": 0.01, "monitoring": 1 }),
        ),
        (
            "digital",
            json!({ "option_type": 0, "strike": 1.1, "style": 1, "payout": 1.0 }),
        ),
        (
            "touch",
            json!({ "kind": 1, "lower_barrier": 0.9, "upper_barrier": 1.3,
                    "rebate": 0.5, "monitoring": 1 }),
        ),
        ("variance_swap", json!({ "strike_vol": 0.1 })),
        ("volatility_swap", json!({ "strike_vol": 0.1 })),
        (
            "asian_option",
            json!({ "option_type": 0, "strike": 1.1, "averaging": 1, "observations": 12,
                    "method": 1, "elapsed_avg": 1.05, "elapsed_weight": 0.5 }),
        ),
        (
            "forward_start",
            json!({ "option_type": 0, "moneyness": 1.0, "reset": 0.25 }),
        ),
        (
            "cliquet",
            json!({ "option_type": 0, "moneyness": 1.0, "periods": 4, "local_floor": -0.02,
                    "local_cap": 0.05, "global_floor": 0.0, "global_cap": 0.2,
                    "mc_pairs": 1_000, "mc_seed": 42 }),
        ),
        (
            "quanto",
            json!({ "payoff": 1, "option_type": 0, "strike": 1.1,
                    "conversion_vol": 0.08, "correlation": 0.3 }),
        ),
        (
            "tarf",
            json!({ "option_type": 0, "strike": 1.1, "target": 0.1, "leverage": 2.0,
                    "redemption": 1, "schedule": schedule(), "mc_pairs": 1_000, "mc_seed": 7 }),
        ),
        (
            "pivot",
            json!({ "option_type": 0, "strike": 1.1, "pivot": 1.05, "target": 0.1,
                    "leverage": 2.0, "redemption": 1, "schedule": schedule(),
                    "mc_pairs": 1_000, "mc_seed": 7 }),
        ),
        (
            "accumulator",
            json!({ "pivot": 1.05, "barrier": 1.2, "leverage": 2.0, "monitoring": 1,
                    "schedule": schedule(), "mc_pairs": 1_000, "mc_seed": 7 }),
        ),
        (
            "lookback",
            json!({ "style": 1, "option_type": 0, "monitoring": 1, "strike": 1.1,
                    "observations": 50, "mc_pairs": 1_000, "mc_seed": 7 }),
        ),
        (
            "window_barrier",
            json!({ "vanilla": strike_or_delta(), "barrier": 1.2, "side": 1,
                    "window_start": 0.1, "window_end": 0.2, "mc_pairs": 1_000,
                    "mc_steps": 100, "mc_seed": 7 }),
        ),
        (
            "american",
            json!({ "option_type": 0, "strike": 1.1, "exercise_style": 1,
                    "bermudan_dates": [0.1, 0.2, 0.3], "lsm_paths": 10_000,
                    "lsm_exercise_dates": 50, "lsm_seed": 7 }),
        ),
        (
            "basket",
            json!({
                "legs": [
                    { "pair": { "base": "EUR", "quote": "USD" }, "weight": 0.5,
                      "spot": 1.1, "vol": 0.1, "r_for": 0.01 },
                    { "pair": { "base": "GBP", "quote": "USD" }, "weight": 0.5,
                      "spot": 1.27, "vol": 0.12, "r_for": 0.02 }
                ],
                "correlations": [1.0, 0.3, 0.3, 1.0],
                "option_type": 0, "strike": 1.1, "kind": 1,
                "mc_paths": 10_000, "mc_replications": 10, "mc_steps": 100, "mc_seed": 7
            }),
        ),
        (
            "fx_forward",
            json!({ "contract_rate": 1.1, "notional": 1_000_000.0, "side": 0 }),
        ),
        (
            "fx_swap",
            json!({
                "near": { "contract_rate": 1.1, "notional": 1_000_000.0, "side": 0 },
                "far": { "contract_rate": 1.12, "notional": 1_000_000.0, "side": 1 }
            }),
        ),
        (
            "ndf",
            json!({ "contract_rate": 1.1, "notional": 1_000_000.0, "side": 0,
                    "fixing": 1, "settlement_ccy": "USD" }),
        ),
        (
            "perpetual_option",
            json!({ "option_type": 0, "strike": 1.1, "notional": 1_000_000.0 }),
        ),
        (
            "listed_future_option",
            json!({ "future_symbol": { "ticker": "CL", "venue": "NYMEX" },
                    "future_expiry_years": 0.5, "option_type": 0, "strike": 80.0,
                    "notional": 1_000.0, "margining": 1 }),
        ),
    ]
}

#[test]
fn instrument_all_24_product_arms_decode_byte_identical() {
    let bodies = product_bodies();
    assert_eq!(bodies.len(), 24, "every product arm must be covered");
    for (key, body) in bodies {
        let v = instrument_with(key, body);
        assert_decode_eq(
            &format!("Instrument(product/{key})"),
            generated::decode_instrument(&v),
            hand::hand_instrument_from_json(&v),
        );
    }
}

#[test]
fn instrument_cross_asset_underlying_decode_byte_identical() {
    // The dual-key `underlying` (the richer cross-asset oneof) takes precedence over
    // the legacy `pair`; both codecs route it to the same asset-class arm.
    let cases = [
        ("metal", json!({ "metal": { "metal": 1, "quote": "USD" } })),
        (
            "equity",
            json!({ "equity": { "symbol": { "ticker": "AAPL", "venue": "XNAS" }, "currency": "USD" } }),
        ),
        (
            "commodity",
            json!({ "commodity": { "symbol": { "ticker": "CL", "venue": "NYMEX" }, "currency": "USD" } }),
        ),
        (
            "digital_asset",
            json!({ "digital_asset": { "base": "BTC", "quote": "USDT" } }),
        ),
    ];
    for (arm, underlying) in cases {
        let mut v = json!({
            "expiry_years": 0.5,
            "vanilla": { "option_type": 0, "strike": { "strike": 1.1 } },
        });
        v.as_object_mut()
            .unwrap()
            .insert("underlying".to_owned(), underlying);
        assert_decode_eq(
            &format!("Instrument(underlying/{arm})"),
            generated::decode_instrument(&v),
            hand::hand_instrument_from_json(&v),
        );
    }
}

/// A representative full `PriceRequest` corpus body: a real vanilla instrument, FX
/// market, conventions and the presence-tracked ids.
fn price_request_body() -> Value {
    json!({
        "request_id": 7,
        "instrument": instrument_with("vanilla", json!({ "option_type": 0, "strike": { "strike": 1.1 } })),
        "market": { "spot": 1.082_53, "vol": 0.091_25, "r_dom": 0.042_10, "r_for": 0.018_70 },
        "conventions": {
            "delta_convention": 1, "atm_convention": 2, "premium_style": 1,
            "cut": 3, "day_count": 1, "settlement": 1
        },
        "correlation_id": 123,
        "surface_version": 5
    })
}

#[test]
fn price_request_envelope_decode_byte_identical() {
    let body = price_request_body();
    let o = body.as_object().expect("price request object");
    assert_decode_eq(
        "PriceRequest",
        generated::decode_price_request(o),
        hand::hand_price_request_from_json(o),
    );
    // The correlation_id / surface_version omitted ⇒ `None` on both paths.
    let mut minimal = price_request_body();
    let m = minimal.as_object_mut().unwrap();
    m.remove("correlation_id");
    m.remove("surface_version");
    assert_decode_eq(
        "PriceRequest(minimal ids)",
        generated::decode_price_request(minimal.as_object().unwrap()),
        hand::hand_price_request_from_json(minimal.as_object().unwrap()),
    );
}

#[test]
fn rates_price_request_envelope_decode_byte_identical() {
    let body = json!({
        "request_id": 3,
        "curve_set": {
            "currency": "USD",
            "reference_date": { "year": 2026, "month": 6, "day": 30 },
            "ois_pillars": [
                { "tenor": { "months": 3 }, "par_rate": 0.030 },
                { "tenor": { "years": 1 }, "par_rate": 0.035 },
                { "tenor": { "maturity_date": { "year": 2031, "month": 6, "day": 30 } }, "par_rate": 0.041 }
            ]
        },
        "instrument": { "ois": { "tenor_years": 5, "fixed_rate": 0.033, "notional": 10_000_000.0, "side": 0 } },
        "correlation_id": 9
    });
    let o = body.as_object().expect("rates request object");
    assert_decode_eq(
        "RatesPriceRequest",
        generated::decode_rates_price_request(o),
        hand::hand_rates_price_request_from_json(o),
    );
}

#[test]
fn rates_price_request_new_arms_decode_byte_identical() {
    // The IRS / FRA / cash-bond arms decode through the generic descriptor-driven
    // rates tree byte-identically to the hand oracle — every field, including the
    // new PaymentFrequency / DayCount / AccrualBasis enum selectors and the bond's
    // nested `maturity_date` message.
    let curve_set = json!({
        "currency": "USD",
        "reference_date": { "year": 2026, "month": 6, "day": 30 },
        "ois_pillars": [
            { "tenor": { "years": 1 }, "par_rate": 0.043 },
            { "tenor": { "years": 2 }, "par_rate": 0.0418 },
            { "tenor": { "years": 5 }, "par_rate": 0.0405 },
            { "tenor": { "years": 10 }, "par_rate": 0.0415 }
        ]
    });
    let arms = [
        (
            "irs",
            json!({ "irs": {
                "tenor_years": 5, "fixed_rate": 0.041, "notional": 100_000_000.0, "side": 1,
                "fixed_frequency": 1, "fixed_day_count": 1,
                "float_frequency": 2, "float_day_count": 1
            }}),
        ),
        (
            "fra",
            json!({ "fra": {
                "start_months": 3, "end_months": 6, "fixed_rate": 0.033,
                "notional": 25_000_000.0, "side": 1, "accrual_basis": 0
            }}),
        ),
        (
            "bond",
            json!({ "bond": {
                "coupon_rate": 0.06, "coupon_frequency": 1, "day_count": 2,
                "maturity_date": { "year": 2035, "month": 6, "day": 15 },
                "redemption": 100.0, "side": 0
            }}),
        ),
    ];
    for (arm, instrument) in arms {
        let body = json!({
            "request_id": 3,
            "curve_set": curve_set.clone(),
            "instrument": instrument,
            "correlation_id": 9
        });
        let o = body.as_object().expect("rates request object");
        assert_decode_eq(
            &format!("RatesPriceRequest({arm})"),
            generated::decode_rates_price_request(o),
            hand::hand_rates_price_request_from_json(o),
        );
    }
}

#[test]
fn price_xva_request_envelope_decode_byte_identical() {
    let body = json!({
        "request_id": 4,
        "trades": [
            { "option_type": 0, "strike": 1.1, "expiry_years": 1.0, "vol": 0.1, "notional": 1_000_000.0 },
            { "option_type": 1, "strike": 1.05, "expiry_years": 2.0, "vol": 0.12, "notional": 500_000.0 }
        ],
        "r_dom": 0.02, "r_for": 0.01, "spot0": 1.1, "sigma": 0.1,
        "paths": 20_000, "seed": 7, "exposure_steps": 50,
        "counterparty": { "pillar_times": [1.0, 2.0], "hazard_rates": [0.01, 0.02] },
        "own": { "pillar_times": [], "hazard_rates": [0.005] },
        "lgd_counterparty": 0.6, "lgd_own": 0.4, "funding_spread": 0.005,
        "correlation_id": 11
    });
    let o = body.as_object().expect("xva request object");
    assert_decode_eq(
        "PriceXvaRequest",
        generated::decode_price_xva_request(o),
        hand::hand_price_xva_request_from_json(o),
    );
}

// --- response encode byte-identity ------------------------------------------

#[test]
fn conventions_encode_is_byte_identical() {
    let c = Conventions {
        delta_convention: 1,
        atm_convention: 2,
        premium_style: 1,
        cut: 3,
        day_count: 1,
        settlement: 1,
    };
    assert_bytes_eq(
        "Conventions",
        &generated::encode_conventions(&c),
        &hand::hand_conventions_to_json(&c),
    );
}

#[test]
fn price_response_encode_is_byte_identical() {
    let conv = Conventions {
        delta_convention: 1,
        atm_convention: 0,
        premium_style: 1,
        cut: 0,
        day_count: 1,
        settlement: 0,
    };
    // Fully-populated: Greeks strip + resolved strike + conventions + all ids.
    let full = PriceResponse {
        request_id: 7,
        greeks: Some(greeks_with(Some(RateSensitivities::fx(880.5, -410.25)))),
        resolved_strike: 1.082_53,
        conventions: Some(conv),
        correlation_id: Some(123),
        surface_version: Some(5),
        price_std_error: Some(0.000_25),
    };
    assert_bytes_eq(
        "PriceResponse(full)",
        &generated::encode_price_response(&full),
        &hand::hand_price_response_to_json(&full),
    );
    // Absent presence-tracked fields: greeks/conventions ⇒ JSON null (singular
    // message), correlation_id/surface_version/price_std_error ⇒ JSON null
    // (proto3-optional scalars — the response-message null policy).
    let empty = PriceResponse {
        request_id: 8,
        greeks: None,
        resolved_strike: 0.0,
        conventions: None,
        correlation_id: None,
        surface_version: None,
        price_std_error: None,
    };
    let g = generated::encode_price_response(&empty);
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_eq!(g.get("surface_version"), Some(&Value::Null));
    assert_eq!(g.get("price_std_error"), Some(&Value::Null));
    assert_eq!(g.get("greeks"), Some(&Value::Null));
    assert_eq!(g.get("conventions"), Some(&Value::Null));
    assert_bytes_eq(
        "PriceResponse(empty)",
        &g,
        &hand::hand_price_response_to_json(&empty),
    );
}

#[test]
fn rates_price_response_encode_is_byte_identical() {
    let full = RatesPriceResponse {
        request_id: 3,
        result: Some(RatesPricingResult {
            pv: 12_345.67,
            par_rate: 0.033,
            pv01: 98.7,
            dv01: 987.6,
            key_rate_ladder: vec![10.0, 20.5, -5.25, 0.0],
        }),
        correlation_id: Some(9),
    };
    assert_bytes_eq(
        "RatesPriceResponse(full)",
        &generated::encode_rates_price_response(&full),
        &hand::hand_rates_price_response_to_json(&full),
    );
    let empty = RatesPriceResponse {
        request_id: 4,
        result: None,
        correlation_id: None,
    };
    let g = generated::encode_rates_price_response(&empty);
    assert_eq!(g.get("result"), Some(&Value::Null));
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "RatesPriceResponse(empty)",
        &g,
        &hand::hand_rates_price_response_to_json(&empty),
    );
}

#[test]
fn price_xva_response_encode_is_byte_identical() {
    let full = PriceXvaResponse {
        request_id: 5,
        result: Some(XvaResult {
            cva: 1_234.5,
            dva: -678.25,
            fva: 90.1,
            total_adjustment: 646.35,
        }),
        correlation_id: Some(11),
    };
    assert_bytes_eq(
        "PriceXvaResponse(full)",
        &generated::encode_price_xva_response(&full),
        &hand::hand_price_xva_response_to_json(&full),
    );
    let empty = PriceXvaResponse {
        request_id: 6,
        result: None,
        correlation_id: None,
    };
    let g = generated::encode_price_xva_response(&empty);
    assert_eq!(g.get("result"), Some(&Value::Null));
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "PriceXvaResponse(empty)",
        &g,
        &hand::hand_price_xva_response_to_json(&empty),
    );
}

#[test]
fn arb_report_encode_is_byte_identical_incl_synthetic_label() {
    // Every SmileModel tag (0..=4) plus an out-of-range tag exercises the
    // synthesized `smile_model_label` override end-to-end.
    let tags = [
        SmileModel::MarketHedge as i32,
        SmileModel::StochasticVol as i32,
        SmileModel::Parametric as i32,
        SmileModel::ParametricSurface as i32,
        SmileModel::ExtendedSurface as i32,
        99, // unknown ⇒ "unknown"
    ];
    for tag in tags {
        let a = ArbReport {
            butterfly_arbitrage_free: true,
            calendar_arbitrage_free: false,
            worst_density: -0.001_25,
            note: "calibrated; model=parametric-surface".to_owned(),
            smile_model: tag,
        };
        assert_bytes_eq(
            &format!("ArbReport(smile_model={tag})"),
            &generated::encode_arb_report(&a),
            &hand::hand_arb_report_to_json(&a),
        );
    }
}

// ===========================================================================
// FixAdminService — the six connection/message administration verbs: the request
// decoders (incl. the shared `EntitlementPrincipal` envelope + the quirked
// `ListFixMessagesRequest` projection) and the response encoders (incl. the
// `correlation_id`-as-`null` policy), byte-identical to the hand codec over the
// admin conformance shapes + edge vectors.
// ===========================================================================

use celnet_proto::{
    CreateFixConnectionResponse, DeleteFixConnectionResponse, FixConnectionDesc, FixMessage,
    ListFixConnectionsResponse, ListFixMessagesResponse, SetFixConnectionEnabledResponse,
    UpdateFixConnectionResponse,
};

/// A representative admin entitlement principal body (grant/deny rules with pinned
/// dimension scopes) — the nested tree every fix-admin request can carry.
fn principal_body() -> Value {
    json!({
        "grant_all": false,
        "grants": [
            { "scopes": [ { "dimension": 1, "value": 99 }, { "dimension": 2, "value": 7 } ] },
            { "scopes": [] }
        ],
        "denies": [ { "scopes": [ { "dimension": 3, "value": 4 } ] } ]
    })
}

/// A representative editable connection spec body.
fn fix_spec_body() -> Value {
    json!({
        "id": "lp-one", "name": "LP One", "kind": 1, "bind_addr": "127.0.0.1:9099",
        "sender_comp_id": "CELNET", "target_comp_id": "LPONE", "enabled": true, "desk": "fx-desk"
    })
}

#[test]
fn list_fix_connections_request_decode_byte_identical() {
    let cases = [
        (
            "full",
            json!({ "principal": principal_body(), "correlation_id": 42, "session_token": "tok-abc" }),
        ),
        // Minimal: all optionals absent ⇒ None / grant-all-default principal absent.
        ("minimal", json!({})),
        // Empty session token ⇒ None (the hand `opt_string` empty-filter, quirk).
        ("empty-session", json!({ "session_token": "" })),
    ];
    for (label, body) in cases {
        let o = body.as_object().expect("object");
        assert_decode_eq(
            &format!("ListFixConnectionsRequest({label})"),
            generated::decode_list_fix_connections_request(o),
            hand::hand_list_fix_connections_request_from_json(o),
        );
    }
}

#[test]
fn create_fix_connection_request_decode_byte_identical() {
    let body = json!({
        "spec": fix_spec_body(), "principal": principal_body(),
        "correlation_id": 7, "session_token": "tok"
    });
    let o = body.as_object().expect("object");
    assert_decode_eq(
        "CreateFixConnectionRequest",
        generated::decode_create_fix_connection_request(o),
        hand::hand_create_fix_connection_request_from_json(o),
    );
    // Spec-only (no principal / ids): the required `spec` with proto3-default id/desk.
    let minimal = json!({ "spec": { "name": "LP Two", "bind_addr": "0.0.0.0:0",
        "sender_comp_id": "C", "target_comp_id": "L" } });
    let mo = minimal.as_object().expect("object");
    assert_decode_eq(
        "CreateFixConnectionRequest(minimal spec)",
        generated::decode_create_fix_connection_request(mo),
        hand::hand_create_fix_connection_request_from_json(mo),
    );
}

#[test]
fn update_fix_connection_request_decode_byte_identical() {
    let body = json!({
        "id": "lp-one", "spec": fix_spec_body(), "principal": principal_body(),
        "correlation_id": 8, "session_token": "tok"
    });
    let o = body.as_object().expect("object");
    assert_decode_eq(
        "UpdateFixConnectionRequest",
        generated::decode_update_fix_connection_request(o),
        hand::hand_update_fix_connection_request_from_json(o),
    );
}

#[test]
fn delete_fix_connection_request_decode_byte_identical() {
    let body = json!({ "id": "lp-one", "principal": principal_body(), "correlation_id": 3 });
    let o = body.as_object().expect("object");
    assert_decode_eq(
        "DeleteFixConnectionRequest",
        generated::decode_delete_fix_connection_request(o),
        hand::hand_delete_fix_connection_request_from_json(o),
    );
}

#[test]
fn set_fix_connection_enabled_request_decode_byte_identical() {
    for (label, body) in [
        (
            "disable",
            json!({ "id": "lp-one", "enabled": false, "session_token": "tok" }),
        ),
        (
            "enable",
            json!({ "id": "lp-one", "enabled": true, "correlation_id": 5 }),
        ),
    ] {
        let o = body.as_object().expect("object");
        assert_decode_eq(
            &format!("SetFixConnectionEnabledRequest({label})"),
            generated::decode_set_fix_connection_enabled_request(o),
            hand::hand_set_fix_connection_enabled_request_from_json(o),
        );
    }
}

#[test]
fn list_fix_messages_request_decode_byte_identical_incl_quirks() {
    let cases = [
        (
            "full",
            json!({ "connection_id": "lp-one", "after_seq": 100, "limit": 50, "correlation_id": 9 }),
        ),
        // Whitespace-only connection_id ⇒ None (the hand `.trim().is_empty()` quirk).
        ("whitespace-conn", json!({ "connection_id": "   " })),
        // `limit` beyond u32::MAX saturates to u32::MAX (the hand `.unwrap_or(u32::MAX)` quirk).
        ("limit-overflow", json!({ "limit": 99_999_999_999_u64 })),
        // Absent cursor/limit ⇒ after_seq 0, limit 0 (server default), connection_id None.
        ("defaults", json!({})),
    ];
    for (label, body) in cases {
        let o = body.as_object().expect("object");
        assert_decode_eq(
            &format!("ListFixMessagesRequest({label})"),
            generated::decode_list_fix_messages_request(o),
            hand::hand_list_fix_messages_request_from_json(o),
        );
    }
}

/// A fully-populated connection descriptor (runtime status included).
fn fix_conn_desc() -> FixConnectionDesc {
    FixConnectionDesc {
        id: "lp-one".to_owned(),
        name: "LP One".to_owned(),
        kind: 1,
        bind_addr: "127.0.0.1:9099".to_owned(),
        sender_comp_id: "CELNET".to_owned(),
        target_comp_id: "LPONE".to_owned(),
        enabled: true,
        running: true,
        bound_addr: "127.0.0.1:9099".to_owned(),
        desk: "fx-desk".to_owned(),
    }
}

#[test]
fn list_fix_connections_response_encode_byte_identical() {
    let full = ListFixConnectionsResponse {
        connections: vec![fix_conn_desc(), FixConnectionDesc::default()],
        correlation_id: Some(42),
    };
    assert_bytes_eq(
        "ListFixConnectionsResponse(full)",
        &generated::encode_list_fix_connections_response(&full),
        &hand::hand_list_fix_connections_response_to_json(&full),
    );
    // Empty connections + absent correlation_id ⇒ `[]` + `null`.
    let empty = ListFixConnectionsResponse {
        connections: vec![],
        correlation_id: None,
    };
    let g = generated::encode_list_fix_connections_response(&empty);
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_eq!(g.get("connections"), Some(&Value::Array(vec![])));
    assert_bytes_eq(
        "ListFixConnectionsResponse(empty)",
        &g,
        &hand::hand_list_fix_connections_response_to_json(&empty),
    );
}

#[test]
fn create_update_setenabled_response_encode_byte_identical() {
    // Present connection + id.
    let created = CreateFixConnectionResponse {
        connection: Some(fix_conn_desc()),
        correlation_id: Some(7),
    };
    assert_bytes_eq(
        "CreateFixConnectionResponse(full)",
        &generated::encode_create_fix_connection_response(&created),
        &hand::hand_create_fix_connection_response_to_json(&created),
    );
    // Absent connection + id ⇒ both `null` (singular-message + optional-scalar null).
    let empty = CreateFixConnectionResponse {
        connection: None,
        correlation_id: None,
    };
    let g = generated::encode_create_fix_connection_response(&empty);
    assert_eq!(g.get("connection"), Some(&Value::Null));
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "CreateFixConnectionResponse(empty)",
        &g,
        &hand::hand_create_fix_connection_response_to_json(&empty),
    );

    let updated = UpdateFixConnectionResponse {
        connection: Some(fix_conn_desc()),
        correlation_id: Some(8),
    };
    assert_bytes_eq(
        "UpdateFixConnectionResponse",
        &generated::encode_update_fix_connection_response(&updated),
        &hand::hand_update_fix_connection_response_to_json(&updated),
    );

    let toggled = SetFixConnectionEnabledResponse {
        connection: Some(fix_conn_desc()),
        correlation_id: None,
    };
    assert_bytes_eq(
        "SetFixConnectionEnabledResponse(null corr)",
        &generated::encode_set_fix_connection_enabled_response(&toggled),
        &hand::hand_set_fix_connection_enabled_response_to_json(&toggled),
    );
}

#[test]
fn delete_fix_connection_response_encode_byte_identical() {
    for (label, correlation_id) in [("with-id", Some(3_u64)), ("null-id", None)] {
        let r = DeleteFixConnectionResponse { correlation_id };
        assert_bytes_eq(
            &format!("DeleteFixConnectionResponse({label})"),
            &generated::encode_delete_fix_connection_response(&r),
            &hand::hand_delete_fix_connection_response_to_json(&r),
        );
    }
}

#[test]
fn list_fix_messages_response_encode_byte_identical() {
    let full = ListFixMessagesResponse {
        messages: vec![
            FixMessage {
                seq: 1,
                connection_id: "lp-one".to_owned(),
                direction: 0,
                msg_type: "R".to_owned(),
                summary: "QuoteRequest".to_owned(),
                epoch_nanos: 1_720_000_000_000_000_000,
                raw: "8=FIX.4.4|35=R|".to_owned(),
            },
            FixMessage {
                seq: 2,
                connection_id: "lp-one".to_owned(),
                direction: 1,
                msg_type: "S".to_owned(),
                summary: "Quote".to_owned(),
                epoch_nanos: 1_720_000_000_500_000_000,
                raw: "8=FIX.4.4|35=S|".to_owned(),
            },
        ],
        latest_seq: 2,
        correlation_id: Some(9),
    };
    assert_bytes_eq(
        "ListFixMessagesResponse(full)",
        &generated::encode_list_fix_messages_response(&full),
        &hand::hand_list_fix_messages_response_to_json(&full),
    );
    let empty = ListFixMessagesResponse {
        messages: vec![],
        latest_seq: 0,
        correlation_id: None,
    };
    let g = generated::encode_list_fix_messages_response(&empty);
    assert_eq!(g.get("messages"), Some(&Value::Array(vec![])));
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "ListFixMessagesResponse(empty)",
        &g,
        &hand::hand_list_fix_messages_response_to_json(&empty),
    );
}

// ===========================================================================
// QuoteService — the RFQ lifecycle: the request decoders (incl. the FX-legacy
// `QuoteRequest.instrument` projection + the camelCase attribution tree) and the
// reply encoders (`Quote` / `MultiDealerQuote` / `Execution` / `RejectAck`),
// byte-identical to the hand codec incl. the presence-tracked-null policy, the
// camelCase attribution keys + omit-absent behavior, and the suppressed
// `Execution.instrument`.
// ===========================================================================

use celnet_proto::{
    AttributionRecord, BookId, Conventions as Conv, DealerQuote, Execution, MultiDealerQuote,
    Owner, Quote, RejectAck, Side as QSide, TwoWayPrice, owner,
};

/// A representative Conventions block.
fn quote_conv() -> Conv {
    Conv {
        delta_convention: 1,
        atm_convention: 2,
        premium_style: 1,
        cut: 3,
        day_count: 1,
        settlement: 1,
    }
}

/// A fully-populated who's-trading attribution chain (both books + won + lp_count).
fn attribution_full() -> AttributionRecord {
    AttributionRecord {
        quoted_by: Some(BookId {
            book: "EM-VOL-1".to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::Trader("alice".to_owned())),
            }),
        }),
        held_by: Some(BookId {
            book: "EM-VOL-2".to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::AutoPricer("pricer-x".to_owned())),
            }),
        }),
        won: Some(true),
        lp_count: Some(3),
    }
}

/// The camelCase attribution JSON body a GUI/Excel client sends.
fn attribution_body() -> Value {
    json!({
        "quotedBy": { "book": "EM-VOL-1", "owner": { "trader": "alice" } },
        "heldBy": { "book": "EM-VOL-2", "owner": { "autoPricer": "pricer-x" } },
        "won": true, "lpCount": 3
    })
}

fn conventions_body() -> Value {
    json!({
        "delta_convention": 1, "atm_convention": 2, "premium_style": 1,
        "cut": 3, "day_count": 1, "settlement": 1
    })
}

#[test]
fn quote_request_decode_byte_identical() {
    let full = {
        let mut o = json!({
            "idempotency_key": "idem-1",
            "instrument": instrument_with("vanilla", json!({ "option_type": 0, "strike": { "strike": 1.1 } })),
            "conventions": conventions_body(),
            "correlation_id": 7,
            "surface_version": 5,
            "session_token": "tok-abc",
        });
        o.as_object_mut()
            .unwrap()
            .insert("attribution".to_owned(), attribution_body());
        o
    };
    let o = full.as_object().expect("object");
    assert_decode_eq(
        "QuoteRequest(full)",
        generated::decode_quote_request(o),
        hand::hand_quote_request_from_json(o),
    );
    // Minimal: only the required fields (idempotency_key + instrument + conventions).
    let minimal = json!({
        "idempotency_key": "idem-2",
        "instrument": instrument_with("vanilla", json!({ "option_type": 1, "strike": { "delta": 0.25 } })),
        "conventions": conventions_body(),
    });
    let mo = minimal.as_object().expect("object");
    assert_decode_eq(
        "QuoteRequest(minimal)",
        generated::decode_quote_request(mo),
        hand::hand_quote_request_from_json(mo),
    );
    // Partial attribution: quoted_by only (held_by / won / lp_count absent ⇒ omitted).
    let partial = json!({
        "idempotency_key": "idem-3",
        "instrument": instrument_with("vanilla", json!({ "option_type": 0, "strike": { "strike": 1.2 } })),
        "conventions": conventions_body(),
        "attribution": { "quotedBy": { "book": "B", "owner": { "trader": "bob" } } },
    });
    let po = partial.as_object().expect("object");
    assert_decode_eq(
        "QuoteRequest(partial attribution)",
        generated::decode_quote_request(po),
        hand::hand_quote_request_from_json(po),
    );
}

#[test]
fn quote_accept_reject_decode_byte_identical() {
    let accept = json!({
        "quote_id": 42, "idempotency_key": "idem-1", "side": 1, "lp_id": "LP-A",
        "session_token": "tok", "principal": principal_body()
    });
    let ao = accept.as_object().expect("object");
    assert_decode_eq(
        "QuoteAccept(full)",
        generated::decode_quote_accept(ao),
        hand::hand_quote_accept_from_json(ao),
    );
    let accept_min = json!({ "quote_id": 7 });
    let amo = accept_min.as_object().expect("object");
    assert_decode_eq(
        "QuoteAccept(minimal)",
        generated::decode_quote_accept(amo),
        hand::hand_quote_accept_from_json(amo),
    );

    let reject = json!({ "quote_id": 42, "reason": "too wide", "session_token": "tok" });
    let ro = reject.as_object().expect("object");
    assert_decode_eq(
        "QuoteReject(full)",
        generated::decode_quote_reject(ro),
        hand::hand_quote_reject_from_json(ro),
    );
    let reject_min = json!({ "quote_id": 8 });
    let rmo = reject_min.as_object().expect("object");
    assert_decode_eq(
        "QuoteReject(minimal)",
        generated::decode_quote_reject(rmo),
        hand::hand_quote_reject_from_json(rmo),
    );
}

#[test]
fn quote_encode_byte_identical() {
    let full = Quote {
        quote_id: 42,
        idempotency_key: "idem-1".to_owned(),
        price: Some(TwoWayPrice {
            bid: 0.021,
            offer: 0.023,
        }),
        greeks: Some(greeks_with(Some(RateSensitivities::fx(880.5, -410.25)))),
        conventions: Some(quote_conv()),
        resolved_strike: 1.082_53,
        epoch_nanos: 1_720_000_000_000_000_000,
        valid_until_nanos: 1_720_000_030_000_000_000,
        correlation_id: Some(7),
        surface_version: Some(5),
        attribution: Some(attribution_full()),
        price_std_error: Some(0.000_25),
    };
    assert_bytes_eq(
        "Quote(full)",
        &generated::encode_quote(&full),
        &hand::hand_quote_to_json(&full),
    );
    // Absent presence-tracked fields ⇒ JSON null (the `json!({ .. })` reply policy).
    let empty = Quote {
        quote_id: 1,
        idempotency_key: "x".to_owned(),
        price: None,
        greeks: None,
        conventions: None,
        resolved_strike: 0.0,
        epoch_nanos: 0,
        valid_until_nanos: 0,
        correlation_id: None,
        surface_version: None,
        attribution: None,
        price_std_error: None,
    };
    let g = generated::encode_quote(&empty);
    for key in [
        "price",
        "greeks",
        "conventions",
        "correlation_id",
        "surface_version",
        "attribution",
        "price_std_error",
    ] {
        assert_eq!(g.get(key), Some(&Value::Null), "`{key}` must be null");
    }
    assert_bytes_eq("Quote(empty)", &g, &hand::hand_quote_to_json(&empty));
}

#[test]
fn quote_attribution_partial_omits_absent_fields() {
    // A quoted_by-only attribution: the manual-`Map` attribution encoder omits the
    // absent `heldBy` / `won` / `lpCount`, and the book's absent `owner` is omitted.
    let q = Quote {
        quote_id: 9,
        idempotency_key: "y".to_owned(),
        price: None,
        greeks: None,
        conventions: None,
        resolved_strike: 0.0,
        epoch_nanos: 0,
        valid_until_nanos: 0,
        correlation_id: None,
        surface_version: None,
        attribution: Some(AttributionRecord {
            quoted_by: Some(BookId {
                book: "B".to_owned(),
                owner: None,
            }),
            held_by: None,
            won: None,
            lp_count: None,
        }),
        price_std_error: None,
    };
    let g = generated::encode_quote(&q);
    let attr = g.get("attribution").expect("attribution present");
    assert!(attr.get("quotedBy").is_some());
    assert!(attr.get("heldBy").is_none(), "absent heldBy omitted");
    assert!(attr.get("won").is_none(), "absent won omitted");
    assert!(attr.get("lpCount").is_none(), "absent lpCount omitted");
    assert!(
        attr.get("quotedBy").and_then(|b| b.get("owner")).is_none(),
        "absent owner omitted from the book"
    );
    assert_bytes_eq(
        "Quote(partial attribution)",
        &g,
        &hand::hand_quote_to_json(&q),
    );
}

#[test]
fn multi_dealer_quote_encode_byte_identical() {
    let dealer = |lp: &str, bid: f64| DealerQuote {
        lp_id: lp.to_owned(),
        price: Some(TwoWayPrice {
            bid,
            offer: bid + 0.002,
        }),
        greeks: Some(greeks_with(Some(RateSensitivities::fx(1.0, -0.5)))),
        resolved_strike: 1.1,
        valid_until_nanos: 1_720_000_030_000_000_000,
        attribution: Some(attribution_full()),
        price_std_error: None,
    };
    let full = MultiDealerQuote {
        quote_id: 100,
        idempotency_key: "idem-md".to_owned(),
        dealers: vec![dealer("LP-A", 0.020), dealer("LP-B", 0.019)],
        best_bid_lp_id: "LP-A".to_owned(),
        best_offer_lp_id: "LP-B".to_owned(),
        conventions: Some(quote_conv()),
        epoch_nanos: 1_720_000_000_000_000_000,
        correlation_id: Some(11),
        surface_version: Some(5),
    };
    assert_bytes_eq(
        "MultiDealerQuote(full)",
        &generated::encode_multi_dealer_quote(&full),
        &hand::hand_multi_dealer_quote_to_json(&full),
    );
    let empty = MultiDealerQuote {
        quote_id: 1,
        idempotency_key: "z".to_owned(),
        dealers: vec![],
        best_bid_lp_id: String::new(),
        best_offer_lp_id: String::new(),
        conventions: None,
        epoch_nanos: 0,
        correlation_id: None,
        surface_version: None,
    };
    let g = generated::encode_multi_dealer_quote(&empty);
    assert_eq!(g.get("dealers"), Some(&Value::Array(vec![])));
    assert_eq!(g.get("conventions"), Some(&Value::Null));
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "MultiDealerQuote(empty)",
        &g,
        &hand::hand_multi_dealer_quote_to_json(&empty),
    );
}

#[test]
fn execution_and_reject_ack_encode_byte_identical() {
    // The booked `Execution` NEVER serializes its `instrument` (suppressed override).
    let exec = Execution {
        execution_id: 500,
        quote_id: 42,
        side: QSide::Buy as i32,
        traded_premium: 0.022,
        instrument: Some(celnet_proto::Instrument {
            expiry_years: 0.25,
            ..Default::default()
        }),
        epoch_nanos: 1_720_000_000_000_000_000,
        attribution: Some(attribution_full()),
    };
    let g = generated::encode_execution(&exec);
    assert!(
        g.get("instrument").is_none(),
        "Execution.instrument must never be serialized: {g}"
    );
    assert_bytes_eq("Execution(full)", &g, &hand::hand_execution_to_json(&exec));
    // Absent attribution ⇒ null (json! policy); instrument still omitted.
    let exec0 = Execution {
        execution_id: 1,
        quote_id: 2,
        side: 0,
        traded_premium: 0.0,
        instrument: None,
        epoch_nanos: 0,
        attribution: None,
    };
    let g0 = generated::encode_execution(&exec0);
    assert_eq!(g0.get("attribution"), Some(&Value::Null));
    assert!(g0.get("instrument").is_none());
    assert_bytes_eq(
        "Execution(empty)",
        &g0,
        &hand::hand_execution_to_json(&exec0),
    );

    let ack = RejectAck {
        quote_id: 42,
        epoch_nanos: 1_720_000_000_000_000_000,
    };
    assert_bytes_eq(
        "RejectAck",
        &generated::encode_reject_ack(&ack),
        &hand::hand_reject_ack_to_json(&ack),
    );
}

// ===========================================================================
// WAVE 3 — SurfaceService: GetSmile / MarkSurface / Scenario. The request
// decoders (incl. the `smile_model` dual int/string enum, the `VegaBucket` /
// `CrossGamma` response-only-field hardcode, and the FX-legacy `ScenarioRequest`
// instrument/market projection) and the `Smile` / `MarkSurfaceResponse` /
// `ScenarioResponse` reply encoders — the exact surface `handle_unary`'s
// get_smile / mark_surface / scenario arms are swapped onto.
// ===========================================================================

use celnet_proto::{
    BrokerQuoteSet, BucketedRisk, CrossGamma, MarkSurfaceResponse, ScenarioPoint, ScenarioResponse,
    Smile, SmilePoint, VegaBucket,
};

/// The six-enum convention block a surface request carries.
fn conventions_json() -> Value {
    json!({
        "delta_convention": 0, "atm_convention": 0, "premium_style": 0,
        "cut": 0, "day_count": 0, "settlement": 0
    })
}

#[test]
fn get_smile_request_decode_is_byte_identical() {
    // With the pair present.
    let v = json!({
        "pair": { "base": "EUR", "quote": "USD" },
        "tenor_years": 0.25,
        "conventions": conventions_json(),
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "GetSmile(pair)",
        generated::decode_get_smile_request(o),
        hand::hand_get_smile_request_from_json(o),
    );
    // With the optional pair absent.
    let v0 = json!({ "tenor_years": 0.1, "conventions": conventions_json() });
    let o0 = v0.as_object().expect("object");
    assert_decode_eq(
        "GetSmile(no-pair)",
        generated::decode_get_smile_request(o0),
        hand::hand_get_smile_request_from_json(o0),
    );
}

#[test]
fn mark_surface_request_decode_is_byte_identical() {
    let five_point = json!({
        "tenor_years": 0.25, "atm_vol": 0.09, "rr_25": -0.01, "bf_25": 0.002,
        "rr_10": -0.02, "bf_10": 0.004, "has_ten_delta": true
    });
    // A three-point set omitting the 10Δ wings (absent optionals default to 0.0).
    let three_point = json!({ "tenor_years": 0.5, "atm_vol": 0.085 });
    // `smile_model` as the enum integer.
    let v_int = json!({
        "pair": { "base": "EUR", "quote": "USD" },
        "broker_quotes": [five_point.clone(), three_point.clone()],
        "conventions": conventions_json(),
        "smile_model": 2,
    });
    let oi = v_int.as_object().expect("object");
    assert_decode_eq(
        "MarkSurface(smile_model=int)",
        generated::decode_mark_surface_request(oi),
        hand::hand_mark_surface_request_from_json(oi),
    );
    // `smile_model` as its `SMILE_MODEL_*` string name (the dual-decode quirk).
    let v_str = json!({
        "broker_quotes": [five_point.clone()],
        "conventions": conventions_json(),
        "smile_model": "SMILE_MODEL_PARAMETRIC_SURFACE",
    });
    let os = v_str.as_object().expect("object");
    assert_decode_eq(
        "MarkSurface(smile_model=str)",
        generated::decode_mark_surface_request(os),
        hand::hand_mark_surface_request_from_json(os),
    );
    // A string the client cannot select (a server-internal repair family) ⇒ None.
    let v_unknown = json!({
        "broker_quotes": [three_point.clone()],
        "conventions": conventions_json(),
        "smile_model": "SMILE_MODEL_EXTENDED_SURFACE",
    });
    let ou = v_unknown.as_object().expect("object");
    assert_decode_eq(
        "MarkSurface(smile_model=unknown-str)",
        generated::decode_mark_surface_request(ou),
        hand::hand_mark_surface_request_from_json(ou),
    );
    // `smile_model` absent ⇒ None (server default).
    let v_none = json!({
        "broker_quotes": [five_point],
        "conventions": conventions_json(),
    });
    let on = v_none.as_object().expect("object");
    assert_decode_eq(
        "MarkSurface(no-smile_model)",
        generated::decode_mark_surface_request(on),
        hand::hand_mark_surface_request_from_json(on),
    );
}

#[test]
fn scenario_request_decode_is_byte_identical() {
    let inst = instrument_with(
        "vanilla",
        json!({ "option_type": 0, "strike": { "strike": 1.12 } }),
    );
    // Full: an FX-legacy instrument + market, a two-axis grid, and the book-shaped
    // risk buckets whose response-only `vega`/`value` fields must decode to 0.0.
    let v = json!({
        "instrument": inst.clone(),
        "base_market": { "spot": 1.08, "vol": 0.09, "r_dom": 0.04, "r_for": 0.01 },
        "conventions": conventions_json(),
        "axes": [
            { "factor": 1, "relative": true, "steps": [-0.1, 0.0, 0.1] },
            { "factor": 4, "relative": false, "steps": [0.003, 0.006] }
        ],
        "expiry_years": 0.25,
        "risk_buckets": {
            "vega_pillars": [{ "tenor_years": 0.25, "delta": 0.25, "vega": 999.0 }],
            "cross_gamma_pairs": [{ "factor_a": 0, "factor_b": 1, "value": 888.0 }],
            "roll_horizons_years": [0.003, 0.006]
        },
        "smile_model": "SMILE_MODEL_STOCHASTIC_VOL",
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "Scenario(full)",
        generated::decode_scenario_request(o),
        hand::hand_scenario_request_from_json(o),
    );
    // Sparse: no risk_buckets, no smile_model (both presence-tracked).
    let v0 = json!({
        "instrument": inst,
        "base_market": { "spot": 1.08, "vol": 0.09, "r_dom": 0.04, "r_for": 0.01 },
        "conventions": conventions_json(),
        "axes": [{ "factor": 0, "relative": false, "steps": [-0.02, 0.02] }],
        "expiry_years": 0.5,
    });
    let o0 = v0.as_object().expect("object");
    assert_decode_eq(
        "Scenario(sparse)",
        generated::decode_scenario_request(o0),
        hand::hand_scenario_request_from_json(o0),
    );
}

/// A fully-populated smile (every optional sub-message present).
fn full_smile() -> Smile {
    Smile {
        pair: Some(CcyPair {
            base: "EUR".to_owned(),
            quote: "USD".to_owned(),
        }),
        tenor_years: 0.25,
        broker_quotes: Some(BrokerQuoteSet {
            tenor_years: 0.25,
            atm_vol: 0.09,
            rr_25: -0.01,
            bf_25: 0.002,
            rr_10: -0.02,
            bf_10: 0.004,
            has_ten_delta: true,
        }),
        points: vec![
            SmilePoint {
                delta: 0.25,
                tenor_years: 0.25,
                vol: 0.095,
            },
            SmilePoint {
                delta: -0.25,
                tenor_years: 0.25,
                vol: 0.098,
            },
        ],
        conventions: Some(Conventions::default()),
        arbitrage: Some(ArbReport {
            butterfly_arbitrage_free: true,
            calendar_arbitrage_free: true,
            worst_density: 0.0,
            note: "clean".to_owned(),
            smile_model: SmileModel::Parametric as i32,
        }),
        epoch_nanos: 1_720_000_000_000_000_000,
    }
}

#[test]
fn smile_reply_encode_is_byte_identical() {
    let full = full_smile();
    assert_bytes_eq(
        "Smile(full)",
        &generated::encode_smile(&full),
        &hand::hand_smile_reply_to_json(&full),
    );
    // Sparse: every optional sub-message absent ⇒ JSON `null` (the `json!` policy).
    let sparse = Smile {
        pair: None,
        tenor_years: 0.1,
        broker_quotes: None,
        points: vec![],
        conventions: None,
        arbitrage: None,
        epoch_nanos: 0,
    };
    let g = generated::encode_smile(&sparse);
    assert_eq!(g.get("pair"), Some(&Value::Null));
    assert_eq!(g.get("broker_quotes"), Some(&Value::Null));
    assert_eq!(g.get("conventions"), Some(&Value::Null));
    assert_eq!(g.get("arbitrage"), Some(&Value::Null));
    assert_bytes_eq(
        "Smile(sparse)",
        &g,
        &hand::hand_smile_reply_to_json(&sparse),
    );
}

#[test]
fn mark_surface_response_encode_is_byte_identical() {
    let r = MarkSurfaceResponse {
        pair: Some(CcyPair {
            base: "GBP".to_owned(),
            quote: "USD".to_owned(),
        }),
        surface_version: 7,
        smiles: vec![full_smile()],
        epoch_nanos: 1_720_000_000_000_000_123,
    };
    assert_bytes_eq(
        "MarkSurfaceResponse",
        &generated::encode_mark_surface_response(&r),
        &hand::hand_mark_surface_response_to_json(&r),
    );
    // Absent pair ⇒ null.
    let r0 = MarkSurfaceResponse {
        pair: None,
        surface_version: 0,
        smiles: vec![],
        epoch_nanos: 0,
    };
    let g0 = generated::encode_mark_surface_response(&r0);
    assert_eq!(g0.get("pair"), Some(&Value::Null));
    assert_bytes_eq(
        "MarkSurfaceResponse(empty)",
        &g0,
        &hand::hand_mark_surface_response_to_json(&r0),
    );
}

#[test]
fn scenario_response_encode_is_byte_identical() {
    let full = ScenarioResponse {
        points: vec![
            ScenarioPoint {
                applied_shocks: vec![-0.1, 0.0, 0.1],
                shocked_market: Some(MarketContext::fx(1.08, 0.09, 0.04, 0.01)),
                greeks: Some(Greeks::default()),
                expiry_years: 0.24,
            },
            // A node with the optional market/greeks absent ⇒ null.
            ScenarioPoint {
                applied_shocks: vec![],
                shocked_market: None,
                greeks: None,
                expiry_years: 0.25,
            },
        ],
        bucketed_risk: Some(BucketedRisk {
            vega_buckets: vec![VegaBucket {
                tenor_years: 0.25,
                delta: 0.25,
                vega: 1234.5,
            }],
            cross_gammas: vec![CrossGamma {
                factor_a: 0,
                factor_b: 1,
                value: 9.9,
            }],
            theta_roll: vec![-1.0, -2.5],
            roll_horizons_years: vec![0.003, 0.006],
        }),
    };
    assert_bytes_eq(
        "ScenarioResponse(full)",
        &generated::encode_scenario_response(&full),
        &hand::hand_scenario_response_to_json(&full),
    );
    // No book-shaped risk ⇒ `bucketed_risk` null.
    let bare = ScenarioResponse {
        points: vec![],
        bucketed_risk: None,
    };
    let gb = generated::encode_scenario_response(&bare);
    assert_eq!(gb.get("bucketed_risk"), Some(&Value::Null));
    assert_bytes_eq(
        "ScenarioResponse(bare)",
        &gb,
        &hand::hand_scenario_response_to_json(&bare),
    );
}

// ===========================================================================
// WAVE 3 — RiskService: ListPositions / AggregateRisk / AggregateRatesRisk /
// DrillRisk / LimitStatus / BookRatesPosition / ListRatesPositions. The request
// decoders (shared `EntitlementPrincipal` + `ReportingNumeraire` + `VegaPillar` +
// linear-rates `CurveSet`/`RatesInstrument` trees) and the rich `RiskNode` /
// `RiskPosition` / `RatesRiskNode` reply encoders (incl. the FX-legacy
// `VanillaInputs` r_dom/r_for and `OrgKey.underlying`→`ccy_pair`, the
// `NonAdditiveRisk` null-absent optionals, and the null-absent `correlation_id`
// echoes) — the exact surface `handle_unary`'s risk arms are swapped onto.
// ===========================================================================

use celnet_proto::{
    AdditiveRisk, AggregateRatesRiskResponse, AggregateRiskResponse, BookRatesPositionResponse,
    CcyExposureLeg, DrillRiskResponse, KeyRateDv01, LimitStatusResponse, LimitUtilization,
    ListPositionsResponse, ListRatesPositionsResponse, NonAdditiveRisk, OisInstrument, OrgKey,
    RatesInstrument, RatesPosition, RatesRiskNode, RiskNode, RiskPosition, RiskScope,
    VanillaInputs, VegaLadderBucket, VegaPillar, rates_instrument,
};

/// The shared entitlement-principal body (`grant_all` + a scoped grant + a deny).
fn principal_json() -> Value {
    json!({
        "grant_all": false,
        "grants": [{ "scopes": [{ "dimension": 3, "value": 99 }] }],
        "denies": [{ "scopes": [{ "dimension": 1, "value": 7 }] }]
    })
}

/// A reporting-numeraire conversion table body.
fn numeraire_json() -> Value {
    json!({ "numeraire": "USD", "rates": [{ "ccy": "EUR", "rate": 1.08 }, { "ccy": "JPY", "rate": 0.0067 }] })
}

/// A calibrated OIS curve-set body (a broken reference date + one pillar).
fn curve_set_json() -> Value {
    json!({
        "currency": "USD",
        "reference_date": { "year": 2026, "month": 7, "day": 3 },
        "ois_pillars": [{ "tenor": { "years": 5 }, "par_rate": 0.041 }]
    })
}

/// One inline rates position (an OIS arm) as `AggregateRatesRisk`/`BookRates` carry.
fn ois_position_json() -> Value {
    json!({
        "position_id": 42,
        "entity": 1,
        "book": 7,
        "instrument": { "ois": { "tenor_years": 5, "fixed_rate": 0.04, "notional": 1_000_000.0, "side": 1 } }
    })
}

#[test]
fn list_positions_request_decode_is_byte_identical() {
    let v = json!({
        "scope": { "dimension": 3, "value": 99 },
        "principal": principal_json(),
        "correlation_id": 12_345,
        "session_token": "sess-abc"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "ListPositions(full)",
        generated::decode_list_positions_request(o),
        hand::hand_list_positions_request_from_json(o),
    );
    // Sparse: no scope / principal / correlation_id (the grant-all show-all posture).
    let v0 = json!({ "session_token": "s" });
    let o0 = v0.as_object().expect("object");
    assert_decode_eq(
        "ListPositions(sparse)",
        generated::decode_list_positions_request(o0),
        hand::hand_list_positions_request_from_json(o0),
    );
}

#[test]
fn aggregate_risk_request_decode_is_byte_identical() {
    let v = json!({
        "dimension": 2,
        "numeraire": numeraire_json(),
        "principal": principal_json(),
        "scope": { "dimension": 3, "value": 99 },
        "vega_pillars": [{ "tenor_days": 30, "delta_bp": 2500 }, { "tenor_days": 90, "delta_bp": -1000 }],
        "var_spot_shocks": [-0.01, 0.0, 0.01],
        "var_alpha": 0.99,
        "curvature_risk_weight": 0.05,
        "correlation_id": 7,
        "session_token": "tok"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "AggregateRisk",
        generated::decode_aggregate_risk_request(o),
        hand::hand_aggregate_risk_request_from_json(o),
    );
    // Minimal: bare dimension only (numeraire/principal/scope absent).
    let v0 = json!({ "dimension": 0 });
    let o0 = v0.as_object().expect("object");
    assert_decode_eq(
        "AggregateRisk(bare)",
        generated::decode_aggregate_risk_request(o0),
        hand::hand_aggregate_risk_request_from_json(o0),
    );
}

#[test]
fn drill_risk_request_decode_is_byte_identical() {
    let v = json!({
        "node": { "dimension": 3, "value": 99 },
        "child_dimension": 4,
        "numeraire": numeraire_json(),
        "principal": principal_json(),
        "vega_pillars": [{ "tenor_days": 30, "delta_bp": 2500 }],
        "include_children": true,
        "include_positions": true,
        "correlation_id": 55,
        "session_token": "t"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "DrillRisk",
        generated::decode_drill_risk_request(o),
        hand::hand_drill_risk_request_from_json(o),
    );
}

#[test]
fn limit_status_request_decode_is_byte_identical() {
    let v = json!({
        "scope": { "dimension": 3, "value": 99 },
        "numeraire": numeraire_json(),
        "principal": principal_json(),
        "vega_pillars": [{ "tenor_days": 30, "delta_bp": 2500 }],
        "var_spot_shocks": [-0.02, 0.02],
        "var_alpha": 0.975,
        "correlation_id": 9,
        "session_token": "t"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "LimitStatus",
        generated::decode_limit_status_request(o),
        hand::hand_limit_status_request_from_json(o),
    );
}

#[test]
fn aggregate_rates_risk_request_decode_is_byte_identical() {
    let v = json!({
        "curve_set": curve_set_json(),
        "positions": [ois_position_json()],
        "scope": { "entity": 1, "book": 7, "ccy": "USD" },
        "principal": principal_json(),
        "correlation_id": 3,
        "session_token": "t"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "AggregateRatesRisk",
        generated::decode_aggregate_rates_risk_request(o),
        hand::hand_aggregate_rates_risk_request_from_json(o),
    );
    // No positions (optional repeated ⇒ empty), no scope.
    let v0 = json!({ "curve_set": curve_set_json() });
    let o0 = v0.as_object().expect("object");
    assert_decode_eq(
        "AggregateRatesRisk(no-positions)",
        generated::decode_aggregate_rates_risk_request(o0),
        hand::hand_aggregate_rates_risk_request_from_json(o0),
    );
}

#[test]
fn book_and_list_rates_positions_request_decode_are_byte_identical() {
    let vb = json!({
        "session_token": "t",
        "position": ois_position_json(),
        "principal": principal_json(),
        "correlation_id": "corr-1"
    });
    let ob = vb.as_object().expect("object");
    assert_decode_eq(
        "BookRatesPosition",
        generated::decode_book_rates_position_request(ob),
        hand::hand_book_rates_position_request_from_json(ob),
    );
    let vl = json!({
        "session_token": "t",
        "scope": { "entity": 1, "ccy": "USD" },
        "principal": principal_json(),
        "correlation_id": "corr-2"
    });
    let ol = vl.as_object().expect("object");
    assert_decode_eq(
        "ListRatesPositions",
        generated::decode_list_rates_positions_request(ol),
        hand::hand_list_rates_positions_request_from_json(ol),
    );
}

/// A fully-attributed risk position: an org key with the FX-legacy underlying, the
/// FX carry-seam `VanillaInputs`, and an attribution record.
fn full_risk_position() -> RiskPosition {
    RiskPosition {
        position_id: 1001,
        org: Some(OrgKey {
            trader: 5,
            book: 7,
            desk: 3,
            underlying: Some(Underlying::fx(CcyPair {
                base: "EUR".to_owned(),
                quote: "USD".to_owned(),
            })),
            location: 2,
            entity: 1,
        }),
        option_type: 0,
        notional_base: 1_000_000.0,
        inputs: Some(VanillaInputs::fx(1.08, 1.10, 0.09, 0.25, 0.04, 0.01)),
        quoted_delta: 1,
        premium_style: 1,
        surface_version: 88,
        attribution: Some(AttributionRecord {
            quoted_by: None,
            held_by: None,
            won: Some(true),
            lp_count: Some(3),
        }),
    }
}

/// A sparse position: every optional sub-message absent ⇒ JSON `null`.
fn sparse_risk_position() -> RiskPosition {
    RiskPosition {
        position_id: 2002,
        org: None,
        option_type: 1,
        notional_base: -500_000.0,
        inputs: None,
        quoted_delta: 0,
        premium_style: 0,
        surface_version: 0,
        attribution: None,
    }
}

#[test]
fn list_positions_response_encode_is_byte_identical() {
    let r = ListPositionsResponse {
        positions: vec![full_risk_position(), sparse_risk_position()],
        correlation_id: Some(12_345),
    };
    let g = generated::encode_list_positions_response(&r);
    // The FX-legacy projections: `VanillaInputs.r_dom`/`r_for` + `OrgKey.ccy_pair`.
    let inputs = &g["positions"][0]["inputs"];
    assert!(inputs.get("r_dom").is_some() && inputs.get("r_for").is_some());
    assert!(inputs.get("discount_rate").is_none() && inputs.get("carry").is_none());
    assert_eq!(g["positions"][0]["org"]["ccy_pair"]["base"], json!("EUR"));
    assert!(g["positions"][0]["org"].get("underlying").is_none());
    // The sparse position's absent optionals ⇒ null.
    assert_eq!(g["positions"][1].get("org"), Some(&Value::Null));
    assert_eq!(g["positions"][1].get("inputs"), Some(&Value::Null));
    assert_eq!(g["positions"][1].get("attribution"), Some(&Value::Null));
    assert_bytes_eq(
        "ListPositionsResponse",
        &g,
        &hand::hand_list_positions_response_to_json(&r),
    );
    // Absent correlation_id ⇒ null.
    let r0 = ListPositionsResponse {
        positions: vec![],
        correlation_id: None,
    };
    let g0 = generated::encode_list_positions_response(&r0);
    assert_eq!(g0.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "ListPositionsResponse(empty)",
        &g0,
        &hand::hand_list_positions_response_to_json(&r0),
    );
}

/// A representative risk node: additive measures with a delta vector + vega ladder,
/// and non-additive VaR/ES present.
fn full_risk_node() -> RiskNode {
    RiskNode {
        dimension: 2,
        group: 99,
        additive: Some(AdditiveRisk {
            delta_numeraire: 1_234.5,
            delta_vector: vec![
                CcyExposureLeg {
                    ccy: "EUR".to_owned(),
                    amount: 1_000.0,
                },
                CcyExposureLeg {
                    ccy: "USD".to_owned(),
                    amount: -1_080.0,
                },
            ],
            gamma: 12.0,
            vega_numeraire: 55.0,
            theta: -3.0,
            vanna: 1.1,
            volga: 2.2,
            charm: 0.3,
            speed: 0.01,
            zomma: 0.02,
            color: 0.03,
            premium_numeraire: 9_000.0,
            vega_ladder: vec![VegaLadderBucket {
                pillar: Some(VegaPillar {
                    tenor_days: 30,
                    delta_bp: 2500,
                }),
                vega: 44.0,
            }],
        }),
        nonadditive: Some(NonAdditiveRisk {
            var: Some(50_000.0),
            es: Some(65_000.0),
            var_alpha: Some(0.99),
            curvature_spot: None,
        }),
        position_count: 4,
    }
}

#[test]
fn aggregate_risk_response_encode_is_byte_identical() {
    let r = AggregateRiskResponse {
        dimension: 2,
        numeraire: "USD".to_owned(),
        nodes: vec![full_risk_node()],
        correlation_id: Some(7),
    };
    let g = generated::encode_aggregate_risk_response(&r);
    // The not-evaluated `curvature_spot` ⇒ null (never a spurious zero).
    assert_eq!(
        g["nodes"][0]["nonadditive"].get("curvature_spot"),
        Some(&Value::Null)
    );
    assert_bytes_eq(
        "AggregateRiskResponse",
        &g,
        &hand::hand_aggregate_risk_response_to_json(&r),
    );
    // Absent additive/nonadditive ⇒ null; absent correlation_id ⇒ null.
    let bare = AggregateRiskResponse {
        dimension: 0,
        numeraire: "USD".to_owned(),
        nodes: vec![RiskNode {
            dimension: 0,
            group: 0,
            additive: None,
            nonadditive: None,
            position_count: 0,
        }],
        correlation_id: None,
    };
    let gb = generated::encode_aggregate_risk_response(&bare);
    assert_eq!(gb["nodes"][0].get("additive"), Some(&Value::Null));
    assert_eq!(gb["nodes"][0].get("nonadditive"), Some(&Value::Null));
    assert_eq!(gb.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "AggregateRiskResponse(bare)",
        &gb,
        &hand::hand_aggregate_risk_response_to_json(&bare),
    );
}

#[test]
fn drill_risk_response_encode_is_byte_identical() {
    let r = DrillRiskResponse {
        node: Some(RiskScope {
            dimension: 3,
            value: 99,
        }),
        children: vec![full_risk_node()],
        positions: vec![full_risk_position()],
        correlation_id: Some(55),
    };
    assert_bytes_eq(
        "DrillRiskResponse",
        &generated::encode_drill_risk_response(&r),
        &hand::hand_drill_risk_response_to_json(&r),
    );
    let bare = DrillRiskResponse {
        node: None,
        children: vec![],
        positions: vec![],
        correlation_id: None,
    };
    let gb = generated::encode_drill_risk_response(&bare);
    assert_eq!(gb.get("node"), Some(&Value::Null));
    assert_eq!(gb.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "DrillRiskResponse(bare)",
        &gb,
        &hand::hand_drill_risk_response_to_json(&bare),
    );
}

#[test]
fn limit_status_response_encode_is_byte_identical() {
    let r = LimitStatusResponse {
        scope: Some(RiskScope {
            dimension: 3,
            value: 99,
        }),
        limits: vec![
            LimitUtilization {
                metric: 2,
                vega_pillar: Some(VegaPillar {
                    tenor_days: 30,
                    delta_bp: 2500,
                }),
                tenor_days: 0,
                cap: 1_000_000.0,
                exposure: 750_000.0,
                ratio: 0.75,
                status: 1,
                enforcement: 1,
                headroom: 250_000.0,
            },
            LimitUtilization {
                metric: 0,
                vega_pillar: None,
                tenor_days: 0,
                cap: 500.0,
                exposure: 600.0,
                ratio: 1.2,
                status: 3,
                enforcement: 1,
                headroom: -100.0,
            },
        ],
        worst: 3,
        hard_breach: true,
        correlation_id: Some(9),
    };
    let g = generated::encode_limit_status_response(&r);
    // The non-VEGA_BUCKET limit's absent `vega_pillar` ⇒ null.
    assert_eq!(g["limits"][1].get("vega_pillar"), Some(&Value::Null));
    assert_bytes_eq(
        "LimitStatusResponse",
        &g,
        &hand::hand_limit_status_response_to_json(&r),
    );
}

#[test]
fn aggregate_rates_risk_response_encode_is_byte_identical() {
    let r = AggregateRatesRiskResponse {
        nodes: vec![RatesRiskNode {
            ccy: "USD".to_owned(),
            net_pv: 12_345.6,
            net_pv01: 78.9,
            net_dv01: 80.1,
            key_rate_ladder: vec![
                KeyRateDv01 {
                    tenor_years: 2,
                    dv01: 10.0,
                },
                KeyRateDv01 {
                    tenor_years: 5,
                    dv01: 25.0,
                },
            ],
        }],
        correlation_id: Some(3),
    };
    assert_bytes_eq(
        "AggregateRatesRiskResponse",
        &generated::encode_aggregate_rates_risk_response(&r),
        &hand::hand_aggregate_rates_risk_response_to_json(&r),
    );
    let bare = AggregateRatesRiskResponse {
        nodes: vec![],
        correlation_id: None,
    };
    let gb = generated::encode_aggregate_rates_risk_response(&bare);
    assert_eq!(gb.get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "AggregateRatesRiskResponse(bare)",
        &gb,
        &hand::hand_aggregate_rates_risk_response_to_json(&bare),
    );
}

/// A booked rates position (an OIS arm) as the rates book/list encode.
fn ois_rates_position() -> RatesPosition {
    RatesPosition {
        position_id: 42,
        entity: 1,
        book: 7,
        instrument: Some(RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: 5,
                fixed_rate: 0.04,
                notional: 1_000_000.0,
                side: 1,
            })),
        }),
    }
}

#[test]
fn book_and_list_rates_positions_response_encode_are_byte_identical() {
    let rb = BookRatesPositionResponse {
        position: Some(ois_rates_position()),
    };
    assert_bytes_eq(
        "BookRatesPositionResponse",
        &generated::encode_book_rates_position_response(&rb),
        &hand::hand_book_rates_position_response_to_json(&rb),
    );
    // Absent position ⇒ null.
    let rb0 = BookRatesPositionResponse { position: None };
    let gb0 = generated::encode_book_rates_position_response(&rb0);
    assert_eq!(gb0.get("position"), Some(&Value::Null));
    assert_bytes_eq(
        "BookRatesPositionResponse(empty)",
        &gb0,
        &hand::hand_book_rates_position_response_to_json(&rb0),
    );
    let rl = ListRatesPositionsResponse {
        positions: vec![ois_rates_position()],
    };
    assert_bytes_eq(
        "ListRatesPositionsResponse",
        &generated::encode_list_rates_positions_response(&rl),
        &hand::hand_list_rates_positions_response_to_json(&rl),
    );
}

// ===========================================================================
// WAVE 3 — RfqDeskService: SubmitDeskRequest / RespondDeskRequest /
// AcceptDeskQuote / ListDeskRequests / ListDeals. The request decoders (incl. the
// `respond` oneof error-on-both quirk and the `list_deals` non-erroring scope) and
// the `DeskRequest` / `Deal` blotter reply encoders (reusing the rates encode tree,
// with null-absent `quote` / `position_id` / `correlation_id`) — the exact surface
// `handle_unary`'s rfq_desk arms are swapped onto.
// ===========================================================================

use celnet_proto::{
    AcceptDeskQuoteResponse, CurveSet, Deal, DeskQuote, DeskRequest, ListDealsResponse,
    ListDeskRequestsResponse, OisPillar, PillarTenor, RespondDeskRequestResponse,
    SubmitDeskRequestResponse, pillar_tenor,
};

/// A calibrated OIS curve set (a broken reference date + one year pillar).
fn a_curve_set() -> CurveSet {
    CurveSet {
        currency: "USD".to_owned(),
        reference_date: Some(BrokenDate {
            year: 2026,
            month: 7,
            day: 3,
        }),
        ois_pillars: vec![OisPillar {
            tenor: Some(PillarTenor {
                point: Some(pillar_tenor::Point::Years(5)),
            }),
            par_rate: 0.041,
        }],
    }
}

/// A dealt OIS instrument.
fn a_rates_instrument() -> RatesInstrument {
    RatesInstrument {
        instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
            tenor_years: 5,
            fixed_rate: 0.04,
            notional: 1_000_000.0,
            side: 1,
        })),
    }
}

/// A desk request in its QUOTED state (every optional sub-message present).
fn full_desk_request() -> DeskRequest {
    DeskRequest {
        request_id: "req-1".to_owned(),
        kind: 1,
        counterparty: "CP-A".to_owned(),
        desk: "USD-RATES".to_owned(),
        instrument: Some(a_rates_instrument()),
        curve_set: Some(a_curve_set()),
        side: 1,
        notional: 1_000_000.0,
        received_at_nanos: 1_720_000_000_000_000_000,
        expires_at_nanos: 1_720_000_030_000_000_000,
        state: 2,
        quote: Some(DeskQuote {
            price: 0.041,
            notional: 1_000_000.0,
            valid_for_ms: 5_000,
            trader: "tdr".to_owned(),
        }),
        correlation_id: Some("corr-1".to_owned()),
    }
}

/// A pending desk request: no quote yet, no correlation id ⇒ those reach the wire as
/// JSON `null`; instrument/curve_set present.
fn pending_desk_request() -> DeskRequest {
    DeskRequest {
        request_id: "req-2".to_owned(),
        kind: 2,
        counterparty: "CP-B".to_owned(),
        desk: "USD-RATES".to_owned(),
        instrument: Some(a_rates_instrument()),
        curve_set: Some(a_curve_set()),
        side: 0,
        notional: 500_000.0,
        received_at_nanos: 1_720_000_000_000_000_000,
        expires_at_nanos: 1_720_000_030_000_000_000,
        state: 1,
        quote: None,
        correlation_id: None,
    }
}

fn a_deal() -> Deal {
    Deal {
        deal_id: "deal-1".to_owned(),
        request_id: "req-1".to_owned(),
        kind: 1,
        counterparty: "CP-A".to_owned(),
        desk: "USD-RATES".to_owned(),
        instrument: Some(a_rates_instrument()),
        curve_set: Some(a_curve_set()),
        side: 0,
        notional: 1_000_000.0,
        price: 0.041,
        executed_at_nanos: 1_720_000_000_000_000_000,
        trader: "tdr".to_owned(),
        position_id: Some(4_242),
        correlation_id: Some("corr-1".to_owned()),
    }
}

#[test]
fn submit_desk_request_decode_is_byte_identical() {
    let v = json!({
        "session_token": "t",
        "kind": 1,
        "counterparty": "CP-A",
        "desk": "USD-RATES",
        "instrument": { "ois": { "tenor_years": 5, "fixed_rate": 0.04, "notional": 1_000_000.0, "side": 1 } },
        "curve_set": curve_set_json(),
        "side": 1,
        "notional": 1_000_000.0,
        "ttl_ms": 5_000,
        "principal": principal_json(),
        "correlation_id": "corr-1"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "SubmitDeskRequest",
        generated::decode_submit_desk_request(o),
        hand::hand_submit_desk_request_from_json(o),
    );
}

#[test]
fn respond_desk_request_decode_is_byte_identical() {
    // The `quote` arm.
    let vq = json!({
        "session_token": "t",
        "request_id": "req-1",
        "quote": { "price": 0.041, "notional": 1_000_000.0, "valid_for_ms": 5_000, "trader": "tdr" },
        "principal": principal_json(),
        "correlation_id": "corr-1"
    });
    let oq = vq.as_object().expect("object");
    assert_decode_eq(
        "RespondDeskRequest(quote)",
        generated::decode_respond_desk_request(oq),
        hand::hand_respond_desk_request_from_json(oq),
    );
    // The `reject` arm.
    let vr = json!({
        "request_id": "req-1",
        "reject": { "reason": "off-the-run" }
    });
    let or = vr.as_object().expect("object");
    assert_decode_eq(
        "RespondDeskRequest(reject)",
        generated::decode_respond_desk_request(or),
        hand::hand_respond_desk_request_from_json(or),
    );
    // Neither arm ⇒ `response: None`.
    let vn = json!({ "request_id": "req-1" });
    let on = vn.as_object().expect("object");
    assert_decode_eq(
        "RespondDeskRequest(neither)",
        generated::decode_respond_desk_request(on),
        hand::hand_respond_desk_request_from_json(on),
    );
    // BOTH arms present ⇒ BOTH decoders error (the mutual-exclusion quirk).
    let vb = json!({
        "request_id": "req-1",
        "quote": { "price": 0.04, "notional": 1.0 },
        "reject": { "reason": "x" }
    });
    let ob = vb.as_object().expect("object");
    assert!(
        generated::decode_respond_desk_request(ob).is_err(),
        "both arms present must error (generated)"
    );
    assert!(
        hand::hand_respond_desk_request_from_json(ob).is_err(),
        "both arms present must error (hand)"
    );
}

#[test]
fn accept_desk_quote_decode_is_byte_identical() {
    let v = json!({
        "session_token": "t",
        "request_id": "req-1",
        "principal": principal_json(),
        "correlation_id": "corr-1"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "AcceptDeskQuote",
        generated::decode_accept_desk_quote(o),
        hand::hand_accept_desk_quote_from_json(o),
    );
}

#[test]
fn list_desk_requests_decode_is_byte_identical() {
    let v = json!({
        "session_token": "t",
        "scope": { "states": [1, 2, 3], "desk": "USD-RATES" },
        "principal": principal_json(),
        "correlation_id": "corr-1"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "ListDeskRequests",
        generated::decode_list_desk_requests(o),
        hand::hand_list_desk_requests_from_json(o),
    );
    // No scope.
    let v0 = json!({ "session_token": "t" });
    let o0 = v0.as_object().expect("object");
    assert_decode_eq(
        "ListDeskRequests(no-scope)",
        generated::decode_list_desk_requests(o0),
        hand::hand_list_desk_requests_from_json(o0),
    );
}

/// REGRESSION (live GUI shape): the WS transport frames the request/reply match by
/// injecting a **numeric** `correlation_id` (the connection sequence) onto EVERY
/// frame — and it collides on the SAME JSON key as the proto **`string
/// correlation_id`** of the RfqDesk + linear-rates-book verbs. The hand codec's
/// lenient `opt_string` reads a non-string as `None`; the generated codec MUST too,
/// or the whole request fails to decode (which silently broke the live rates-book
/// booking + simulator RFQ-injection e2e flows — `book_rates_position` /
/// `submit_desk_request` returned an error frame the GUI swallowed, so the book
/// stayed empty and the injected item never rendered). The earlier vectors above set
/// `correlation_id` to a STRING (`"corr-1"`), so they never exercised the collision;
/// these use the transport's REAL numeric framing id + the injected `type`
/// discriminator, and assert the message `correlation_id` decodes to `None` (the
/// numeric framing id is NOT the message's business correlation id) on BOTH codecs.
#[test]
fn desk_and_rates_book_decode_transport_numeric_correlation_id() {
    // The exact live grant-all principal the GUI sends on these gated verbs (an
    // explicit `grant_all: true` with empty grants/denies — never the populated
    // `principal_json()` the other vectors use).
    let grant_all = || json!({ "grant_all": true, "grants": [], "denies": [] });

    // submit_desk_request — the exact live frame (9-pillar SOFR curve + numeric
    // framing `correlation_id` + injected `type`).
    let submit = json!({
        "type": "submit_desk_request",
        "correlation_id": 20,
        "session_token": "sess-tok",
        "kind": 1,
        "counterparty": "Acme Capital",
        "desk": "g10-rates",
        "instrument": { "ois": { "tenor_years": 5, "fixed_rate": 0.04, "notional": 50_000_000.0, "side": 0 } },
        "curve_set": {
            "currency": "USD",
            "reference_date": { "year": 2026, "month": 6, "day": 25 },
            "ois_pillars": [
                { "tenor": { "years": 1 }, "par_rate": 0.0432 },
                { "tenor": { "years": 2 }, "par_rate": 0.0418 },
                { "tenor": { "years": 3 }, "par_rate": 0.0409 },
                { "tenor": { "years": 5 }, "par_rate": 0.0405 },
                { "tenor": { "years": 7 }, "par_rate": 0.0408 },
                { "tenor": { "years": 10 }, "par_rate": 0.0415 },
                { "tenor": { "years": 15 }, "par_rate": 0.0421 },
                { "tenor": { "years": 20 }, "par_rate": 0.0424 },
                { "tenor": { "years": 30 }, "par_rate": 0.0423 }
            ]
        },
        "side": 0,
        "notional": 50_000_000.0,
        "ttl_ms": 120_000,
        "principal": grant_all()
    });
    let o = submit.as_object().expect("object");
    let decoded = generated::decode_submit_desk_request(o).expect("generated decodes live frame");
    assert_eq!(
        decoded.correlation_id, None,
        "the numeric framing correlation_id must NOT populate the message `string correlation_id`"
    );
    assert_decode_eq(
        "SubmitDeskRequest(live/numeric-corr)",
        generated::decode_submit_desk_request(o),
        hand::hand_submit_desk_request_from_json(o),
    );

    // book_rates_position — the exact live frame (numeric framing correlation_id).
    let book = json!({
        "type": "book_rates_position",
        "correlation_id": 33,
        "session_token": "sess-tok",
        "position": {
            "position_id": 0,
            "entity": 3,
            "book": 5,
            "instrument": { "ois": { "tenor_years": 5, "fixed_rate": 0.0405, "notional": 50_000_000.0, "side": 1 } }
        },
        "principal": grant_all()
    });
    let o = book.as_object().expect("object");
    let decoded =
        generated::decode_book_rates_position_request(o).expect("generated decodes live frame");
    assert_eq!(decoded.correlation_id, None);
    assert_decode_eq(
        "BookRatesPosition(live/numeric-corr)",
        generated::decode_book_rates_position_request(o),
        hand::hand_book_rates_position_request_from_json(o),
    );

    // list_rates_positions — grant-all, numeric framing correlation_id, no scope.
    let list_rates = json!({
        "type": "list_rates_positions",
        "correlation_id": 30,
        "session_token": "sess-tok",
        "principal": grant_all()
    });
    let o = list_rates.as_object().expect("object");
    assert_eq!(
        generated::decode_list_rates_positions_request(o)
            .expect("generated decodes")
            .correlation_id,
        None
    );
    assert_decode_eq(
        "ListRatesPositions(live/numeric-corr)",
        generated::decode_list_rates_positions_request(o),
        hand::hand_list_rates_positions_request_from_json(o),
    );

    // list_desk_requests — grant-all, numeric framing correlation_id, no scope.
    let list_desk = json!({
        "type": "list_desk_requests",
        "correlation_id": 6,
        "session_token": "sess-tok",
        "principal": grant_all()
    });
    let o = list_desk.as_object().expect("object");
    assert_eq!(
        generated::decode_list_desk_requests(o)
            .expect("generated decodes")
            .correlation_id,
        None
    );
    assert_decode_eq(
        "ListDeskRequests(live/numeric-corr)",
        generated::decode_list_desk_requests(o),
        hand::hand_list_desk_requests_from_json(o),
    );
}

#[test]
fn list_deals_decode_is_byte_identical() {
    // Object scope.
    let v = json!({
        "session_token": "t",
        "scope": { "desk": "USD-RATES" },
        "principal": principal_json(),
        "correlation_id": "corr-1"
    });
    let o = v.as_object().expect("object");
    assert_decode_eq(
        "ListDeals(object-scope)",
        generated::decode_list_deals(o),
        hand::hand_list_deals_from_json(o),
    );
    // A NON-OBJECT scope is silently dropped to `None` (the non-erroring quirk) —
    // both decoders build `scope: None` rather than erroring.
    let vs = json!({ "session_token": "t", "scope": "all-desks" });
    let os = vs.as_object().expect("object");
    assert_decode_eq(
        "ListDeals(non-object-scope)",
        generated::decode_list_deals(os),
        hand::hand_list_deals_from_json(os),
    );
    // No scope at all.
    let v0 = json!({ "session_token": "t" });
    let o0 = v0.as_object().expect("object");
    assert_decode_eq(
        "ListDeals(no-scope)",
        generated::decode_list_deals(o0),
        hand::hand_list_deals_from_json(o0),
    );
}

#[test]
fn desk_reply_encode_is_byte_identical() {
    // SubmitDeskRequestResponse: the captured PENDING request (null quote/corr).
    let submit = SubmitDeskRequestResponse {
        request: Some(pending_desk_request()),
    };
    let gs = generated::encode_submit_desk_request_response(&submit);
    assert_eq!(gs["request"].get("quote"), Some(&Value::Null));
    assert_eq!(gs["request"].get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "SubmitDeskRequestResponse",
        &gs,
        &hand::hand_submit_desk_request_response_to_json(&submit),
    );
    // Absent request ⇒ null.
    let submit0 = SubmitDeskRequestResponse { request: None };
    let gs0 = generated::encode_submit_desk_request_response(&submit0);
    assert_eq!(gs0.get("request"), Some(&Value::Null));
    assert_bytes_eq(
        "SubmitDeskRequestResponse(empty)",
        &gs0,
        &hand::hand_submit_desk_request_response_to_json(&submit0),
    );
    // RespondDeskRequestResponse: the QUOTED request (all sub-messages present).
    let respond = RespondDeskRequestResponse {
        request: Some(full_desk_request()),
    };
    let gr = generated::encode_respond_desk_request_response(&respond);
    // The FI instrument/curve encode tree rides inside the request.
    assert_eq!(gr["request"]["instrument"]["ois"]["tenor_years"], json!(5));
    assert_eq!(gr["request"]["curve_set"]["currency"], json!("USD"));
    assert_bytes_eq(
        "RespondDeskRequestResponse",
        &gr,
        &hand::hand_respond_desk_request_response_to_json(&respond),
    );
    // AcceptDeskQuoteResponse: the booked deal + the terminal request.
    let accept = AcceptDeskQuoteResponse {
        deal: Some(a_deal()),
        request: Some(full_desk_request()),
    };
    assert_bytes_eq(
        "AcceptDeskQuoteResponse",
        &generated::encode_accept_desk_quote_response(&accept),
        &hand::hand_accept_desk_quote_response_to_json(&accept),
    );
    // Absent deal/request ⇒ null.
    let accept0 = AcceptDeskQuoteResponse {
        deal: None,
        request: None,
    };
    let ga0 = generated::encode_accept_desk_quote_response(&accept0);
    assert_eq!(ga0.get("deal"), Some(&Value::Null));
    assert_eq!(ga0.get("request"), Some(&Value::Null));
    assert_bytes_eq(
        "AcceptDeskQuoteResponse(empty)",
        &ga0,
        &hand::hand_accept_desk_quote_response_to_json(&accept0),
    );
    // ListDeskRequestsResponse.
    let list_req = ListDeskRequestsResponse {
        requests: vec![full_desk_request(), pending_desk_request()],
    };
    assert_bytes_eq(
        "ListDeskRequestsResponse",
        &generated::encode_list_desk_requests_response(&list_req),
        &hand::hand_list_desk_requests_response_to_json(&list_req),
    );
    // ListDealsResponse: a deal with a null position_id/correlation_id too.
    let deal_null = Deal {
        position_id: None,
        correlation_id: None,
        ..a_deal()
    };
    let list_deals = ListDealsResponse {
        deals: vec![a_deal(), deal_null],
    };
    let gd = generated::encode_list_deals_response(&list_deals);
    assert_eq!(gd["deals"][1].get("position_id"), Some(&Value::Null));
    assert_eq!(gd["deals"][1].get("correlation_id"), Some(&Value::Null));
    assert_bytes_eq(
        "ListDealsResponse",
        &gd,
        &hand::hand_list_deals_response_to_json(&list_deals),
    );
}

// ===========================================================================
// AuthService — the admin + session surface (wave 4, the FINAL family): the
// request decoders (login/session, user/desk/entity/book CRUD, capabilities +
// roles, the instrument registry with its `definition` family oneof, and
// `BuildCurve`) and the reply encoders, byte-identical to the hand codec incl. the
// presence-tracked-null policy (`correlation_id`, `UserDesc.desk_id`, `BondDef`
// coupon dates), the `capabilities` repeated-message lists and the `calendars`
// repeated-string.
// ===========================================================================

use celnet_proto::{
    BondDef, BookDesc, CalibratedCurve, CalibratedCurvePoint, CapabilityDesc, CreateBookResponse,
    CreateDeskResponse, CreateEntityResponse, CreateInstrumentResponse, CreateUserResponse,
    DeleteBookResponse, DeleteDeskResponse, DeleteEntityResponse, DeleteInstrumentResponse,
    DeleteUserResponse, DepositDef, DeskDesc, EntityDesc, ExternalId, FraDef,
    GetInstrumentResponse, GetRoleCapabilitiesResponse, GetUserCapabilitiesResponse,
    InstrumentDefDesc, ListBooksResponse, ListDesksResponse, ListEntitiesResponse,
    ListInstrumentsResponse, ListUsersResponse, LoginResponse, LogoutResponse, OisDef,
    ResetPasswordResponse, SetRoleCapabilitiesResponse, SetUserCapabilitiesResponse, StirFutureDef,
    UpdateBookResponse, UpdateDeskResponse, UpdateEntityResponse, UpdateInstrumentResponse,
    UpdateUserResponse, UserDesc, VanillaIrsDef, instrument_def_desc::Definition,
};

// --- fixtures ----------------------------------------------------------------

fn a_capability(action: &str, asset: &str) -> CapabilityDesc {
    CapabilityDesc {
        action: action.to_owned(),
        asset: asset.to_owned(),
    }
}

fn a_user_desc(desk_id: Option<&str>) -> UserDesc {
    UserDesc {
        id: "u-1".to_owned(),
        email: "trader@celer.example".to_owned(),
        display_name: "A Trader".to_owned(),
        role: 2,
        desk_id: desk_id.map(str::to_owned),
        disabled: false,
    }
}

fn an_entity_desc() -> EntityDesc {
    EntityDesc {
        key: 7,
        name: "Celer Capital".to_owned(),
        code: "CELCAP".to_owned(),
    }
}

fn a_book_desc() -> BookDesc {
    BookDesc {
        key: 11,
        name: "G10 Vol".to_owned(),
        entity_key: 7,
    }
}

fn a_broken_date(year: i32, month: u32, day: u32) -> BrokenDate {
    BrokenDate { year, month, day }
}

/// A fully-populated instrument carrying `definition` — exercising every family
/// adapter + the `calendars` repeated-string and (for the bond) the optional
/// coupon dates.
fn instrument_def_with(definition: Definition) -> InstrumentDefDesc {
    InstrumentDefDesc {
        instrument_id: "usd-inst".to_owned(),
        name: "USD Instrument".to_owned(),
        description: "a calibrating instrument".to_owned(),
        currency: "USD".to_owned(),
        external_ids: vec![
            ExternalId {
                scheme: "isin".to_owned(),
                value: "US0000000000".to_owned(),
            },
            ExternalId {
                scheme: "ticker".to_owned(),
                value: "USDX".to_owned(),
            },
        ],
        definition: Some(definition),
    }
}

fn deposit_def() -> Definition {
    Definition::Deposit(DepositDef {
        index: "SOFR".to_owned(),
        tenor: "3M".to_owned(),
        day_count: "ACT/360".to_owned(),
        business_day_convention: "MODFOLLOWING".to_owned(),
        calendars: vec!["USD".to_owned(), "USNY".to_owned()],
        spot_lag_days: 2,
    })
}

fn fra_def() -> Definition {
    Definition::Fra(FraDef {
        float_index: "SOFR".to_owned(),
        start_tenor: "3M".to_owned(),
        end_tenor: "6M".to_owned(),
        accrual_day_count: "ACT/360".to_owned(),
        business_day_convention: "MODFOLLOWING".to_owned(),
        calendars: vec!["USD".to_owned()],
        spot_lag_days: 2,
    })
}

fn stir_future_def() -> Definition {
    Definition::StirFuture(StirFutureDef {
        contract_code: "SR3".to_owned(),
        reference_start: "2026-06-17".to_owned(),
        reference_end: "2026-09-16".to_owned(),
        day_count: "ACT/360".to_owned(),
        calendars: vec!["USD".to_owned()],
        convexity_vol: 0.006,
        contract_size: 2500.0,
    })
}

fn vanilla_irs_def() -> Definition {
    Definition::VanillaIrs(VanillaIrsDef {
        tenor: "10Y".to_owned(),
        fixed_frequency: "6M".to_owned(),
        fixed_day_count: "30/360".to_owned(),
        float_index: "SOFR".to_owned(),
        float_frequency: "3M".to_owned(),
        float_day_count: "ACT/360".to_owned(),
        business_day_convention: "MODFOLLOWING".to_owned(),
        calendars: vec!["USD".to_owned()],
        roll_convention: "EOM".to_owned(),
        spot_lag_days: 2,
    })
}

fn ois_def() -> Definition {
    Definition::Ois(OisDef {
        tenor: "5Y".to_owned(),
        index: "SOFR".to_owned(),
        fixed_frequency: "1Y".to_owned(),
        fixed_day_count: "ACT/360".to_owned(),
        float_day_count: "ACT/360".to_owned(),
        business_day_convention: "MODFOLLOWING".to_owned(),
        calendars: vec!["USD".to_owned()],
        spot_lag_days: 2,
    })
}

/// A bond with a present `issue_date`/`maturity_date` and an absent
/// `dated_date`/`first_coupon_date` — exercises both the present-message and the
/// `null`-when-absent optional/singular coupon-date rules.
fn bond_def() -> Definition {
    Definition::Bond(BondDef {
        issuer: "US TREASURY".to_owned(),
        coupon_rate: 0.0425,
        coupon_type: "FIXED".to_owned(),
        coupon_frequency: "6M".to_owned(),
        day_count: "ACT/ACT".to_owned(),
        issue_date: Some(a_broken_date(2026, 5, 15)),
        dated_date: None,
        first_coupon_date: None,
        maturity_date: Some(a_broken_date(2036, 5, 15)),
        redemption: 100.0,
        calendars: vec!["USGS".to_owned()],
    })
}

// --- decode: session + user + capabilities -----------------------------------

#[test]
fn auth_session_verbs_decode_byte_identical() {
    let login = json!({ "email": "a@b.c", "password": "pw", "correlation_id": 5 });
    let o = login.as_object().expect("object");
    assert_decode_eq(
        "LoginRequest",
        generated::decode_login_request(o),
        hand::hand_login_request_from_json(o),
    );
    // Absent correlation_id ⇒ None.
    let login_min = json!({ "email": "a@b.c", "password": "pw" });
    let o = login_min.as_object().expect("object");
    assert_decode_eq(
        "LoginRequest(no-corr)",
        generated::decode_login_request(o),
        hand::hand_login_request_from_json(o),
    );

    let logout = json!({ "session_token": "tok", "correlation_id": 1 });
    let o = logout.as_object().expect("object");
    assert_decode_eq(
        "LogoutRequest",
        generated::decode_logout_request(o),
        hand::hand_logout_request_from_json(o),
    );

    let list = json!({ "session_token": "tok" });
    let o = list.as_object().expect("object");
    assert_decode_eq(
        "ListUsersRequest",
        generated::decode_list_users_request(o),
        hand::hand_list_users_request_from_json(o),
    );
}

#[test]
fn auth_user_crud_decode_byte_identical() {
    let cases = [
        (
            "create-full",
            json!({ "session_token": "tok", "email": "a@b.c", "display_name": "A",
                    "role": 2, "desk_id": "fx", "password": "pw", "correlation_id": 3 }),
        ),
        // Absent desk_id (⇒ None) + empty desk_id (⇒ None via opt_string filter).
        (
            "create-no-desk",
            json!({ "session_token": "tok", "email": "a@b.c", "display_name": "A",
                    "role": 0, "password": "pw" }),
        ),
        (
            "create-empty-desk",
            json!({ "session_token": "tok", "email": "a@b.c", "display_name": "A",
                    "role": 1, "desk_id": "", "password": "pw" }),
        ),
    ];
    for (label, body) in &cases {
        let o = body.as_object().expect("object");
        assert_decode_eq(
            &format!("CreateUserRequest({label})"),
            generated::decode_create_user_request(o),
            hand::hand_create_user_request_from_json(o),
        );
    }

    let update = json!({ "session_token": "tok", "id": "u-1", "display_name": "A2",
        "role": 3, "desk_id": "rates", "disabled": true, "correlation_id": 9 });
    let o = update.as_object().expect("object");
    assert_decode_eq(
        "UpdateUserRequest",
        generated::decode_update_user_request(o),
        hand::hand_update_user_request_from_json(o),
    );

    let del = json!({ "session_token": "tok", "id": "u-1" });
    let o = del.as_object().expect("object");
    assert_decode_eq(
        "DeleteUserRequest",
        generated::decode_delete_user_request(o),
        hand::hand_delete_user_request_from_json(o),
    );

    let reset = json!({ "session_token": "tok", "id": "u-1", "new_password": "np" });
    let o = reset.as_object().expect("object");
    assert_decode_eq(
        "ResetPasswordRequest",
        generated::decode_reset_password_request(o),
        hand::hand_reset_password_request_from_json(o),
    );
}

#[test]
fn auth_capabilities_decode_byte_identical() {
    let get_user = json!({ "session_token": "tok", "id": "u-1", "correlation_id": 2 });
    let o = get_user.as_object().expect("object");
    assert_decode_eq(
        "GetUserCapabilitiesRequest",
        generated::decode_get_user_capabilities_request(o),
        hand::hand_get_user_capabilities_request_from_json(o),
    );

    let set_user = json!({ "session_token": "tok", "id": "u-1",
        "grants": [{ "action": "price", "asset": "fx_options" }],
        "denies": [{ "action": "execute", "asset": "fixed_income" }] });
    let o = set_user.as_object().expect("object");
    assert_decode_eq(
        "SetUserCapabilitiesRequest",
        generated::decode_set_user_capabilities_request(o),
        hand::hand_set_user_capabilities_request_from_json(o),
    );
    // Absent grants/denies ⇒ empty lists.
    let set_user_min = json!({ "session_token": "tok", "id": "u-1" });
    let o = set_user_min.as_object().expect("object");
    assert_decode_eq(
        "SetUserCapabilitiesRequest(empty)",
        generated::decode_set_user_capabilities_request(o),
        hand::hand_set_user_capabilities_request_from_json(o),
    );

    let get_role = json!({ "session_token": "tok", "role": 2 });
    let o = get_role.as_object().expect("object");
    assert_decode_eq(
        "GetRoleCapabilitiesRequest",
        generated::decode_get_role_capabilities_request(o),
        hand::hand_get_role_capabilities_request_from_json(o),
    );

    let set_role = json!({ "session_token": "tok", "role": 2,
        "capabilities": [{ "action": "view", "asset": "fx_options" },
                         { "action": "quote_respond", "asset": "fx_options" }] });
    let o = set_role.as_object().expect("object");
    assert_decode_eq(
        "SetRoleCapabilitiesRequest",
        generated::decode_set_role_capabilities_request(o),
        hand::hand_set_role_capabilities_request_from_json(o),
    );
}

// --- decode: desk / entity / book --------------------------------------------

#[test]
fn auth_desk_crud_decode_byte_identical() {
    let list = json!({ "session_token": "tok", "correlation_id": 4 });
    let o = list.as_object().expect("object");
    assert_decode_eq(
        "ListDesksRequest",
        generated::decode_list_desks_request(o),
        hand::hand_list_desks_request_from_json(o),
    );

    let create = json!({ "session_token": "tok", "name": "FX Vol" });
    let o = create.as_object().expect("object");
    assert_decode_eq(
        "CreateDeskRequest",
        generated::decode_create_desk_request(o),
        hand::hand_create_desk_request_from_json(o),
    );

    let upd = json!({ "session_token": "tok", "id": "d-1", "name": "FX Vol EMEA" });
    let o = upd.as_object().expect("object");
    assert_decode_eq(
        "UpdateDeskRequest",
        generated::decode_update_desk_request(o),
        hand::hand_update_desk_request_from_json(o),
    );

    let del = json!({ "session_token": "tok", "id": "d-1" });
    let o = del.as_object().expect("object");
    assert_decode_eq(
        "DeleteDeskRequest",
        generated::decode_delete_desk_request(o),
        hand::hand_delete_desk_request_from_json(o),
    );
}

#[test]
fn auth_entity_crud_decode_byte_identical() {
    let list = json!({ "session_token": "tok" });
    let o = list.as_object().expect("object");
    assert_decode_eq(
        "ListEntitiesRequest",
        generated::decode_list_entities_request(o),
        hand::hand_list_entities_request_from_json(o),
    );

    // create: `key` optional (defaults to 0).
    for (label, body) in [
        (
            "with-key",
            json!({ "session_token": "tok", "name": "E", "code": "EEE", "key": 5 }),
        ),
        (
            "default-key",
            json!({ "session_token": "tok", "name": "E", "code": "EEE" }),
        ),
    ] {
        let o = body.as_object().expect("object");
        assert_decode_eq(
            &format!("CreateEntityRequest({label})"),
            generated::decode_create_entity_request(o),
            hand::hand_create_entity_request_from_json(o),
        );
    }

    let update = json!({ "session_token": "tok", "key": 5, "name": "E2", "code": "EE2" });
    let o = update.as_object().expect("object");
    assert_decode_eq(
        "UpdateEntityRequest",
        generated::decode_update_entity_request(o),
        hand::hand_update_entity_request_from_json(o),
    );

    let del = json!({ "session_token": "tok", "key": 5 });
    let o = del.as_object().expect("object");
    assert_decode_eq(
        "DeleteEntityRequest",
        generated::decode_delete_entity_request(o),
        hand::hand_delete_entity_request_from_json(o),
    );
}

#[test]
fn auth_book_crud_decode_byte_identical() {
    let list = json!({ "session_token": "tok" });
    let o = list.as_object().expect("object");
    assert_decode_eq(
        "ListBooksRequest",
        generated::decode_list_books_request(o),
        hand::hand_list_books_request_from_json(o),
    );

    for (label, body) in [
        (
            "with-key",
            json!({ "session_token": "tok", "name": "B", "entity_key": 7, "key": 3 }),
        ),
        (
            "default-key",
            json!({ "session_token": "tok", "name": "B", "entity_key": 7 }),
        ),
    ] {
        let o = body.as_object().expect("object");
        assert_decode_eq(
            &format!("CreateBookRequest({label})"),
            generated::decode_create_book_request(o),
            hand::hand_create_book_request_from_json(o),
        );
    }

    let update = json!({ "session_token": "tok", "key": 3, "name": "B2", "entity_key": 7 });
    let o = update.as_object().expect("object");
    assert_decode_eq(
        "UpdateBookRequest",
        generated::decode_update_book_request(o),
        hand::hand_update_book_request_from_json(o),
    );

    let del = json!({ "session_token": "tok", "key": 3 });
    let o = del.as_object().expect("object");
    assert_decode_eq(
        "DeleteBookRequest",
        generated::decode_delete_book_request(o),
        hand::hand_delete_book_request_from_json(o),
    );
}

// --- decode: instrument registry (the `definition` family oneof) -------------

fn instrument_body(family: Value) -> Value {
    let mut inst = json!({
        "instrument_id": "usd-inst", "name": "USD Instrument",
        "description": "a calibrating instrument", "currency": "USD",
        "external_ids": [ { "scheme": "isin", "value": "US0000000000" } ]
    });
    let (key, body) = family
        .as_object()
        .expect("family object")
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v.clone()))
        .expect("one family arm");
    inst.as_object_mut().expect("object").insert(key, body);
    inst
}

#[test]
fn auth_instrument_list_get_delete_decode_byte_identical() {
    let list = json!({ "session_token": "tok" });
    let o = list.as_object().expect("object");
    assert_decode_eq(
        "ListInstrumentsRequest",
        generated::decode_list_instruments_request(o),
        hand::hand_list_instruments_request_from_json(o),
    );

    let get = json!({ "session_token": "tok", "instrument_id": "usd-inst" });
    let o = get.as_object().expect("object");
    assert_decode_eq(
        "GetInstrumentRequest",
        generated::decode_get_instrument_request(o),
        hand::hand_get_instrument_request_from_json(o),
    );

    let del = json!({ "session_token": "tok", "instrument_id": "usd-inst" });
    let o = del.as_object().expect("object");
    assert_decode_eq(
        "DeleteInstrumentRequest",
        generated::decode_delete_instrument_request(o),
        hand::hand_delete_instrument_request_from_json(o),
    );
}

#[test]
fn auth_create_update_instrument_decode_byte_identical_all_families() {
    let families = [
        (
            "deposit",
            json!({ "deposit": { "index": "SOFR", "tenor": "3M", "day_count": "ACT/360",
                "business_day_convention": "MODFOLLOWING", "calendars": ["USD", "USNY"],
                "spot_lag_days": 2 } }),
        ),
        (
            "fra",
            json!({ "fra": { "float_index": "SOFR", "start_tenor": "3M", "end_tenor": "6M",
                "accrual_day_count": "ACT/360", "business_day_convention": "MODFOLLOWING",
                "calendars": ["USD"], "spot_lag_days": 2 } }),
        ),
        (
            "stir_future",
            json!({ "stir_future": { "contract_code": "SR3", "reference_start": "2026-06-17",
                "reference_end": "2026-09-16", "day_count": "ACT/360", "calendars": ["USD"],
                "convexity_vol": 0.006, "contract_size": 2500.0 } }),
        ),
        (
            "vanilla_irs",
            json!({ "vanilla_irs": { "tenor": "10Y", "fixed_frequency": "6M",
                "fixed_day_count": "30/360", "float_index": "SOFR", "float_frequency": "3M",
                "float_day_count": "ACT/360", "business_day_convention": "MODFOLLOWING",
                "calendars": ["USD"], "roll_convention": "EOM", "spot_lag_days": 2 } }),
        ),
        (
            "ois",
            json!({ "ois": { "tenor": "5Y", "index": "SOFR", "fixed_frequency": "1Y",
                "fixed_day_count": "ACT/360", "float_day_count": "ACT/360",
                "business_day_convention": "MODFOLLOWING", "calendars": ["USD"],
                "spot_lag_days": 2 } }),
        ),
        (
            "bond",
            json!({ "bond": { "issuer": "US TREASURY", "coupon_rate": 0.0425,
                "coupon_type": "FIXED", "coupon_frequency": "6M", "day_count": "ACT/ACT",
                "issue_date": { "year": 2026, "month": 5, "day": 15 },
                "maturity_date": { "year": 2036, "month": 5, "day": 15 },
                "redemption": 100.0, "calendars": ["USGS"] } }),
        ),
    ];
    for (label, family) in families {
        let create =
            json!({ "session_token": "tok", "instrument": instrument_body(family.clone()) });
        let o = create.as_object().expect("object");
        assert_decode_eq(
            &format!("CreateInstrumentRequest({label})"),
            generated::decode_create_instrument_request(o),
            hand::hand_create_instrument_request_from_json(o),
        );
        let update = json!({ "session_token": "tok", "instrument": instrument_body(family) });
        let o = update.as_object().expect("object");
        assert_decode_eq(
            &format!("UpdateInstrumentRequest({label})"),
            generated::decode_update_instrument_request(o),
            hand::hand_update_instrument_request_from_json(o),
        );
    }
    // A no-family instrument keeps `definition` unset (family_from_json ⇒ None).
    let no_family = json!({ "session_token": "tok",
        "instrument": { "name": "bare", "currency": "USD" } });
    let o = no_family.as_object().expect("object");
    assert_decode_eq(
        "CreateInstrumentRequest(no-family)",
        generated::decode_create_instrument_request(o),
        hand::hand_create_instrument_request_from_json(o),
    );
}

#[test]
fn auth_build_curve_decode_byte_identical() {
    let full = json!({
        "request_id": "req-1", "currency": "USD", "session_token": "tok",
        "reference_date": { "year": 2026, "month": 6, "day": 30 },
        "pillars": [ { "instrument_id": "usd-irs-10y", "quote": 0.0405 } ],
        "date_pillars": [ { "maturity_date": { "year": 2026, "month": 9, "day": 30 },
                            "quote": 0.0415 } ]
    });
    let o = full.as_object().expect("object");
    assert_decode_eq(
        "BuildCurveRequest(full)",
        generated::decode_build_curve_request(o),
        hand::hand_build_curve_request_from_json(o),
    );
    // Absent pillar arrays ⇒ empty.
    let min = json!({ "request_id": "req-2", "currency": "EUR", "session_token": "tok",
        "reference_date": { "year": 2026, "month": 6, "day": 30 } });
    let o = min.as_object().expect("object");
    assert_decode_eq(
        "BuildCurveRequest(no-pillars)",
        generated::decode_build_curve_request(o),
        hand::hand_build_curve_request_from_json(o),
    );
}

// --- encode: session + user + capabilities -----------------------------------

#[test]
fn auth_session_replies_encode_byte_identical() {
    let login = LoginResponse {
        session_token: "sess-abc".to_owned(),
        user: Some(a_user_desc(Some("fx"))),
        expires_nanos: 1_720_000_000_000_000_000,
        capabilities: vec![
            a_capability("view", "fx_options"),
            a_capability("price", "fixed_income"),
        ],
        correlation_id: Some(5),
    };
    assert_bytes_eq(
        "LoginResponse(full)",
        &generated::encode_login_response(&login),
        &hand::hand_login_response_to_json(&login),
    );
    // Absent user + correlation_id + no caps ⇒ user null, correlation_id null, [].
    let login0 = LoginResponse {
        session_token: "sess-x".to_owned(),
        user: None,
        expires_nanos: 0,
        capabilities: vec![],
        correlation_id: None,
    };
    let g = generated::encode_login_response(&login0);
    assert_eq!(g.get("user"), Some(&Value::Null));
    assert_eq!(g.get("correlation_id"), Some(&Value::Null));
    assert_eq!(g.get("capabilities"), Some(&Value::Array(vec![])));
    assert_bytes_eq(
        "LoginResponse(empty)",
        &g,
        &hand::hand_login_response_to_json(&login0),
    );

    for (label, correlation_id, ended) in [("with", Some(1_u64), true), ("null", None, false)] {
        let r = LogoutResponse {
            ended,
            correlation_id,
        };
        assert_bytes_eq(
            &format!("LogoutResponse({label})"),
            &generated::encode_logout_response(&r),
            &hand::hand_logout_response_to_json(&r),
        );
    }

    // ListUsersResponse: a user with desk_id None ⇒ nested `desk_id` renders `null`.
    let users = ListUsersResponse {
        users: vec![a_user_desc(Some("fx")), a_user_desc(None)],
        correlation_id: Some(2),
    };
    let g = generated::encode_list_users_response(&users);
    assert_eq!(g["users"][1].get("desk_id"), Some(&Value::Null));
    assert_bytes_eq(
        "ListUsersResponse",
        &g,
        &hand::hand_list_users_response_to_json(&users),
    );
    let users_empty = ListUsersResponse {
        users: vec![],
        correlation_id: None,
    };
    assert_bytes_eq(
        "ListUsersResponse(empty)",
        &generated::encode_list_users_response(&users_empty),
        &hand::hand_list_users_response_to_json(&users_empty),
    );
}

#[test]
fn auth_user_crud_replies_encode_byte_identical() {
    for (label, user) in [
        ("with-user", Some(a_user_desc(Some("fx")))),
        ("no-user", None),
    ] {
        let created = CreateUserResponse {
            user: user.clone(),
            correlation_id: Some(3),
        };
        assert_bytes_eq(
            &format!("CreateUserResponse({label})"),
            &generated::encode_create_user_response(&created),
            &hand::hand_create_user_response_to_json(&created),
        );
        let updated = UpdateUserResponse {
            user,
            correlation_id: None,
        };
        assert_bytes_eq(
            &format!("UpdateUserResponse({label})"),
            &generated::encode_update_user_response(&updated),
            &hand::hand_update_user_response_to_json(&updated),
        );
    }

    for (label, correlation_id, removed) in [("hit", Some(4_u64), true), ("miss", None, false)] {
        let del = DeleteUserResponse {
            removed,
            correlation_id,
        };
        assert_bytes_eq(
            &format!("DeleteUserResponse({label})"),
            &generated::encode_delete_user_response(&del),
            &hand::hand_delete_user_response_to_json(&del),
        );
    }

    for (label, correlation_id) in [("with", Some(7_u64)), ("null", None)] {
        let reset = ResetPasswordResponse { correlation_id };
        assert_bytes_eq(
            &format!("ResetPasswordResponse({label})"),
            &generated::encode_reset_password_response(&reset),
            &hand::hand_reset_password_response_to_json(&reset),
        );
    }
}

#[test]
fn auth_capabilities_replies_encode_byte_identical() {
    let caps = vec![
        a_capability("view", "fx_options"),
        a_capability("price", "fixed_income"),
    ];
    let get_user = GetUserCapabilitiesResponse {
        grants: caps.clone(),
        denies: vec![a_capability("execute", "fixed_income")],
        effective: caps.clone(),
        correlation_id: Some(1),
    };
    assert_bytes_eq(
        "GetUserCapabilitiesResponse",
        &generated::encode_get_user_capabilities_response(&get_user),
        &hand::hand_get_user_capabilities_response_to_json(&get_user),
    );
    let set_user = SetUserCapabilitiesResponse {
        grants: caps.clone(),
        denies: vec![],
        effective: caps.clone(),
        correlation_id: None,
    };
    assert_bytes_eq(
        "SetUserCapabilitiesResponse",
        &generated::encode_set_user_capabilities_response(&set_user),
        &hand::hand_set_user_capabilities_response_to_json(&set_user),
    );
    let get_role = GetRoleCapabilitiesResponse {
        capabilities: caps.clone(),
        correlation_id: Some(2),
    };
    assert_bytes_eq(
        "GetRoleCapabilitiesResponse",
        &generated::encode_get_role_capabilities_response(&get_role),
        &hand::hand_get_role_capabilities_response_to_json(&get_role),
    );
    let set_role = SetRoleCapabilitiesResponse {
        capabilities: caps,
        correlation_id: None,
    };
    assert_bytes_eq(
        "SetRoleCapabilitiesResponse",
        &generated::encode_set_role_capabilities_response(&set_role),
        &hand::hand_set_role_capabilities_response_to_json(&set_role),
    );
}

// --- encode: desk / entity / book --------------------------------------------

#[test]
fn auth_desk_replies_encode_byte_identical() {
    let desk = DeskDesc {
        id: "d-1".to_owned(),
        name: "FX Vol".to_owned(),
    };
    let list = ListDesksResponse {
        desks: vec![desk.clone()],
        correlation_id: Some(1),
    };
    assert_bytes_eq(
        "ListDesksResponse",
        &generated::encode_list_desks_response(&list),
        &hand::hand_list_desks_response_to_json(&list),
    );
    for (label, d) in [("with", Some(desk)), ("null", None)] {
        let created = CreateDeskResponse {
            desk: d,
            correlation_id: None,
        };
        assert_bytes_eq(
            &format!("CreateDeskResponse({label})"),
            &generated::encode_create_desk_response(&created),
            &hand::hand_create_desk_response_to_json(&created),
        );
    }
    let renamed = DeskDesc {
        id: "d-1".to_owned(),
        name: "FX Vol EMEA".to_owned(),
    };
    for (label, d) in [("with", Some(renamed)), ("null", None)] {
        let updated = UpdateDeskResponse {
            desk: d,
            correlation_id: Some(7),
        };
        assert_bytes_eq(
            &format!("UpdateDeskResponse({label})"),
            &generated::encode_update_desk_response(&updated),
            &hand::hand_update_desk_response_to_json(&updated),
        );
    }
    let del = DeleteDeskResponse {
        removed: true,
        correlation_id: Some(9),
    };
    assert_bytes_eq(
        "DeleteDeskResponse",
        &generated::encode_delete_desk_response(&del),
        &hand::hand_delete_desk_response_to_json(&del),
    );
}

#[test]
fn auth_entity_replies_encode_byte_identical() {
    let list = ListEntitiesResponse {
        entities: vec![an_entity_desc()],
        correlation_id: Some(1),
    };
    assert_bytes_eq(
        "ListEntitiesResponse",
        &generated::encode_list_entities_response(&list),
        &hand::hand_list_entities_response_to_json(&list),
    );
    for (label, e) in [("with", Some(an_entity_desc())), ("null", None)] {
        let created = CreateEntityResponse {
            entity: e.clone(),
            correlation_id: Some(2),
        };
        assert_bytes_eq(
            &format!("CreateEntityResponse({label})"),
            &generated::encode_create_entity_response(&created),
            &hand::hand_create_entity_response_to_json(&created),
        );
        let updated = UpdateEntityResponse {
            entity: e,
            correlation_id: None,
        };
        assert_bytes_eq(
            &format!("UpdateEntityResponse({label})"),
            &generated::encode_update_entity_response(&updated),
            &hand::hand_update_entity_response_to_json(&updated),
        );
    }
    let del = DeleteEntityResponse {
        removed: false,
        correlation_id: None,
    };
    assert_bytes_eq(
        "DeleteEntityResponse",
        &generated::encode_delete_entity_response(&del),
        &hand::hand_delete_entity_response_to_json(&del),
    );
}

#[test]
fn auth_book_replies_encode_byte_identical() {
    let list = ListBooksResponse {
        books: vec![a_book_desc()],
        correlation_id: Some(1),
    };
    assert_bytes_eq(
        "ListBooksResponse",
        &generated::encode_list_books_response(&list),
        &hand::hand_list_books_response_to_json(&list),
    );
    for (label, b) in [("with", Some(a_book_desc())), ("null", None)] {
        let created = CreateBookResponse {
            book: b.clone(),
            correlation_id: Some(2),
        };
        assert_bytes_eq(
            &format!("CreateBookResponse({label})"),
            &generated::encode_create_book_response(&created),
            &hand::hand_create_book_response_to_json(&created),
        );
        let updated = UpdateBookResponse {
            book: b,
            correlation_id: None,
        };
        assert_bytes_eq(
            &format!("UpdateBookResponse({label})"),
            &generated::encode_update_book_response(&updated),
            &hand::hand_update_book_response_to_json(&updated),
        );
    }
    let del = DeleteBookResponse {
        removed: true,
        correlation_id: None,
    };
    assert_bytes_eq(
        "DeleteBookResponse",
        &generated::encode_delete_book_response(&del),
        &hand::hand_delete_book_response_to_json(&del),
    );
}

// --- encode: instrument registry + build_curve -------------------------------

#[test]
fn auth_instrument_replies_encode_byte_identical_all_families() {
    let families = [
        ("deposit", deposit_def()),
        ("fra", fra_def()),
        ("stir_future", stir_future_def()),
        ("vanilla_irs", vanilla_irs_def()),
        ("ois", ois_def()),
        ("bond", bond_def()),
    ];
    let mut instruments = Vec::new();
    for (label, def) in families {
        let inst = instrument_def_with(def);
        // Via the single-instrument reply.
        let get = GetInstrumentResponse {
            instrument: Some(inst.clone()),
            correlation_id: Some(1),
        };
        assert_bytes_eq(
            &format!("GetInstrumentResponse({label})"),
            &generated::encode_get_instrument_response(&get),
            &hand::hand_get_instrument_response_to_json(&get),
        );
        // Create/Update replies carry the same nested projection.
        let created = CreateInstrumentResponse {
            instrument: Some(inst.clone()),
            correlation_id: None,
        };
        assert_bytes_eq(
            &format!("CreateInstrumentResponse({label})"),
            &generated::encode_create_instrument_response(&created),
            &hand::hand_create_instrument_response_to_json(&created),
        );
        let updated = UpdateInstrumentResponse {
            instrument: Some(inst.clone()),
            correlation_id: Some(3),
        };
        assert_bytes_eq(
            &format!("UpdateInstrumentResponse({label})"),
            &generated::encode_update_instrument_response(&updated),
            &hand::hand_update_instrument_response_to_json(&updated),
        );
        instruments.push(inst);
    }
    // The bond's absent coupon dates render `null`; its present ones render objects.
    let bond = GetInstrumentResponse {
        instrument: Some(instrument_def_with(bond_def())),
        correlation_id: None,
    };
    let gb = generated::encode_get_instrument_response(&bond);
    assert_eq!(
        gb["instrument"]["bond"].get("dated_date"),
        Some(&Value::Null)
    );
    assert_eq!(
        gb["instrument"]["bond"].get("first_coupon_date"),
        Some(&Value::Null)
    );
    assert!(gb["instrument"]["bond"]["issue_date"].is_object());
    // The list reply over every family.
    let list = ListInstrumentsResponse {
        instruments,
        correlation_id: Some(5),
    };
    assert_bytes_eq(
        "ListInstrumentsResponse",
        &generated::encode_list_instruments_response(&list),
        &hand::hand_list_instruments_response_to_json(&list),
    );
    // An absent instrument singular ⇒ null.
    let get_none = GetInstrumentResponse {
        instrument: None,
        correlation_id: None,
    };
    let gn = generated::encode_get_instrument_response(&get_none);
    assert_eq!(gn.get("instrument"), Some(&Value::Null));
    assert_bytes_eq(
        "GetInstrumentResponse(none)",
        &gn,
        &hand::hand_get_instrument_response_to_json(&get_none),
    );
    let del = DeleteInstrumentResponse {
        removed: true,
        correlation_id: None,
    };
    assert_bytes_eq(
        "DeleteInstrumentResponse",
        &generated::encode_delete_instrument_response(&del),
        &hand::hand_delete_instrument_response_to_json(&del),
    );
}

#[test]
fn auth_calibrated_curve_encode_byte_identical() {
    let curve = CalibratedCurve {
        request_id: "req-1".to_owned(),
        currency: "USD".to_owned(),
        reference_date: Some(a_broken_date(2026, 6, 30)),
        points: vec![
            CalibratedCurvePoint {
                instrument_id: "usd-depo-3m".to_owned(),
                time_years: 0.25,
                discount_factor: 0.9899,
                zero_rate: 0.0406,
                label: "3M".to_owned(),
            },
            CalibratedCurvePoint {
                instrument_id: "usd-irs-10y".to_owned(),
                time_years: 10.0,
                discount_factor: 0.665,
                zero_rate: 0.0408,
                label: "10Y".to_owned(),
            },
        ],
    };
    assert_bytes_eq(
        "CalibratedCurve(full)",
        &generated::encode_calibrated_curve(&curve),
        &hand::hand_calibrated_curve_to_json(&curve),
    );
    // Absent reference_date ⇒ `null`; empty points ⇒ `[]`.
    let bare = CalibratedCurve {
        request_id: "req-2".to_owned(),
        currency: "EUR".to_owned(),
        reference_date: None,
        points: vec![],
    };
    let g = generated::encode_calibrated_curve(&bare);
    assert_eq!(g.get("reference_date"), Some(&Value::Null));
    assert_eq!(g.get("points"), Some(&Value::Array(vec![])));
    assert_bytes_eq(
        "CalibratedCurve(bare)",
        &g,
        &hand::hand_calibrated_curve_to_json(&bare),
    );
}
