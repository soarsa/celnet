//! Fixed-income RFQ parity on the WS/gRPC `QuoteService` (`rates-rfq-ws`).
//!
//! Brings request-for-quote parity to the contract surface: FI RFQ previously
//! existed only over FIX, and the WS/gRPC `QuoteService` was options-`Instrument`
//! only. These tests exercise `QuoteService.RequestRatesQuote` end to end and prove:
//!
//! 1. **WS round-trip** — a `RatesQuoteRequest` JSON body decodes through the
//!    descriptor-driven generated codec to the intended `RatesInstrument`
//!    field-for-field (the client wire is faithful);
//! 2. **end-to-end FI RFQ quote** — the WS FI RFQ prices through the server to a
//!    two-way whose **mid** equals the LANDED `price_rates` par rate (OIS) /
//!    `quote_bond` clean price (cash bond) to `<= 1e-12` — the taker two-way rides
//!    the one authoritative FI pricing path, no second implementation
//!    (non-circular: the reference is the independently-invoked landed engine);
//! 3. **gRPC parity** — the same `RequestRatesQuote` over the generated gRPC client
//!    returns the identical quote (one contract, two encodings).
//!
//! Every body is hard wall-clock bounded and every socket await is bounded.

mod common;

use std::time::Duration;

use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::{
    BondInstrument, BrokenDate, CurveSet, OisInstrument, OisPillar, PillarTenor, RatesInstrument,
    RatesPriceRequest, RatesQuoteRequest, Side, pillar_tenor, rates_instrument,
};
use celnet_server::rates_pricing::{price_rates, quote_bond};
use celnet_server::ws::generated_codec;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use common::{TEST_DEADLINE, start_ready_edge};

const STEP: Duration = Duration::from_secs(20);
const NOTIONAL: f64 = 100_000_000.0;

// ---------------------------------------------------------------------------
// shared market: one small USD-SOFR ladder built identically as JSON (the wire a
// client sends) and as the proto `CurveSet` (the independent-reference input).
// ---------------------------------------------------------------------------

fn curve_json() -> Value {
    json!({
        "currency": "USD",
        "reference_date": { "year": 2026, "month": 6, "day": 25 },
        "ois_pillars": [
            { "tenor": { "years": 1 }, "par_rate": 0.0420 },
            { "tenor": { "years": 2 }, "par_rate": 0.0410 },
            { "tenor": { "years": 5 }, "par_rate": 0.0405 },
            { "tenor": { "years": 10 }, "par_rate": 0.0415 }
        ]
    })
}

fn years_pillar(years: u32, par_rate: f64) -> OisPillar {
    OisPillar {
        tenor: Some(PillarTenor {
            point: Some(pillar_tenor::Point::Years(years)),
        }),
        par_rate,
    }
}

fn curve_proto() -> CurveSet {
    CurveSet {
        currency: "USD".to_owned(),
        reference_date: Some(BrokenDate {
            year: 2026,
            month: 6,
            day: 25,
        }),
        ois_pillars: vec![
            years_pillar(1, 0.0420),
            years_pillar(2, 0.0410),
            years_pillar(5, 0.0405),
            years_pillar(10, 0.0415),
        ],
    }
}

/// The OIS instrument JSON (the wire arm `side` is 0; the RFQ envelope `side`
/// governs the risk sign, so this arm side is intentionally distinct).
fn ois_json() -> Value {
    json!({ "ois": { "tenor_years": 5, "fixed_rate": 0.04, "notional": NOTIONAL, "side": 0 } })
}

fn ois_proto(side: Side) -> OisInstrument {
    OisInstrument {
        tenor_years: 5,
        fixed_rate: 0.04,
        notional: NOTIONAL,
        side: side as i32,
    }
}

fn bond_json() -> Value {
    json!({
        "bond": {
            "coupon_rate": 0.05,
            "coupon_frequency": 1,
            "day_count": 2,
            "maturity_date": { "year": 2031, "month": 6, "day": 25 },
            "redemption": 100.0,
            "side": 0
        }
    })
}

fn bond_proto(side: Side) -> BondInstrument {
    BondInstrument {
        coupon_rate: 0.05,
        coupon_frequency: 1, // PAYMENT_FREQUENCY_SEMI_ANNUAL
        day_count: 2,        // ACCRUAL_BASIS_THIRTY_360_BOND_BASIS
        maturity_date: Some(BrokenDate {
            year: 2031,
            month: 6,
            day: 25,
        }),
        redemption: 100.0,
        side: side as i32,
    }
}

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
            WsMessage::Text(t) => return serde_json::from_str(&t).expect("frame is valid JSON"),
            WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
            other => panic!("unexpected non-text WS frame: {other:?}"),
        }
    }
}

async fn send_json<S>(ws: &mut S, v: Value)
where
    S: SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    tokio::time::timeout(STEP, ws.send(WsMessage::Text(v.to_string())))
        .await
        .expect("the send completes in time")
        .expect("the send succeeds");
}

