//! Trader-workflow integration tests for the dealer-side quoting desk SDK surface
//! (`RfqDeskService`): a counterparty submits an inbound RFQ, the desk quotes it, the
//! counterparty lifts the quote (booking a deal + a rates position), and the inbox /
//! deals blotter read it back — the full maker-side lifecycle driven end-to-end
//! through the typed [`celnet_client`] SDK (the api-first parity rule: the SDK user
//! drives the SAME desk contract the GUI desk view does, never raw proto).
//!
//! Each test starts a real in-process `celnet-server` edge under the production
//! `Enforce` posture and authenticates as the seed admin: the desk `respond` /
//! `accept` actions are capability-gated (`RfqRespond` / `Execute·FixedIncome`), which
//! resolve ONLY from an authenticated session. Never a mock. Every body is hard
//! wall-clock bounded and every network await is bounded, so a regression fails fast.

mod common;

use std::time::Duration;

use celnet_client::{
    CivilDate, DealFilter, DeskQuote, DeskRequestKind, DeskRequestState, DeskRfq, Ois, Side,
    UsdSofrCurve,
};

use common::{STEP_DEADLINE, TEST_DEADLINE, start_edge_and_authed_client};

/// A calibrated USD-SOFR curve the desk prices its inbound requests against.
fn curve() -> UsdSofrCurve {
    UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
        .pillar(1, 0.0432)
        .pillar(2, 0.0418)
        .pillar(5, 0.0405)
}

/// The full desk lifecycle: submit → quote → accept, driven through the typed
/// [`celnet_client::DeskClient`], books a deal + a rates position; the terminal
/// states and the desk's opposite side are surfaced verbatim.
#[tokio::test]
async fn submit_quote_accept_books_a_deal() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_authed_client().await;
        let desk = client.desk();

        // The counterparty submits a firm RFQ (pay-fixed, i.e. Buy from their side).
        let submitted = step(
            desk.submit(
                DeskRfq::rfq(
                    "ACME",
                    "g10",
                    Ois::pay_fixed(5, 0.0405).notional(25_000_000.0),
                    curve(),
                )
                .ttl(Duration::from_secs(5))
                .correlation_id("cid-42"),
            ),
        )
        .await;
        assert!(
            !submitted.request_id.is_empty(),
            "server assigns a request id"
        );
        assert_eq!(submitted.kind, DeskRequestKind::Rfq);
        assert_eq!(submitted.state, DeskRequestState::Pending);
        assert_eq!(submitted.desk, "g10");
        assert_eq!(submitted.side, Side::Buy, "pay-fixed ⇒ counterparty Buy");
        assert_eq!(submitted.correlation_id.as_deref(), Some("cid-42"));
        assert!(submitted.quote.is_none(), "PENDING carries no quote");

        // The desk quotes the request at a firm level.
        let quoted = step(desk.respond_quote(
            &submitted.request_id,
            DeskQuote::new(0.0412, 25_000_000.0, Duration::from_secs(2), "alice"),
        ))
        .await;
        assert_eq!(quoted.state, DeskRequestState::Quoted);
        let quote = quoted.quote.expect("QUOTED carries the desk's quote");
        assert_eq!(quote.price, 0.0412);
        assert_eq!(quote.trader, "alice");

        // The counterparty lifts the quote → a deal + rates position book.
        let accepted = step(desk.accept(&submitted.request_id)).await;
        assert_eq!(accepted.request.state, DeskRequestState::Accepted);
        assert_eq!(accepted.deal.request_id, submitted.request_id);
        assert_eq!(accepted.deal.price, 0.0412);
        assert_eq!(accepted.deal.notional, 25_000_000.0);
        assert_eq!(
            accepted.deal.side,
            Side::Sell,
            "the desk deals the opposite side of the counterparty's Buy"
        );
        assert!(
            accepted.deal.position_id.is_some(),
            "the accepted deal booked a rates position"
        );

        // The inbox shows the terminal request; the blotter shows the booked deal.
        let inbox = step(desk.list_requests(&Default::default())).await;
        assert!(
            inbox
                .iter()
                .any(|r| r.request_id == submitted.request_id
                    && r.state == DeskRequestState::Accepted),
            "the accepted request is in the desk inbox"
        );
        let deals = step(desk.list_deals(&DealFilter::new().desk("g10"))).await;
        assert!(
            deals.iter().any(|d| d.deal_id == accepted.deal.deal_id),
            "the booked deal is on the received-deals blotter"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// A declined request advances to `REJECTED` and can no longer be accepted (the
/// state precondition), and a decline surfaces the typed terminal state.
#[tokio::test]
async fn declined_request_cannot_be_accepted() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_authed_client().await;
        let desk = client.desk();

        let submitted = step(desk.submit(DeskRfq::rfq(
            "ACME",
            "g10",
            Ois::receive_fixed(2, 0.0418).notional(10_000_000.0),
            curve(),
        )))
        .await;

        let declined = step(desk.decline(&submitted.request_id, "off-the-run")).await;
        assert_eq!(declined.state, DeskRequestState::Rejected);

        // Accepting a rejected (non-QUOTED) request is refused server-side.
        let err = desk
            .accept(&submitted.request_id)
            .await
            .expect_err("a rejected request cannot be accepted");
        // A typed server status (failed_precondition), not a panic or a silent ok.
        assert!(
            matches!(err, celnet_client::ClientError::Status(_)),
            "accept of a rejected request is a typed server status, got {err:?}"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// Bound a single desk call so a never-arriving reply fails fast.
async fn step<T>(fut: impl std::future::Future<Output = celnet_client::ClientResult<T>>) -> T {
    tokio::time::timeout(STEP_DEADLINE, fut)
        .await
        .expect("desk call resolves in time")
        .expect("desk call succeeds")
}
