//! WebSocket JSON-mirror integration tests.
//!
//! A browser/GUI client speaks the **same single contract** over WS that gRPC
//! clients speak over protobuf. These tests dial the edge's WS mirror with a real
//! `tokio-tungstenite` client and assert:
//!
//! 1. an RFQ over WS returns a quote whose price equals a first-principles
//!    Garman-Kohlhagen reference (the same number the gRPC/direct path produces —
//!    one pricing path, two encodings);
//! 2. an RFS subscriber over WS gets a `snapshot` then ≥2 sequenced `update` deltas,
//!    and a `resync` replays the missing sequence;
//! 3. click-to-trade over WS books an `executed` on a live token, then rejects a
//!    stale (already-consumed) token with a `stream_reject`.
//!
//! Every body is hard wall-clock bounded and every socket await is bounded, so a
//! regression fails fast rather than hanging.

mod common;

use std::time::Duration;

use celnet_core::is_close;
use celnet_types::{OptionType, VanillaInputs};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use common::{TEST_DEADLINE, live_market, start_ready_edge};

/// A bound on any single socket send/receive so a never-arriving frame fails fast.
const STEP: Duration = Duration::from_secs(5);

/// The wire conventions as a JSON object (proto enum numbers), mirroring
/// `common::wire_conventions`: spot-unadjusted Δ / ATM-forward / domestic-pips /
/// NY-cut / Act365 / deliverable.
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

/// A vanilla-call instrument at an absolute strike with a 1Y expiry, as the JSON a
/// browser client sends (mirrors `common::vanilla_call`).
fn vanilla_call_json(strike: f64) -> Value {
    json!({
        "pair": { "base": "EUR", "quote": "USD" },
        "tenor": { "unit": 3, "count": 1 },
        "expiry_years": 1.0,
        "quantity": { "notional": 1_000_000.0, "base_ccy": true },
        "side": 2,
        "vanilla": { "option_type": 0, "strike": { "strike": strike } }
    })
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
            WsMessage::Text(t) => return serde_json::from_str(&t).expect("frame is valid JSON"),
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
/// in-flight drain guard, letting `Edge::shutdown` return promptly (a live session
/// otherwise legitimately holds the drain open to the timeout).
async fn close_ws<S>(mut ws: S)
where
    S: SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let _ = tokio::time::timeout(STEP, ws.close()).await;
}

