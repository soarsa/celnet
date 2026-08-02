//! Cross-asset (equity / commodity / digital-asset) WS-mirror conformance.
//!
//! The server's cross-asset WS decode arms (`underlying_object_from_json` →
//! `price_cross_asset`) previously had ZERO end-to-end test coverage (the SDK
//! conformance drives them over gRPC only, and against a synthetic shared market,
//! not the frozen corpus). This suite drives **every frozen cross-asset golden
//! vector** (`crates/celnet-golden/vectors/{equity_option,commodity_option,
//! crypto_option}.json`) through the REAL WS mirror with a real
//! `tokio-tungstenite` client:
//!
//! 1. JSON `price` frame (cross-asset `underlying` object, the vector's OWN
//!    market) → decode → cross-asset cost-of-carry leaf → reconcile the reply to
//!    the vector's INDEPENDENT oracle within its frozen `(rel, abs)` band — the
//!    same gate the SDK/Excel/GUI corpus suites apply. Both crypto settlement
//!    styles (LINEAR and the coin-margined INVERSE_COIN `1/S_T`) are asserted
//!    exercised, and every vector's market is the FX two-rate projection
//!    `{spot, vol, r_dom = r, r_for = carry yield}` — proving the WS
//!    `MarketContext` DOES transport the cross-asset corpus (the carry guard
//!    derives `b = r_dom − r_for`, the vectors' exact net carry).
//! 2. The route is PROVEN, not assumed: a delta-keyed strike is an FX-convention
//!    construct the cross-asset path refuses with a typed error while the FX
//!    path would resolve it — so the refusal discriminates the routing for the
//!    linear payoffs whose price alone is numerically identical to the FX
//!    projection (ADR-0008: asset class is payoff identity).
//! 3. PRODUCTION-FRAME ROUTING: the WS decoder treats the richer `underlying`
//!    oneof as authoritative over the legacy FX `pair` projection. A production
//!    client-shaped frame carries the FX `pair` projection BESIDE `underlying`
//!    (so the FX-keyed surfaces stay total — `instrumentToWire` in both clients);
//!    it now decodes to the cross-asset arm and prices to the cross-asset oracle.
//!    The INVERSE_COIN frame is the sharpest proof: it routes to the digital-asset
//!    arm with its `settlement_style` honoured and prices to the coin-margined
//!    `1/S_T` oracle, NOT the LINEAR (USD) value four orders of magnitude away.
//!
//! Helpers mirror the `ws_mirror.rs` harness pattern: every body is hard
//! wall-clock bounded and every socket await is bounded, so a regression fails
//! fast rather than hanging.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use celnet_golden::{CROSS_ASSET_FAMILIES, GoldenVector, load_cross_asset_vectors};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use common::start_ready_edge;

/// A bound on any single socket send/receive — a HANG detector, not a latency
/// gate (the latency budgets are gated in `celnet-bench` on a quiet machine).
/// Sized for full `cargo test --workspace` parallel load, where every test
/// binary competes for the same cores; the closed-form cross-asset prices
/// themselves are sub-millisecond server-side.
const STEP: Duration = Duration::from_secs(30);

/// The hard wall-clock ceiling for one test body (edge boot + the whole
/// closed-form corpus loop over one socket). Same hang-detector doctrine as
/// `STEP`; a genuine hang still surfaces fast relative to the gate budget.
const SUITE_DEADLINE: Duration = Duration::from_secs(120);

/// The wire conventions as a JSON object (proto enum zeros), mirroring
/// `ws_mirror::conventions_json`: spot-unadjusted Δ / ATM-forward /
/// domestic-pips / NY-cut / Act365 / deliverable. The cross-asset path prices
/// absolute strikes only, so no convention enters the price.
fn conventions_json() -> Value {
    json!({
        "delta_convention": 0,
        "atm_convention": 0,
        "premium_style": 0,
        "cut": 0,
        "day_count": 0,
        "settlement": 0
    })
}

