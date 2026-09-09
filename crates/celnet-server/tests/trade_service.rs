//! TradeService integration tests: testing the unified RFQ & execution RPCs over
//! `celnet_proto::trade_service_client::TradeServiceClient`.

mod common;

use celnet_proto::trade_service_client::TradeServiceClient;
use celnet_proto::{QuoteAccept, QuoteReject, QuoteRequest, Side};
use common::{
    STEP_DEADLINE, TEST_DEADLINE, start_ready_edge, vanilla_call, wire_conventions,
};

#[tokio::test]
async fn trade_service_rfq_lifecycle_quote_accept_and_reject() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, addr, _data_dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            TradeServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let strike = 1.12;
        // 1. Request a quote over TradeService
        let req = QuoteRequest {
            idempotency_key: "trade-rfq-001".to_owned(),
            instrument: Some(vanilla_call(strike)),
            conventions: Some(wire_conventions()),
            correlation_id: Some(12345),
            surface_version: None,
            attribution: None,
            session_token: None,
            principal: None,
        };

        let quote = tokio::time::timeout(STEP_DEADLINE, client.request_quote(req))
            .await
            .expect("request_quote returns in time")
            .expect("request_quote succeeds")
            .into_inner();

        assert!(quote.quote_id >= 1, "quote id is assigned");
        assert!(quote.price.is_some(), "price is present");

        // 2. Accept the quote over TradeService
        let accept = QuoteAccept {
            quote_id: quote.quote_id,
            idempotency_key: "trade-rfq-001".to_owned(),
            side: Side::Buy as i32,
            session_token: None,
            principal: None,
            lp_id: String::new(),
        };

        let execution = tokio::time::timeout(STEP_DEADLINE, client.accept_quote(accept))
            .await
            .expect("accept_quote returns in time")
            .expect("accept_quote succeeds")
            .into_inner();

        assert_eq!(execution.quote_id, quote.quote_id);
        assert!(execution.execution_id >= 1, "execution id assigned");
        assert_eq!(execution.side, Side::Buy as i32);

        // 3. Request a second quote to test reject
        let req2 = QuoteRequest {
            idempotency_key: "trade-rfq-002".to_owned(),
            instrument: Some(vanilla_call(strike)),
            conventions: Some(wire_conventions()),
            correlation_id: Some(12346),
            surface_version: None,
            attribution: None,
            session_token: None,
            principal: None,
        };

        let quote2 = tokio::time::timeout(STEP_DEADLINE, client.request_quote(req2))
            .await
            .expect("request_quote returns in time")
            .expect("request_quote succeeds")
            .into_inner();

        let reject = QuoteReject {
            quote_id: quote2.quote_id,
            reason: "test reject".to_owned(),
            session_token: None,
            principal: None,
        };

        let ack = tokio::time::timeout(STEP_DEADLINE, client.reject_quote(reject))
            .await
            .expect("reject_quote returns in time")
            .expect("reject_quote succeeds")
            .into_inner();

        assert_eq!(ack.quote_id, quote2.quote_id);
    })
    .await
    .expect("test completes within wall-clock ceiling");
}