/// (1) The `RatesQuoteRequest` wire body decodes through the generated codec to the
/// intended `RatesInstrument` / envelope field-for-field — the FI RFQ round-trip.
#[test]
fn rates_quote_request_decodes_field_for_field() {
    let body = json!({
        "type": "request_rates_quote",
        "idempotency_key": "fi-rfq-decode",
        "curve_set": curve_json(),
        "instrument": ois_json(),
        "notional": NOTIONAL,
        "side": 2,
        "correlation_id": 77
    });
    let o = body.as_object().expect("request object");
    let req = generated_codec::decode_rates_quote_request(o).expect("decodes");

    assert_eq!(req.idempotency_key, "fi-rfq-decode");
    assert_eq!(req.notional, NOTIONAL);
    assert_eq!(req.side, Side::TwoWay as i32);
    assert_eq!(req.correlation_id, Some(77));
    assert_eq!(req.curve_set.as_ref(), Some(&curve_proto()));
    // The intended instrument arm, field-for-field (the wire arm side is 0/BUY).
    assert_eq!(
        req.instrument,
        Some(RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(ois_proto(Side::Buy))),
        })
    );
}

/// (2) End-to-end WS OIS RFQ: the two-way mid equals the LANDED `price_rates` par
/// rate to `<= 1e-12` (the taker two-way rides the one FI pricing path).
#[tokio::test]
async fn ws_rates_quote_ois_mid_matches_price_rates() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc, _dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        send_json(
            &mut ws,
            json!({
                "type": "request_rates_quote",
                "idempotency_key": "ws-fi-ois-1",
                "curve_set": curve_json(),
                "instrument": ois_json(),
                "notional": NOTIONAL,
                "side": 2
            }),
        )
        .await;
        let quote = next_json(&mut ws).await;
        assert_eq!(quote["type"], json!("rates_quote"));
        assert!(quote["quote_id"].as_u64().unwrap() >= 1);
        assert_eq!(quote["notional"].as_f64().unwrap(), NOTIONAL);
        assert!(
            quote["valid_until_nanos"].as_i64().unwrap() > quote["epoch_nanos"].as_i64().unwrap()
        );

        let bid = quote["price"]["bid"].as_f64().unwrap();
        let offer = quote["price"]["offer"].as_f64().unwrap();
        assert!(bid < offer, "two-way is bid < offer");
        let mid = 0.5 * (bid + offer);

        // Independent reference: the LANDED price_rates par rate (side-independent).
        let reference = price_rates(&RatesPriceRequest {
            request_id: 0,
            curve_set: Some(curve_proto()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(ois_proto(Side::Sell))),
            }),
            correlation_id: None,
        })
        .expect("reference prices")
        .par_rate;

        assert!(
            (mid - reference).abs() <= 1e-12,
            "WS OIS RFQ mid {mid} vs price_rates par {reference}"
        );
        // The reply's own risk result carries the same par rate.
        assert!((quote["result"]["par_rate"].as_f64().unwrap() - reference).abs() <= 1e-12);
        // dv01 present (the full FI risk rides the reply).
        assert!(quote["result"]["dv01"].as_f64().is_some());

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// (2b) End-to-end WS cash-bond RFQ: the two-way mid equals the LANDED `quote_bond`
/// clean price to `<= 1e-12` — the bond clean-price machinery on the WS surface.
#[tokio::test]
async fn ws_rates_quote_bond_mid_matches_quote_bond_clean_price() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc, _dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        send_json(
            &mut ws,
            json!({
                "type": "request_rates_quote",
                "idempotency_key": "ws-fi-bond-1",
                "curve_set": curve_json(),
                "instrument": bond_json(),
                "notional": 25_000_000.0,
                "side": 2
            }),
        )
        .await;
        let quote = next_json(&mut ws).await;
        assert_eq!(quote["type"], json!("rates_quote"));
        let bid = quote["price"]["bid"].as_f64().unwrap();
        let offer = quote["price"]["offer"].as_f64().unwrap();
        assert!(bid < offer, "bond two-way is bid < offer");
        let mid = 0.5 * (bid + offer);

        let reference = quote_bond(&bond_proto(Side::TwoWay), &curve_proto())
            .expect("reference bond prices")
            .clean_price;
        assert!(
            (mid - reference).abs() <= 1e-12,
            "WS bond RFQ mid {mid} vs quote_bond clean {reference}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// (3) gRPC parity: `RequestRatesQuote` over the generated client returns the same
/// two-way, its mid equal to the landed par rate (one contract, two encodings).
#[tokio::test]
async fn grpc_rates_quote_mid_matches_price_rates() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr, _dir) = start_ready_edge().await;
        let mut client =
            tokio::time::timeout(STEP, QuoteServiceClient::connect(format!("http://{addr}")))
                .await
                .expect("client connects in time")
                .expect("client connects");

        let quote = tokio::time::timeout(
            STEP,
            client.request_rates_quote(RatesQuoteRequest {
                idempotency_key: "grpc-fi-ois-1".to_owned(),
                curve_set: Some(curve_proto()),
                instrument: Some(RatesInstrument {
                    instrument: Some(rates_instrument::Instrument::Ois(ois_proto(Side::Buy))),
                }),
                notional: NOTIONAL,
                side: Side::TwoWay as i32,
                correlation_id: Some(5),
            }),
        )
        .await
        .expect("request_rates_quote returns in time")
        .expect("request_rates_quote succeeds")
        .into_inner();

        assert!(quote.quote_id >= 1);
        assert_eq!(quote.correlation_id, Some(5));
        let price = quote.price.expect("two-way present");
        let mid = 0.5 * (price.bid + price.offer);
        let reference = price_rates(&RatesPriceRequest {
            request_id: 0,
            curve_set: Some(curve_proto()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(ois_proto(Side::Sell))),
            }),
            correlation_id: None,
        })
        .expect("reference prices")
        .par_rate;
        assert!(
            (mid - reference).abs() <= 1e-12,
            "gRPC OIS RFQ mid {mid} vs price_rates par {reference}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
