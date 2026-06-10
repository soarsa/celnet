//! Multi-dealer RFQ panel integration tests over the `celnet-proto`
//! `QuoteService` (`RequestMultiDealerQuote` → `MultiDealerQuote`,
//! `AcceptQuote` with a dealer `lp_id`) and the WS JSON mirror of the same
//! contract.
//!
//! Each test starts the edge on an ephemeral port with an **explicit**
//! [`celnet_server::LpPanelConfig`] (race-free — no process-global env mutation)
//! and asserts:
//!
//! 1. a ≥3-LP panel ranks per the engine law (best bid = max bid, best offer =
//!    min offer; ties prefer earlier epoch then smaller `lp_id`), with the
//!    synthetic dealers' two-ways re-derived independently from the maker quote;
//! 2. `AcceptQuote(quote_id, lp_id)` books exactly the **pinned** panel row —
//!    the row's price and LP attribution, never a re-price — idempotently and
//!    line-matched on retry;
//! 3. an empty `lp_id` stays the single-dealer path byte-identical (the stored
//!    quote's own price and maker attribution);
//! 4. an accept after a panel row's `valid_until_nanos` is refused exactly as
//!    the engine's last-look law;
//! 5. the WS mirror carries the same panel frame (snake_case, field-for-field)
//!    and books a dealer line via `accept_quote` + `lp_id`.
//!
//! Honest boundary: the panel dealers beyond the native maker are deterministic
//! **synthetic** demo/test LPs quoting around the same edge mid — live LP
//! connectivity is ENV and never claimed here. Every body is hard wall-clock
//! bounded and every network await is bounded, so a stall fails fast.

mod common;

use std::time::Duration;

use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::{DealerQuote, QuoteAccept, QuoteRequest, Side, owner};
use celnet_server::Clock;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use common::{
    STEP_DEADLINE, TEST_DEADLINE, start_panel_edge_with, start_ready_panel_edge, vanilla_call,
    wire_conventions,
};

/// The native maker's stable `lp_id` on the wire (the edge auto-pricer's identity
/// carried in `DealerQuote.lp_id` / attribution seats).
const MAKER_LP_ID: &str = "celnet-auto-pricer";

/// A quote request for the standard 1Y EURUSD vanilla call under `key`.
fn quote_request(key: &str) -> QuoteRequest {
    QuoteRequest {
        idempotency_key: key.to_owned(),
        instrument: Some(vanilla_call(1.12)),
        conventions: Some(wire_conventions()),
        correlation_id: None,
        surface_version: None,
        attribution: None,
    }
}

/// Dial the edge's `QuoteService` within the step deadline.
async fn quote_client(addr: std::net::SocketAddr) -> QuoteServiceClient<tonic::transport::Channel> {
    tokio::time::timeout(
        STEP_DEADLINE,
        QuoteServiceClient::connect(format!("http://{addr}")),
    )
    .await
    .expect("client connects in time")
    .expect("client connects")
}

/// One side's price on a panel row.
fn row_price(d: &DealerQuote, bid_side: bool) -> f64 {
    let p = d.price.expect("every panel row carries a two-way");
    if bid_side { p.bid } else { p.offer }
}

/// Independently re-derive one side's winner per the engine law over the
/// returned panel rows: best bid = **max** bid, best offer = **min** offer; on an
/// equal best price the earlier `epoch_nanos` wins — every row here shares the
/// aggregate quote's epoch, so that leg ties — then the lexicographically
/// smallest `lp_id`.
fn law_winner(dealers: &[DealerQuote], bid_side: bool) -> String {
    let mut best: Option<&DealerQuote> = None;
    for d in dealers {
        let better = match best {
            None => true,
            Some(b) => {
                let (dp, bp) = (row_price(d, bid_side), row_price(b, bid_side));
                if dp == bp {
                    d.lp_id < b.lp_id
                } else if bid_side {
                    dp > bp
                } else {
                    dp < bp
                }
            }
        };
        if better {
            best = Some(d);
        }
    }
    best.expect("panel has rows").lp_id.clone()
}