/// The wire `underlying` object for a vector — the cross-asset arm WITHOUT the
/// legacy FX `pair` key (the canonical cross-asset form, mirroring the gRPC
/// `Underlying` oneof). The listing venue/currency are contract identity only
/// (ADR-0008 — the payoff is asset-class-agnostic over the carry-producing
/// market); every priced input lives in the vector's market context.
fn underlying_json(v: &GoldenVector) -> Value {
    match v.family.as_str() {
        "equity_option" => {
            let (venue, currency) = match v.underlying.as_str() {
                "SPX" => ("XCBO", "USD"),
                "AAPL" => ("XNAS", "USD"),
                "STOXX" => ("XEUR", "EUR"),
                other => panic!("vector {}: unmapped equity underlying `{other}`", v.id),
            };
            json!({
                "equity": {
                    "symbol": { "ticker": v.underlying, "venue": venue },
                    "currency": currency
                }
            })
        }
        // Venue-less contract identity (the Excel `TICKER@:CCY` commodity form);
        // the corpus commodities (WTI / GOLD) quote in USD.
        "commodity_option" => json!({
            "commodity": { "symbol": { "ticker": v.underlying }, "currency": "USD" }
        }),
        "crypto_option" => {
            assert_eq!(
                v.underlying.len(),
                6,
                "vector {}: crypto underlying `{}` is not a 3+3 pair token",
                v.id,
                v.underlying
            );
            let (base, quote) = v.underlying.split_at(3);
            json!({ "digital_asset": { "base": base, "quote": quote } })
        }
        other => panic!("vector {}: `{other}` is not a cross-asset family", v.id),
    }
}

/// The vector's wire instrument: a vanilla over the cross-asset arm at the
/// vector's absolute strike and exact `expiry_years`, two-way, unit notional.
/// LINEAR settlement is presence-omitted (the proto3 zero, the canonical client
/// encoding); INVERSE_COIN selects the coin-margined `1/S_T` crypto convention.
fn instrument_json(v: &GoldenVector) -> Value {
    let option_type = match v.term_str("option_type") {
        "CALL" => celnet_proto::OptionType::Call as i32,
        "PUT" => celnet_proto::OptionType::Put as i32,
        other => panic!("vector {}: unknown option_type `{other}`", v.id),
    };
    let mut instrument = json!({
        "underlying": underlying_json(v),
        "expiry_years": v.term_f64("expiry_years"),
        "quantity": { "notional": 1.0, "base_ccy": true },
        "side": celnet_proto::Side::TwoWay as i32,
        "vanilla": {
            "option_type": option_type,
            "strike": { "strike": v.term_f64("strike") }
        }
    });
    if v.terms.get("settlement_style").and_then(Value::as_str) == Some("INVERSE_COIN") {
        instrument["settlement_style"] = json!(celnet_proto::SettlementStyle::InverseCoin as i32);
    }
    instrument
}

/// The vector's own market as the WS JSON `market` object — the FX two-rate
/// projection (`r_dom` the discount rate, `r_for` the asset's carry yield:
/// dividend+repo / convenience / funding), which the server's carry guard reads
/// as `CostOfCarry { r: r_dom, b: r_dom − r_for }`.
fn market_json(v: &GoldenVector) -> Value {
    json!({
        "spot": v.market.spot,
        "vol": v.market.vol,
        "r_dom": v.market.r_dom,
        "r_for": v.market.r_for
    })
}

/// Assert a WS-priced value against a vector's frozen closed-form band:
/// `|got − oracle| ≤ abs + rel · max(|got|, |oracle|)` — the same gate the
/// SDK/Excel/GUI corpus suites apply. Every cross-asset vector is closed-form
/// (no Monte-Carlo stderr), enforced rather than assumed.
fn assert_within_band(v: &GoldenVector, got: f64) {
    assert!(
        v.expected.price_std_error.is_none(),
        "vector {}: cross-asset vectors are closed-form (no MC stderr)",
        v.id
    );
    let want = v.expected.price;
    let band = v.tolerance.abs + v.tolerance.rel * got.abs().max(want.abs());
    assert!(
        (got - want).abs() <= band,
        "vector {}: WS-priced {got} vs independent oracle {want} (band {band})",
        v.id
    );
}

/// Receive the next JSON frame from the socket within the step deadline.
async fn next_json<S>(ws: &mut S) -> Value
where
    S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let frame = tokio::time::timeout(STEP, ws.next())
            .await
            .expect("a WS frame arrives before the deadline")
            .expect("the socket stays open")
            .expect("the frame is well-formed");
        match frame {
            WsMessage::Text(t) => {
                let v: Value = serde_json::from_str(&t).expect("frame is valid JSON");
                // Skip the unsolicited connect-time firm-wide pricing-control frame.
                if v.get("type").and_then(Value::as_str) == Some("pricing_control") {
                    continue;
                }
                return v;
            }
            WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
            other => panic!("unexpected non-text WS frame: {other:?}"),
        }
    }
}

/// Send a JSON value as a WS text frame within the step deadline.
async fn send_json<S>(ws: &mut S, v: Value)
where
    S: SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    tokio::time::timeout(STEP, ws.send(WsMessage::Text(v.to_string())))
        .await
        .expect("the send completes in time")
        .expect("the send succeeds");
}

/// Close the WS connection client-side so its session task ends and releases the
/// in-flight drain guard, letting `Edge::shutdown` return promptly.
async fn close_ws<S>(mut ws: S)
where
    S: SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let _ = tokio::time::timeout(STEP, ws.close()).await;
}

