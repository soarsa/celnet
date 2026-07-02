//! Trader-workflow integration test for the notification push-channel SDK surface
//! (`NotificationService.StreamNotifications`): a client opens ONE long-lived typed
//! subscription scoped to a desk, and a desk request submitted afterwards is fanned
//! to it as a typed [`celnet_client::Notification`] — the server→client push channel
//! driven end-to-end through the SDK (the api-first parity rule: the SDK user gets
//! the SAME notification stream the GUI desk toast/inbox does, never raw proto).
//!
//! The edge runs under the production `Enforce` posture; the subscription is
//! `ReadAny`-gated and the SDK asserts the audited grant-all principal, so it is
//! admitted. The subscription is registered server-side before the open resolves, so
//! a request submitted after the open is delivered. Never a mock. Every await is
//! bounded, so a regression fails fast rather than hanging on a never-arriving push.

mod common;

use std::time::Duration;

use celnet_client::{
    CivilDate, DeskRequestKind, DeskRfq, NotificationKind, NotificationScopeSpec, Ois, UsdSofrCurve,
};

use common::{STEP_DEADLINE, TEST_DEADLINE, start_edge_and_client};

fn curve() -> UsdSofrCurve {
    UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
        .pillar(1, 0.0432)
        .pillar(5, 0.0405)
}

/// A notification subscription scoped to a desk receives the `RFQ_RECEIVED` event
/// the instant a matching request is submitted — the push channel, end-to-end.
#[tokio::test]
async fn subscription_receives_a_pushed_rfq_event() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;

        // Open the subscription FIRST (registered server-side before the open
        // resolves), scoped to the desk we will submit to.
        let mut stream = tokio::time::timeout(
            STEP_DEADLINE,
            client.stream_notifications(&NotificationScopeSpec::desks(["g10"])),
        )
        .await
        .expect("stream opens in time")
        .expect("stream opens");

        // Now submit an RFQ to that desk — it fans a notification to the subscriber.
        let desk = client.desk();
        let submitted = tokio::time::timeout(
            STEP_DEADLINE,
            desk.submit(DeskRfq::rfq(
                "ACME",
                "g10",
                Ois::pay_fixed(5, 0.0405).notional(25_000_000.0),
                curve(),
            )),
        )
        .await
        .expect("submit resolves in time")
        .expect("submit succeeds");

        // The subscriber receives the typed RFQ_RECEIVED notification.
        let event = tokio::time::timeout(STEP_DEADLINE, stream.next())
            .await
            .expect("a notification arrives in time")
            .expect("the stream yields an event")
            .expect("the event decodes");

        assert_eq!(event.kind, NotificationKind::RfqReceived);
        assert_eq!(event.request_kind, DeskRequestKind::Rfq);
        assert_eq!(event.desk, "g10");
        assert_eq!(event.counterparty, "ACME");
        assert_eq!(
            event.request_id.as_deref(),
            Some(submitted.request_id.as_str()),
            "the notification click-through targets the submitted request"
        );
        assert!(!event.headline.is_empty(), "a human headline is rendered");

        // An out-of-scope desk is NOT delivered: a request to `em` never arrives on
        // the g10-scoped subscription within a bounded settle window.
        let _ = tokio::time::timeout(
            STEP_DEADLINE,
            desk.submit(DeskRfq::rfq(
                "OTHER",
                "em",
                Ois::pay_fixed(2, 0.0418).notional(5_000_000.0),
                curve(),
            )),
        )
        .await
        .expect("second submit resolves")
        .expect("second submit succeeds");
        let out_of_scope = tokio::time::timeout(Duration::from_millis(300), stream.next()).await;
        assert!(
            out_of_scope.is_err(),
            "an `em` request must not reach the g10-scoped subscription"
        );

        // Close the long-lived subscription before draining the edge: the server
        // holds the streaming response open until the client drops it, so the graceful
        // shutdown's connection drain would otherwise block on it.
        drop(stream);
        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}
