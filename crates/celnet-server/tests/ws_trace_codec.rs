//! WS wire-contract tests for the end-to-end event-tracing verbs (`GetTrace` /
//! `ListTraces`). The descriptor-driven generated codec is validated against a
//! hand-written expected-JSON oracle — the same independent-oracle discipline the
//! differential harness uses, proving the `TraceEvent` / `TraceSummary` /
//! `GetTraceResponse` / `ListTracesResponse` adapters emit the right JSON shape
//! (enum-as-int `stage`, null-absent optionals, repeated nested rows) and the
//! request builders decode round-trip.

use celnet_proto::{
    GetTraceRequest, GetTraceResponse, ListTracesRequest, ListTracesResponse, TraceEvent,
    TraceStage, TraceSummary,
};
use celnet_server::ws::generated_codec as generated;
use serde_json::{Value, json};

/// A fully-populated event (every optional present) at the QUOTE_PUBLISHED stage.
fn quote_event() -> TraceEvent {
    TraceEvent {
        trace_id: 7,
        seq: 0,
        stage: TraceStage::QuotePublished as i32,
        timestamp_ns: 1_000,
        symbol: "EURUSD".into(),
        side: "buy".into(),
        price: Some(1.0850),
        notional: Some(5_000_000.0),
        quote_id: Some("Q-1".into()),
        deal_id: None,
        book_id: None,
        counterparty: Some("cp-a".into()),
        decision: None,
        hedge_id: None,
        detail: Some("bid=1.0849; offer=1.0851".into()),
        position_id: None,
    }
}

/// A booking-stage event: durable `position_id` present, display `deal_id` absent.
fn booked_event() -> TraceEvent {
    TraceEvent {
        trace_id: 7,
        seq: 1,
        stage: TraceStage::DealBooked as i32,
        timestamp_ns: 2_500,
        symbol: "EURUSD".into(),
        side: "".into(),
        price: Some(1.0851),
        notional: None,
        quote_id: None,
        deal_id: None,
        book_id: Some("BOOK-G10".into()),
        counterparty: Some("cp-a".into()),
        decision: None,
        hedge_id: None,
        detail: None,
        position_id: Some(42),
    }
}

#[test]
fn get_trace_response_encodes_to_expected_json() {
    let resp = GetTraceResponse {
        events: vec![quote_event(), booked_event()],
        correlation_id: Some(99),
    };
    let got = generated::encode_get_trace_response(&resp);
    let expected = json!({
        "events": [
            {
                "trace_id": 7,
                "seq": 0,
                // enum-as-int: TraceStage::QuotePublished == 2
                "stage": 2,
                "timestamp_ns": 1000,
                "symbol": "EURUSD",
                "side": "buy",
                "price": 1.0850,
                "notional": 5_000_000.0,
                "quote_id": "Q-1",
                // null-absent optionals (TraceEvent is on the null-absent list)
                "deal_id": Value::Null,
                "book_id": Value::Null,
                "counterparty": "cp-a",
                "decision": Value::Null,
                "hedge_id": Value::Null,
                "detail": "bid=1.0849; offer=1.0851",
                "position_id": Value::Null,
            },
            {
                "trace_id": 7,
                "seq": 1,
                // TraceStage::DealBooked == 7
                "stage": 7,
                "timestamp_ns": 2500,
                "symbol": "EURUSD",
                "side": "",
                "price": 1.0851,
                "notional": Value::Null,
                "quote_id": Value::Null,
                "deal_id": Value::Null,
                "book_id": "BOOK-G10",
                "counterparty": "cp-a",
                "decision": Value::Null,
                "hedge_id": Value::Null,
                "detail": Value::Null,
                "position_id": 42,
            }
        ],
        "correlation_id": 99,
    });
    assert_eq!(got, expected);
}

#[test]
fn get_trace_response_empty_renders_empty_array_and_null_corr() {
    let resp = GetTraceResponse {
        events: vec![],
        correlation_id: None,
    };
    let got = generated::encode_get_trace_response(&resp);
    assert_eq!(got.get("events"), Some(&Value::Array(vec![])));
    // Absent optional correlation_id ⇒ present-with-null (null-absent convention).
    assert_eq!(got.get("correlation_id"), Some(&Value::Null));
}