/// A ≥3-LP panel returns native + synthetic rows whose two-ways re-derive exactly
/// from the maker quote, ranked per the engine law (independently recomputed).
#[tokio::test]
async fn multi_dealer_panel_ranks_per_engine_law() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_panel_edge(3).await;
        let mut client = quote_client(addr).await;

        let key = "md-rank-1";
        let multi = tokio::time::timeout(
            STEP_DEADLINE,
            client.request_multi_dealer_quote(quote_request(key)),
        )
        .await
        .expect("multi-dealer request returns in time")
        .expect("multi-dealer request succeeds")
        .into_inner();

        // The single-dealer quote under the SAME idempotency key is the stored
        // aggregate quote (no re-pricing), so its two-way is the maker mid ± half
        // the panel sources were built from.
        let quote = tokio::time::timeout(STEP_DEADLINE, client.request_quote(quote_request(key)))
            .await
            .expect("request_quote returns in time")
            .expect("request_quote succeeds")
            .into_inner();
        assert_eq!(
            multi.quote_id, quote.quote_id,
            "the aggregate quote_id keys the same stored quote"
        );

        assert_eq!(
            multi.dealers.len(),
            4,
            "native maker + 3 synthetic demo dealers"
        );
        for d in &multi.dealers {
            let p = d.price.expect("two-way present");
            assert!(p.bid < p.offer, "{}: uncrossed two-way", d.lp_id);
            assert_eq!(
                d.resolved_strike, quote.resolved_strike,
                "{}: every dealer quotes the same resolved line",
                d.lp_id
            );
            assert!(
                d.valid_until_nanos > multi.epoch_nanos,
                "{}: forward last-look deadline",
                d.lp_id
            );
        }

        // Re-derive each synthetic dealer's two-way from the maker quote with the
        // documented offsets (spread widen 5%·k, mid shade ±0.25·half) — the same
        // IEEE-754 expression order the edge uses, so equality is exact.
        let q = quote.price.expect("quote two-way present");
        let mid = (q.bid + q.offer) / 2.0;
        let half = (q.offer - q.bid) / 2.0;
        for k in 1u32..=3 {
            let widen = 1.0 + 0.05 * f64::from(k);
            let shade = 0.25 * half;
            let skew = if k % 2 == 1 { shade } else { -shade };
            let (synth_mid, synth_half) = (mid + skew, half * widen);
            let lp = format!("SYNTH-LP-{k}");
            let row = multi
                .dealers
                .iter()
                .find(|d| d.lp_id == lp)
                .unwrap_or_else(|| panic!("panel carries {lp}"));
            let p = row.price.expect("two-way present");
            assert!(
                p.bid == synth_mid - synth_half && p.offer == synth_mid + synth_half,
                "{lp}: quoted ({}, {}) != derived ({}, {})",
                p.bid,
                p.offer,
                synth_mid - synth_half,
                synth_mid + synth_half
            );
            // A synthetic LP discloses its price and identity, not the maker's
            // greeks; the line is attributed to the LP's own auto-pricer seat.
            assert!(row.greeks.is_none(), "{lp}: an LP discloses no greeks");
            let quoted_by = row
                .attribution
                .as_ref()
                .and_then(|a| a.quoted_by.as_ref())
                .expect("synthetic line is attributed");
            assert_eq!(quoted_by.book, lp);
            assert!(matches!(
                quoted_by.owner.as_ref().and_then(|o| o.seat.as_ref()),
                Some(owner::Seat::AutoPricer(id)) if *id == lp
            ));
        }
        // The native maker row carries the edge-priced greeks.
        let native = multi
            .dealers
            .iter()
            .find(|d| d.lp_id == MAKER_LP_ID)
            .expect("native maker row present");
        assert!(native.greeks.is_some(), "maker row carries greeks");

        // Engine law, independently recomputed over the returned rows.
        assert_eq!(multi.best_bid_lp_id, law_winner(&multi.dealers, true));
        assert_eq!(multi.best_offer_lp_id, law_winner(&multi.dealers, false));
        // And the deterministic offsets make the expected dealers the touch:
        // SYNTH-LP-1 shades up (strongest bid), SYNTH-LP-2 shades down (cheapest
        // offer).
        assert_eq!(multi.best_bid_lp_id, "SYNTH-LP-1");
        assert_eq!(multi.best_offer_lp_id, "SYNTH-LP-2");

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// `AcceptQuote(quote_id, lp_id)` books exactly the pinned panel row — its price
/// (BUY lifts the row's offer, SELL hits the row's bid) and LP attribution — and
/// the retry contract is side- AND line-matched.
#[tokio::test]
async fn accept_with_lp_id_books_pinned_panel_row() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_panel_edge(3).await;
        let mut client = quote_client(addr).await;

        // BUY the best offer's line (SYNTH-LP-2).
        let key = "md-book-buy";
        let multi = client
            .request_multi_dealer_quote(quote_request(key))
            .await
            .expect("multi-dealer request succeeds")
            .into_inner();
        let row = multi
            .dealers
            .iter()
            .find(|d| d.lp_id == "SYNTH-LP-2")
            .expect("SYNTH-LP-2 row present")
            .clone();
        let accept = QuoteAccept {
            quote_id: multi.quote_id,
            idempotency_key: key.to_owned(),
            side: Side::Buy as i32,
            lp_id: "SYNTH-LP-2".to_owned(),
        };
        let exec = client
            .accept_quote(accept.clone())
            .await
            .expect("dealer-line accept books")
            .into_inner();
        assert_eq!(exec.quote_id, multi.quote_id);
        assert_eq!(exec.side, Side::Buy as i32);
        assert!(
            exec.traded_premium == row.price.expect("row two-way").offer,
            "BUY books the pinned row's offer exactly: {} != {}",
            exec.traded_premium,
            row.price.expect("row two-way").offer
        );
        // The execution carries the dealer line's attribution (the LP identity).
        let quoted_by = exec
            .attribution
            .as_ref()
            .and_then(|a| a.quoted_by.as_ref())
            .expect("booking is attributed");
        assert_eq!(quoted_by.book, "SYNTH-LP-2");

        // Idempotent retry (same key, side, line) returns the same booking.
        let again = client
            .accept_quote(accept)
            .await
            .expect("retry returns the booking")
            .into_inner();
        assert_eq!(again.execution_id, exec.execution_id);
        assert!(again.traded_premium == exec.traded_premium);

        // A retry naming a DIFFERENT line is a different intent, not a retry.
        let flipped = client
            .accept_quote(QuoteAccept {
                quote_id: multi.quote_id,
                idempotency_key: key.to_owned(),
                side: Side::Buy as i32,
                lp_id: "SYNTH-LP-1".to_owned(),
            })
            .await
            .expect_err("a line-flipped retry is refused");
        assert_eq!(flipped.code(), tonic::Code::FailedPrecondition);

        // SELL hits the chosen row's bid (fresh quote — one booking per quote).
        let key2 = "md-book-sell";
        let multi2 = client
            .request_multi_dealer_quote(quote_request(key2))
            .await
            .expect("second multi-dealer request succeeds")
            .into_inner();
        let row1 = multi2
            .dealers
            .iter()
            .find(|d| d.lp_id == "SYNTH-LP-1")
            .expect("SYNTH-LP-1 row present")
            .clone();
        let sell = client
            .accept_quote(QuoteAccept {
                quote_id: multi2.quote_id,
                idempotency_key: key2.to_owned(),
                side: Side::Sell as i32,
                lp_id: "SYNTH-LP-1".to_owned(),
            })
            .await
            .expect("SELL books the dealer line")
            .into_inner();
        assert_eq!(sell.side, Side::Sell as i32);
        assert!(
            sell.traded_premium == row1.price.expect("row two-way").bid,
            "SELL hits the pinned row's bid exactly"
        );

        // A line that was never on this quote's panel is not bookable.
        let key3 = "md-book-unknown";
        let multi3 = client
            .request_multi_dealer_quote(quote_request(key3))
            .await
            .expect("third multi-dealer request succeeds")
            .into_inner();
        let unknown = client
            .accept_quote(QuoteAccept {
                quote_id: multi3.quote_id,
                idempotency_key: key3.to_owned(),
                side: Side::Buy as i32,
                lp_id: "LP-NEVER-ON-PANEL".to_owned(),
            })
            .await
            .expect_err("an unknown dealer line is refused");
        assert_eq!(unknown.code(), tonic::Code::FailedPrecondition);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// On a panel-enabled edge an empty `lp_id` (and the native maker's own id) stays
