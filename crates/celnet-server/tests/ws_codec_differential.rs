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
use serde_json::Value;

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
