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