/// the single-dealer path: the booking trades the stored quote's own two-way with
/// the maker attribution — byte-identical to a single-dealer accept.
#[tokio::test]
async fn empty_lp_id_keeps_single_dealer_path_byte_identical() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_panel_edge(3).await;
        let mut client = quote_client(addr).await;

        // Issue the single-dealer quote FIRST, then run the panel over the same
        // idempotency key: the aggregate reuses the stored quote verbatim.
        let key = "md-native-1";
        let quote = client
            .request_quote(quote_request(key))
            .await
            .expect("request_quote succeeds")
            .into_inner();
        let multi = client
            .request_multi_dealer_quote(quote_request(key))
            .await
            .expect("multi-dealer request succeeds")
            .into_inner();
        assert_eq!(multi.quote_id, quote.quote_id);

        // Empty lp_id books the stored quote's own offer, exactly.
        let exec = client
            .accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: key.to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
            })
            .await
            .expect("single-dealer accept books")
            .into_inner();
        assert!(
            exec.traded_premium == quote.price.expect("quote two-way").offer,
            "empty lp_id books the single-dealer offer exactly"
        );
        // The maker auto-pricer quoted the booked line.
        let quoted_by = exec
            .attribution
            .as_ref()
            .and_then(|a| a.quoted_by.as_ref())
            .expect("booking is attributed");
        assert!(matches!(
            quoted_by.owner.as_ref().and_then(|o| o.seat.as_ref()),
            Some(owner::Seat::AutoPricer(id)) if id == MAKER_LP_ID
        ));

        // The native maker's explicit lp_id is the SAME line: a retry naming it
        // returns the same booking (empty ≡ native, never a different intent).
        let again = client
            .accept_quote(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: key.to_owned(),
                side: Side::Buy as i32,
                lp_id: MAKER_LP_ID.to_owned(),
            })
            .await
            .expect("native-line retry returns the booking")
            .into_inner();
        assert_eq!(again.execution_id, exec.execution_id);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An accept after the panel row's `valid_until_nanos` is refused as expired —