/// Send one one-shot `price` frame and return the typed reply frame.
async fn price_on_ws<S>(ws: &mut S, request_id: u64, instrument: Value, market: Value) -> Value
where
    S: SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error>
        + StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    send_json(
        &mut *ws,
        json!({
            "type": "price",
            "request_id": request_id,
            "instrument": instrument,
            "market": market,
            "conventions": conventions_json()
        }),
    )
    .await;
    next_json(&mut *ws).await
}

/// Every frozen cross-asset golden vector prices over the REAL WS mirror to its
/// independent oracle within the vector's frozen band: `underlying` decode →
/// `price_cross_asset` → the generalized cost-of-carry leaf, end-to-end. The
/// INVERSE_COIN vectors are the strongest routing proof on their own: an
/// FX-path misroute would return the LINEAR (USD) value, missing the
/// coin-denominated oracle by four orders of magnitude against a 1e-9 band.
#[tokio::test]
async fn ws_prices_cross_asset_vectors_to_their_independent_oracles() {
    tokio::time::timeout(SUITE_DEADLINE, async {
        let vectors = load_cross_asset_vectors().expect("the cross-asset corpus loads");
        assert!(
            !vectors.is_empty(),
            "the cross-asset corpus must not be empty"
        );

        let (edge, _grpc, _data_dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        let mut families: HashSet<&str> = HashSet::new();
        let mut styles: HashSet<&str> = HashSet::new();
        for (i, v) in vectors.iter().enumerate() {
            let request_id = i as u64 + 1;
            let reply = price_on_ws(&mut ws, request_id, instrument_json(v), market_json(v)).await;
            assert_eq!(
                reply["type"],
                json!("price_response"),
                "vector {}: expected a price_response, got: {reply}",
                v.id
            );
            assert_eq!(
                reply["request_id"].as_u64(),
                Some(request_id),
                "vector {}: the reply echoes its request id",
                v.id
            );
            let got = reply["greeks"]["price"]
                .as_f64()
                .unwrap_or_else(|| panic!("vector {}: reply carries no price: {reply}", v.id));
            assert_within_band(v, got);
            // Closed-form leaves disclose no MC stderr; a non-null value here
            // would mean the request was misrouted to a Monte-Carlo engine.
            assert!(
                reply["price_std_error"].is_null(),
                "vector {}: a closed-form cross-asset price must carry no MC stderr: {reply}",
                v.id
            );
            families.insert(v.family.as_str());
            if v.family == "crypto_option" {
                styles.insert(v.term_str("settlement_style"));
            }
        }

        // Reachability: every cross-asset family AND both crypto settlement
        // styles were actually exercised, never silently absent from the corpus.
        for family in CROSS_ASSET_FAMILIES {
            assert!(
                families.contains(family),
                "family `{family}` was never exercised over the WS mirror"
            );
        }
        for style in ["LINEAR", "INVERSE_COIN"] {
            assert!(
                styles.contains(style),
                "crypto settlement style `{style}` was never exercised over the WS mirror"
            );
        }

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// The cross-asset routing is PROVEN per family, not assumed from the price: a
/// delta-keyed strike is an FX-convention construct the cross-asset path refuses
/// with a typed `absolute strike` error, while the FX path would resolve it to a
/// strike and price. For the equity/commodity/linear-crypto vectors the priced
/// value alone cannot discriminate the route (ADR-0008 — the linear payoff is
/// asset-class-agnostic, so the FX projection prices identically); this typed
/// refusal can.
#[tokio::test]
async fn ws_routes_every_cross_asset_arm_to_the_cross_asset_path() {
    tokio::time::timeout(SUITE_DEADLINE, async {
        let vectors = load_cross_asset_vectors().expect("the cross-asset corpus loads");

        let (edge, _grpc, _data_dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        for (i, family) in CROSS_ASSET_FAMILIES.iter().enumerate() {
            let donor = vectors
                .iter()
                .find(|v| v.family == *family)
                .unwrap_or_else(|| panic!("no corpus vector for family `{family}`"));
            // The donor's instrument with the strike re-keyed by delta — exactly
            // one mutation, so the refusal can only come from the strike law.
            let mut instrument = instrument_json(donor);
            instrument["vanilla"]["strike"] = json!({ "delta": 0.25 });
            let request_id = i as u64 + 1;
            let reply =
                price_on_ws(&mut ws, request_id, instrument, market_json(donor)).await;
            assert_eq!(
                reply["type"],
                json!("error"),
                "family `{family}`: a delta strike on a cross-asset arm must be refused, got: {reply}"
            );
            assert_eq!(
                reply["code"],
                json!("InvalidArgument"),
                "family `{family}`: the refusal is a typed INVALID_ARGUMENT: {reply}"
            );
            let message = reply["message"].as_str().unwrap_or_default();
            assert!(
                message.contains("absolute strike"),
                "family `{family}`: the refusal names the cross-asset absolute-strike law \
                 (proving the cross-asset route), got: {message}"
            );
        }

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// The production client frame routes by the AUTHORITATIVE `underlying` oneof,
/// never the legacy FX `pair` projection. The GUI/Excel encoders ALWAYS emit the
/// FX `pair` projection beside `underlying` (so the FX-keyed surfaces stay total —
/// `instrumentToWire` in both clients); the server's WS decoder
/// (`instrument_underlying_from_json`) now treats the richer `underlying` as
/// authoritative when both are present.
///
/// The INVERSE_COIN crypto call is the sharpest proof of correct routing: an
/// FX-path misroute would honour neither the digital-asset arm NOR the
/// `settlement_style`, returning the LINEAR (USD-margined) value — four orders of
/// magnitude away from the coin-margined `1/S_T` oracle the frame asks for. This
/// asserts the frame WITH the legacy `pair` prices to the INVERSE_COIN vector's
/// own coin oracle (within its 1e-9 band) and is nowhere near the LINEAR call's
/// value. It is the server-asserted reason `equity_option`/`commodity_option`/
/// `crypto_option` are WS-priced in the Excel/GUI corpora (`excel/e2e/corpus.ts`,
/// `gui/e2e/goldenCorpus.ts`): the cross-asset arm survives the FX projection.
#[tokio::test]
async fn ws_underlying_precedence_routes_client_shaped_frames_to_the_cross_asset_arm() {
    tokio::time::timeout(SUITE_DEADLINE, async {
        let vectors = load_cross_asset_vectors().expect("the cross-asset corpus loads");
        let inverse_call = vectors
            .iter()
            .find(|v| {
                v.family == "crypto_option"
                    && v.term_str("settlement_style") == "INVERSE_COIN"
                    && v.term_str("option_type") == "CALL"
            })
            .expect("the corpus carries an INVERSE_COIN crypto call");
        let linear_call = vectors
            .iter()
            .find(|v| {
                v.family == "crypto_option"
                    && v.term_str("settlement_style") == "LINEAR"
                    && v.term_str("option_type") == "CALL"
            })
            .expect("the corpus carries a LINEAR crypto call");
        // The inverse/linear calls are the SAME contract modulo settlement style:
        // the only thing that can move the price between the two oracles is the
        // settlement economics, so the coin/linear divergence isolates the routing.
        assert_eq!(
            inverse_call.term_f64("strike"),
            linear_call.term_f64("strike"),
            "the corpus inverse/linear calls share a strike"
        );
        assert_eq!(
            inverse_call.term_f64("expiry_years"),
            linear_call.term_f64("expiry_years"),
            "the corpus inverse/linear calls share an expiry"
        );
        assert_eq!(
            inverse_call.market, linear_call.market,
            "the corpus inverse/linear calls share a market"
        );

        let (edge, _grpc, _data_dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        // The production client frame: the `digital_asset` arm + INVERSE_COIN
        // `settlement_style` PLUS the legacy FX `pair` projection
        // ({base: BTC, quote: USD}, which `Ccy::parse` accepts as 3-ASCII-letter
        // legs, so an FX misroute WOULD decode cleanly and silently mis-price).
        let mut instrument = instrument_json(inverse_call);
        let (base, quote) = inverse_call.underlying.split_at(3);
        instrument["pair"] = json!({ "base": base, "quote": quote });

        let reply = price_on_ws(&mut ws, 1, instrument, market_json(inverse_call)).await;
        assert_eq!(
            reply["type"],
            json!("price_response"),
            "the client-shaped frame prices over the cross-asset path: {reply}"
        );
        let got = reply["greeks"]["price"]
            .as_f64()
            .unwrap_or_else(|| panic!("reply carries no price: {reply}"));

        // `underlying` wins: the reply is the coin-margined `1/S_T` value of the
        // INVERSE_COIN contract, within that vector's own frozen 1e-9 band — the
        // `settlement_style` is honoured, the `pair` projection ignored.
        assert_within_band(inverse_call, got);
        // ...and nowhere near the LINEAR (USD-margined) value an FX misroute would
        // have returned (the two differ by four orders of magnitude — coin
        // fractions vs USD thousands — so a 1.0 margin is conservative both ways).
        assert!(
            (got - linear_call.expected.price).abs() > 1.0,
            "regression: the client-shaped INVERSE_COIN frame mis-routed to the FX/LINEAR \
             path ({got} vs the LINEAR oracle {}) — `underlying` must win over the legacy \
             `pair` projection in `instrument_underlying_from_json`",
            linear_call.expected.price
        );

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