/// An RFQ over the WS mirror returns a quote whose two-way brackets the
/// first-principles GK mid — byte-identical to the gRPC/direct pricing path.
#[tokio::test]
async fn ws_rfq_quote_matches_direct_price() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        let strike = 1.12;
        send_json(
            &mut ws,
            json!({
                "type": "request_quote",
                "idempotency_key": "ws-rfq-1",
                "instrument": vanilla_call_json(strike),
                "conventions": conventions_json()
            }),
        )
        .await;

        let quote = next_json(&mut ws).await;
        assert_eq!(quote["type"], json!("quote"), "reply is a quote frame");
        assert!(quote["quote_id"].as_u64().unwrap() >= 1);

        // First-principles GK reference at the live ATM vol (the same market the
        // gRPC RFQ prices against).
        let m = live_market();
        let direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom(), m.r_for()),
        );
        let bid = quote["price"]["bid"].as_f64().unwrap();
        let offer = quote["price"]["offer"].as_f64().unwrap();
        assert!(bid < offer, "two-way is bid < offer");
        assert!(
            bid <= direct && direct <= offer,
            "mid {direct} must sit inside the two-way [{bid}, {offer}]"
        );
        let mid = 0.5 * (bid + offer);
        assert!(
            is_close(mid, direct, 1e-9, 1e-9) || bid == 0.0,
            "recovered WS mid {mid} vs direct {direct}"
        );

        // Accept over WS (BUY lifts the offer) and book the execution.
        let quote_id = quote["quote_id"].as_u64().unwrap();
        send_json(
            &mut ws,
            json!({
                "type": "accept_quote",
                "quote_id": quote_id,
                "idempotency_key": "ws-rfq-1",
                "side": 0
            }),
        )
        .await;
        let exec = next_json(&mut ws).await;
        assert_eq!(exec["type"], json!("execution"));
        assert_eq!(exec["quote_id"].as_u64().unwrap(), quote_id);
        assert!(is_close(
            exec["traded_premium"].as_f64().unwrap(),
            offer,
            1e-12,
            1e-12
        ));

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An RFS subscriber over WS receives a `snapshot` then ≥2 sequenced `update`
/// deltas, and a `resync` replays the missing sequence (or a fresh baseline).
#[tokio::test]
async fn ws_rfs_snapshot_deltas_and_resync() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        send_json(
            &mut ws,
            json!({
                "type": "subscribe",
                "subscription": { "value": 7 },
                "instrument": vanilla_call_json(1.12),
                "conventions": conventions_json()
            }),
        )
        .await;

        // First: the baseline snapshot at sequence 1 for our subscription.
        let snap = next_json(&mut ws).await;
        assert_eq!(snap["type"], json!("snapshot"));
        assert_eq!(snap["subscription"]["value"].as_u64().unwrap(), 7);
        assert_eq!(snap["sequence"].as_u64().unwrap(), 1);
        assert!(
            !snap["tradable"].as_array().unwrap().is_empty(),
            "snapshot stamps click-to-trade tokens"
        );

        // Then ≥2 sequenced update deltas (heartbeats may interleave).
        let mut last_seq = 1u64;
        let mut updates = 0;
        while updates < 2 {
            let m = next_json(&mut ws).await;
            match m["type"].as_str().unwrap() {
                "update" => {
                    let seq = m["sequence"].as_u64().unwrap();
                    assert_eq!(seq, last_seq + 1, "updates are strictly sequenced");
                    last_seq = seq;
                    updates += 1;
                }
                "heartbeat" => {}
                other => panic!("unexpected stream frame: {other}"),
            }
        }
        assert!(last_seq >= 3, "saw snapshot + ≥2 deltas");

        // Resync from sequence 1: the server replays the missing sequence 2 (or a
        // fresh baseline snapshot).
        send_json(
            &mut ws,
            json!({
                "type": "resync",
                "subscription": { "value": 7 },
                "last_sequence": 1
            }),
        )
        .await;
        let mut recovered = false;
        for _ in 0..64 {
            let m = next_json(&mut ws).await;
            match m["type"].as_str().unwrap() {
                "update" if m["sequence"].as_u64() == Some(2) => {
                    assert_eq!(m["subscription"]["value"].as_u64().unwrap(), 7);
                    recovered = true;
                    break;
                }
                "snapshot" => {
                    assert_eq!(m["subscription"]["value"].as_u64().unwrap(), 7);
                    recovered = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(
            recovered,
            "resync must replay the missing sequence 2 (or a fresh baseline)"
        );

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An RFS subscriber over WS observes the who's-trading attribution chain on the
/// baseline snapshot. The server unconditionally stamps the maker auto-pricer seat
/// (see `services::attribution::resolve`), and the WS JSON encoder must surface it
/// with the same camelCase keys the GUI/Excel decoders read — proving the WS path
/// carries attribution end-to-end, not just the gRPC path.
#[tokio::test]
async fn ws_snapshot_carries_maker_attribution() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        send_json(
            &mut ws,
            json!({
                "type": "subscribe",
                "subscription": { "value": 11 },
                "instrument": vanilla_call_json(1.12),
                "conventions": conventions_json()
            }),
        )
        .await;

        let snap = next_json(&mut ws).await;
        assert_eq!(snap["type"], json!("snapshot"));
        let attr = &snap["attribution"];
        assert!(
            attr.is_object(),
            "the WS snapshot must carry the attribution chain, got: {snap}"
        );
        // The maker seat is an auto-pricer (the demo edge auto-quotes). The chain's
        // `quotedBy.owner.autoPricer` identifies the seat that showed the market.
        let seat = &attr["quotedBy"]["owner"]["autoPricer"];
        assert!(
            seat.is_string() && !seat.as_str().unwrap().is_empty(),
            "snapshot attribution names the quoting auto-pricer seat, got: {attr}"
        );

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Click-to-trade over WS books an `executed` on a live token, then rejects a
/// replay of the same (now consumed) token with a `stream_reject`.
#[tokio::test]
async fn ws_click_to_trade_books_then_rejects_stale_token() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        send_json(
            &mut ws,
            json!({
                "type": "subscribe",
                "subscription": { "value": 11 },
                "instrument": vanilla_call_json(1.10),
                "conventions": conventions_json()
            }),
        )
        .await;

        // Grab a live BUY token off the baseline snapshot.
        let snap = next_json(&mut ws).await;
        assert_eq!(snap["type"], json!("snapshot"));
        let tradable = snap["tradable"].as_array().unwrap();
        let buy = tradable
            .iter()
            .find(|t| t["side"].as_i64() == Some(0)) // SIDE_BUY = 0
            .expect("a BUY token on the snapshot");
        let token = buy["token"].as_u64().unwrap();
        let premium = buy["premium"].as_f64().unwrap();

        // Click-to-trade: present the live token; expect an `executed` at the stamped
        // premium. The live tick also emits updates, so scan for the executed frame.
        send_json(
            &mut ws,
            json!({
                "type": "execute",
                "subscription": { "value": 11 },
                "token": token,
                "idempotency_key": "ws-click-1"
            }),
        )
        .await;
        let mut booked_premium = None;
        for _ in 0..64 {
            let m = next_json(&mut ws).await;
            if m["type"] == json!("executed") && m["token"].as_u64() == Some(token) {
                assert_eq!(m["side"].as_i64().unwrap(), 0, "BUY booked");
                booked_premium = m["traded_premium"].as_f64();
                break;
            }
        }
        let booked = booked_premium.expect("the live token books an executed");
        assert!(
            is_close(booked, premium, 1e-12, 1e-12),
            "booked premium {booked} != stamped {premium}"
        );

        // Replay the SAME (now consumed) token with a different key: rejected as
        // already-consumed (a stale token never re-books).
        send_json(
            &mut ws,
            json!({
                "type": "execute",
                "subscription": { "value": 11 },
                "token": token,
                "idempotency_key": "ws-click-2"
            }),
        )
        .await;
        let mut rejected = false;
        for _ in 0..64 {
            let m = next_json(&mut ws).await;
            if m["type"] == json!("stream_reject") && m["token"].as_u64() == Some(token) {
                // REASON_ALREADY_CONSUMED = 2.
                assert_eq!(
                    m["reason"].as_i64().unwrap(),
                    2,
                    "a consumed token is rejected as already-consumed"
                );
                rejected = true;
                break;
            }
        }
        assert!(rejected, "a stale token must be rejected, never re-booked");

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// A browser-style client opens a market-series (TrendMode) feed over the WS
/// mirror and receives the same real observed series the gRPC client gets — proving
/// API-first parity: the GUI/WS, SDK/gRPC, and Excel all consume the one contract.
#[tokio::test]
async fn ws_market_series_emits_real_observed_points() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        // Open an ATM-vol series (MarketObservable::AtmVol == 0) for EURUSD.
        send_json(
            &mut ws,
            json!({
                "type": "market_series_subscribe",
                "subscription": { "value": 31 },
                "pair": { "base": "EUR", "quote": "USD" },
                "observable": 0,
                "throttle_nanos": 0,
                "history_limit": 0
            }),
        )
        .await;

        // First: the opening market-series snapshot with one observed point.
        let snap = next_json(&mut ws).await;
        assert_eq!(snap["type"], json!("market_series_snapshot"));
        let pts = snap["points"].as_array().expect("snapshot points");
        assert_eq!(pts.len(), 1, "seeded with one observed point");
        let v0 = pts[0]["value"].as_f64().expect("a numeric value");
        assert!(v0 > 0.0 && v0 < 1.0, "a genuine ATM vol observation: {v0}");

        // Then ≥2 appended live points, each a real observation.
        let mut points = 0;
        while points < 2 {
            let f = next_json(&mut ws).await;
            if f["type"] == json!("market_series_point") {
                let v = f["value"].as_f64().expect("numeric value");
                assert!(
                    v > 0.0 && v < 1.0,
                    "appended point is a real observation: {v}"
                );
                points += 1;
            }
        }

        close_ws(ws).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