#[test]
fn list_traces_response_encodes_to_expected_json() {
    let resp = ListTracesResponse {
        traces: vec![TraceSummary {
            trace_id: 7,
            first_stage: TraceStage::PriceComputed as i32,
            last_stage: TraceStage::HedgeFired as i32,
            first_timestamp_ns: 100,
            last_timestamp_ns: 900,
            total_latency_ns: 800,
            event_count: 9,
            symbol: "EURUSD".into(),
            counterparty: Some("cp-a".into()),
            outcome: "hedged".into(),
        }],
        correlation_id: None,
    };
    let got = generated::encode_list_traces_response(&resp);
    let expected = json!({
        "traces": [{
            "trace_id": 7,
            // TraceStage::PriceComputed == 1, HedgeFired == 9
            "first_stage": 1,
            "last_stage": 9,
            "first_timestamp_ns": 100,
            "last_timestamp_ns": 900,
            "total_latency_ns": 800,
            "event_count": 9,
            "symbol": "EURUSD",
            "counterparty": "cp-a",
            "outcome": "hedged",
        }],
        // null-absent optional correlation_id
        "correlation_id": Value::Null,
    });
    assert_eq!(got, expected);
}

#[test]
fn trace_summary_absent_counterparty_renders_null() {
    let resp = ListTracesResponse {
        traces: vec![TraceSummary {
            trace_id: 1,
            first_stage: TraceStage::OrderReceived as i32,
            last_stage: TraceStage::AcceptanceDecided as i32,
            first_timestamp_ns: 10,
            last_timestamp_ns: 20,
            total_latency_ns: 10,
            event_count: 2,
            symbol: "GBPUSD".into(),
            counterparty: None,
            outcome: "rejected".into(),
        }],
        correlation_id: Some(5),
    };
    let got = generated::encode_list_traces_response(&resp);
    let row = &got.get("traces").and_then(Value::as_array).expect("traces")[0];
    assert_eq!(row.get("counterparty"), Some(&Value::Null));
    assert_eq!(row.get("outcome").and_then(Value::as_str), Some("rejected"));
    assert_eq!(got.get("correlation_id").and_then(Value::as_u64), Some(5));
}

#[test]
fn get_trace_request_decodes() {
    let full = json!({ "session_token": "tok", "trace_id": 42, "correlation_id": 9 });
    let req: GetTraceRequest =
        generated::decode_get_trace_request(full.as_object().expect("obj")).expect("decode");
    assert_eq!(req.session_token, "tok");
    assert_eq!(req.trace_id, 42);
    assert_eq!(req.correlation_id, Some(9));

    // Minimal: required session_token + trace_id, correlation_id absent.
    let minimal = json!({ "session_token": "t", "trace_id": 1 });
    let req: GetTraceRequest =
        generated::decode_get_trace_request(minimal.as_object().expect("obj")).expect("decode");
    assert_eq!(req.trace_id, 1);
    assert_eq!(req.correlation_id, None);

    // A missing required trace_id is a decode error, never a silent zero.
    let bad = json!({ "session_token": "t" });
    assert!(generated::decode_get_trace_request(bad.as_object().expect("obj")).is_err());
}

#[test]
fn list_traces_request_decodes_with_optional_filters() {
    let full = json!({
        "session_token": "tok",
        "limit": 50,
        "symbol": "EURUSD",
        "counterparty": "cp-a",
        "correlation_id": 3
    });
    let req: ListTracesRequest =
        generated::decode_list_traces_request(full.as_object().expect("obj")).expect("decode");
    assert_eq!(req.session_token, "tok");
    assert_eq!(req.limit, Some(50));
    assert_eq!(req.symbol.as_deref(), Some("EURUSD"));
    assert_eq!(req.counterparty.as_deref(), Some("cp-a"));
    assert_eq!(req.correlation_id, Some(3));

    // Minimal: only the required token; all filters absent.
    let minimal = json!({ "session_token": "t" });
    let req: ListTracesRequest =
        generated::decode_list_traces_request(minimal.as_object().expect("obj")).expect("decode");
    assert_eq!(req.limit, None);
    assert_eq!(req.symbol, None);
    assert_eq!(req.counterparty, None);
}