/// the engine's last-look law applied to the exact line being booked (and the
/// single-dealer line refuses identically).
#[tokio::test]
async fn expired_panel_row_accept_refused() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let clock = Clock::manual(1_000_000_000);
        let (edge, addr) = start_panel_edge_with(true, clock.clone(), 3).await;
        let mut client = quote_client(addr).await;

        let key = "md-lastlook-1";
        let multi = client
            .request_multi_dealer_quote(quote_request(key))
            .await
            .expect("multi-dealer request succeeds")
            .into_inner();
        assert_eq!(multi.dealers.len(), 4);

        // Drive past the 5s last-look window; every line is now stale.
        clock.advance(6_000_000_000);

        let stale_row = client
            .accept_quote(QuoteAccept {
                quote_id: multi.quote_id,
                idempotency_key: key.to_owned(),
                side: Side::Buy as i32,
                lp_id: "SYNTH-LP-2".to_owned(),
            })
            .await
            .expect_err("a lapsed dealer line is refused");
        assert_eq!(stale_row.code(), tonic::Code::DeadlineExceeded);

        let stale_native = client
            .accept_quote(QuoteAccept {
                quote_id: multi.quote_id,
                idempotency_key: key.to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
            })
            .await
            .expect_err("the lapsed single-dealer line is refused identically");
        assert_eq!(stale_native.code(), tonic::Code::DeadlineExceeded);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

// ---------------------------------------------------------------------------
// WS mirror: the same panel contract as type-tagged JSON frames
// ---------------------------------------------------------------------------

/// The wire conventions as the JSON a browser client sends (proto enum numbers).
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

/// The standard 1Y EURUSD vanilla call as WS JSON (mirrors `common::vanilla_call`).
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
        let frame = tokio::time::timeout(STEP_DEADLINE, ws.next())
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
    tokio::time::timeout(STEP_DEADLINE, ws.send(WsMessage::Text(v.to_string())))
        .await
        .expect("the send completes in time")
        .expect("the send succeeds");
}

/// A `request_multi_dealer_quote` frame over the WS mirror returns the ranked
/// panel (snake_case, field-for-field with the proto), and an `accept_quote`
/// frame carrying a row's `lp_id` books that dealer line at its pinned price.
#[tokio::test]
async fn ws_multi_dealer_frame_round_trips_and_books_lp_line() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc) = start_ready_panel_edge(3).await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP_DEADLINE, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        send_json(
            &mut ws,
            json!({
                "type": "request_multi_dealer_quote",
                "idempotency_key": "ws-md-1",
                "instrument": vanilla_call_json(1.12),
                "conventions": conventions_json()
            }),
        )
        .await;
        let frame = next_json(&mut ws).await;
        assert_eq!(frame["type"], "multi_dealer_quote", "frame: {frame}");
        assert_eq!(frame["idempotency_key"], "ws-md-1");
        let quote_id = frame["quote_id"].as_u64().expect("aggregate quote_id");
        let dealers = frame["dealers"].as_array().expect("dealers array");
        assert_eq!(dealers.len(), 4, "native maker + 3 synthetic demo dealers");
        // Field-for-field snake_case mirror of the proto DealerQuote.
        for d in dealers {
            assert!(d["lp_id"].is_string());
            assert!(d["price"]["bid"].is_f64() && d["price"]["offer"].is_f64());
            assert!(d["resolved_strike"].is_f64());
            assert!(d["valid_until_nanos"].is_i64() || d["valid_until_nanos"].is_u64());
        }
        assert_eq!(frame["best_bid_lp_id"], "SYNTH-LP-1");
        assert_eq!(frame["best_offer_lp_id"], "SYNTH-LP-2");
        assert!(frame["conventions"].is_object());
        // The maker row carries greeks; a synthetic LP row does not.
        let maker = dealers
            .iter()
            .find(|d| d["lp_id"] == MAKER_LP_ID)
            .expect("maker row present");
        assert!(maker["greeks"].is_object());
        let synth = dealers
            .iter()
            .find(|d| d["lp_id"] == "SYNTH-LP-2")
            .expect("SYNTH-LP-2 row present");
        assert!(synth["greeks"].is_null());
        let synth_offer = synth["price"]["offer"].as_f64().expect("offer present");

        // Lift SYNTH-LP-2's offer over the same socket: accept_quote + lp_id.
        send_json(
            &mut ws,
            json!({
                "type": "accept_quote",
                "quote_id": quote_id,
                "idempotency_key": "ws-md-1",
                "side": 0,
                "lp_id": "SYNTH-LP-2"
            }),
        )
        .await;
        let exec = next_json(&mut ws).await;
        assert_eq!(exec["type"], "execution", "frame: {exec}");
        assert_eq!(exec["quote_id"].as_u64(), Some(quote_id));
        let premium = exec["traded_premium"].as_f64().expect("premium present");
        assert!(
            premium == synth_offer,
            "WS booking trades the pinned row's offer exactly: {premium} != {synth_offer}"
        );
        // The booking is attributed to the dealer line's LP.
        assert_eq!(exec["attribution"]["quotedBy"]["book"], "SYNTH-LP-2");

        let _ = tokio::time::timeout(STEP_DEADLINE, ws.close(None)).await;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
