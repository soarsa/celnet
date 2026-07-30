//! The `notify` subcommand: subscribe to the dealer-desk notification push stream
//! (`NotificationService.StreamNotifications`) against a running edge through the
//! **same** typed [`celnet_client`] SDK the GUI `NotificationCenter` and the Excel
//! add-in consume (the api-first client-parity rule), and print each pushed event
//! with a human-readable label.
//!
//! The CLI adds no logic of its own: it opens the typed [`celnet_client::Client::stream_notifications`]
//! subscription, then renders each [`celnet_client::Notification`] verbatim. The one
//! CLI-owned concern is the terminal-facing label/severity for each
//! [`celnet_client::NotificationKind`] — an **exhaustive** mapping so every kind
//! (including the phase-6 `ORDER_RECEIVED` / `FILL` arms) is first-class and no kind
//! falls into a silent default bucket. The runtime/connect/deadline plumbing and
//! [`RiskError`] are shared with the [`crate::risk`] networked commands.

use std::fmt::Write as _;
use std::io::Write;
use std::time::Duration;

use celnet_client::{Notification, NotificationKind, NotificationScopeSpec};

use crate::risk::{RiskError, block_on, bounded, connect};

/// The hard ceiling on waiting for one pushed notification. A quiet desk (no events)
/// fails fast with a timeout rather than hanging the terminal indefinitely; a real
/// listener is driven by piping the command.
const NOTIFY_DEADLINE: Duration = Duration::from_secs(30);

/// A fully-parsed `notify` request.
#[derive(Debug, Clone)]
pub(crate) struct NotifyReq {
    pub(crate) endpoint: String,
    pub(crate) session_token: Option<String>,
    /// The desks to scope the subscription to (empty ⇒ every entitled desk).
    pub(crate) desks: Vec<String>,
    /// How many pushed events to print before unsubscribing.
    pub(crate) count: u32,
}

/// The human-readable label for a notification kind. **Exhaustive**: every kind has a
/// first-class label — there is no silent default that would drop `ORDER_RECEIVED` /
/// `FILL` into a generic bucket (four-client parity with the GUI `eventTypeForKind`).
pub(crate) fn kind_label(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::RfqReceived => "RFQ in",
        NotificationKind::IoiReceived => "IOI in",
        NotificationKind::RequestWithdrawn => "Withdrawn",
        NotificationKind::RequestExpired => "Expired",
        NotificationKind::QuoteAccepted => "Won",
        NotificationKind::QuoteRejected => "Lost",
        NotificationKind::OrderReceived => "Order in",
        NotificationKind::Fill => "Fill",
    }
}

/// The derived severity tag for a kind (§3 of `docs/NOTIFICATIONS-REQUIREMENTS.md`:
/// exceptions are `notable`; receipts / fills / wins are `info`). Severity is derived
/// on the client, never a wire field. Exhaustive for the same no-silent-default reason
/// as [`kind_label`].
pub(crate) fn kind_severity(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::RequestWithdrawn
        | NotificationKind::RequestExpired
        | NotificationKind::QuoteRejected => "notable",
        NotificationKind::RfqReceived
        | NotificationKind::IoiReceived
        | NotificationKind::QuoteAccepted
        | NotificationKind::OrderReceived
        | NotificationKind::Fill => "info",
    }
}

/// Render one pushed notification as a terminal line (label + severity + who/what).
fn format_notification_line(n: &Notification) -> String {
    let mut out = String::new();
    let req = n.request_id.as_deref().unwrap_or("—");
    let detail = n
        .detail
        .as_deref()
        .filter(|d| !d.is_empty())
        .map_or_else(String::new, |d| format!(" — {d}"));
    let _ = writeln!(
        out,
        "  [{:<9}] {:<7} desk={} cp={} req={}  {}{}",
        kind_label(n.kind),
        kind_severity(n.kind),
        n.desk,
        n.counterparty,
        req,
        n.headline,
        detail,
    );
    out
}

/// A short label for the subscription scope (a desk list or "all entitled desks").
fn scope_label(req: &NotifyReq) -> String {
    if req.desks.is_empty() {
        "all entitled desks".to_owned()
    } else {
        format!("desks=[{}]", req.desks.join(","))
    }
}

/// Run `notify`: open the notification subscription, print `count` pushed events with
/// their human-readable labels, then unsubscribe cleanly (dropping the stream).
pub(crate) fn run_notify<W: Write>(req: &NotifyReq, out: &mut W) -> Result<(), RiskError> {
    let report = block_on(async {
        let client = connect(&req.endpoint).await?;
        // Authenticate as the Login-issued user when a token is supplied; otherwise the
        // SDK asserts the audited grant-all principal the production `Enforce` edge
        // admits (parity with the `stream` / risk commands' default).
        let client = match &req.session_token {
            Some(token) => client.with_session_token(token.clone()),
            None => client,
        };
        let scope = if req.desks.is_empty() {
            NotificationScopeSpec::all()
        } else {
            NotificationScopeSpec::desks(req.desks.clone())
        };
        let mut stream = bounded(
            "open_notification_stream",
            client.stream_notifications(&scope),
        )
        .await?;

        let mut report = String::new();
        let _ = writeln!(
            report,
            "notify stream {} (printing {} event(s)):",
            scope_label(req),
            req.count
        );

        // Each event-await is bounded, so a silent desk fails fast rather than hanging
        // the terminal. Dropping `stream` at the end tears the subscription down.
        let mut seen = 0u32;
        while seen < req.count {
            let next = tokio::time::timeout(NOTIFY_DEADLINE, stream.next())
                .await
                .map_err(|_| RiskError::Timeout("the next notification"))?;
            let Some(item) = next else {
                let _ = writeln!(report, "  stream ended");
                break;
            };
            let notification = item.map_err(RiskError::Client)?;
            report.push_str(&format_notification_line(&notification));
            seen += 1;
        }
        if seen >= req.count {
            let _ = writeln!(report, "  ({seen} event(s) printed; unsubscribing)");
        }
        drop(stream);
        Ok::<_, RiskError>(report)
    })??;

    out.write_all(report.as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_client::DeskRequestKind;

    /// Build a sample notification of `kind` for the pure-render tests (no live edge).
    fn sample(kind: NotificationKind) -> Notification {
        Notification {
            notification_id: "notif-1".to_owned(),
            kind,
            at_nanos: 1,
            request_id: Some("req-1".to_owned()),
            desk: "g10".to_owned(),
            counterparty: "ACME".to_owned(),
            request_kind: DeskRequestKind::Rfq,
            headline: "headline".to_owned(),
            detail: Some("5y OIS".to_owned()),
        }
    }

    /// The full set of typed kinds the SDK decodes (`Unspecified` is a wire error, not
    /// a variant) — the exhaustiveness fixture for the parity assertions below.
    const ALL_KINDS: [NotificationKind; 8] = [
        NotificationKind::RfqReceived,
        NotificationKind::IoiReceived,
        NotificationKind::RequestWithdrawn,
        NotificationKind::RequestExpired,
        NotificationKind::QuoteAccepted,
        NotificationKind::QuoteRejected,
        NotificationKind::OrderReceived,
        NotificationKind::Fill,
    ];

    #[test]
    fn phase6_kinds_have_first_class_labels() {
        assert_eq!(kind_label(NotificationKind::OrderReceived), "Order in");
        assert_eq!(kind_label(NotificationKind::Fill), "Fill");
    }

    #[test]
    fn every_kind_has_a_nonempty_label_and_a_valid_severity() {
        // No silent default: each typed kind maps to a distinct non-empty label and a
        // recognised severity band.
        for kind in ALL_KINDS {
            assert!(!kind_label(kind).is_empty(), "empty label for {kind:?}");
            assert!(
                matches!(kind_severity(kind), "info" | "notable"),
                "unexpected severity for {kind:?}",
            );
        }
    }

    #[test]
    fn order_received_and_fill_are_informational() {
        assert_eq!(kind_severity(NotificationKind::OrderReceived), "info");
        assert_eq!(kind_severity(NotificationKind::Fill), "info");
    }

    #[test]
    fn rendered_line_carries_the_new_kind_labels() {
        let order = format_notification_line(&sample(NotificationKind::OrderReceived));
        assert!(order.contains("Order in"), "line was: {order}");
        assert!(order.contains("desk=g10"));
        assert!(order.contains("cp=ACME"));

        let fill = format_notification_line(&sample(NotificationKind::Fill));
        assert!(fill.contains("Fill"), "line was: {fill}");
        assert!(fill.contains("req=req-1"));
        assert!(fill.contains("5y OIS"));
    }

    #[test]
    fn scope_label_reflects_the_desks() {
        let all = NotifyReq {
            endpoint: "http://x".to_owned(),
            session_token: None,
            desks: vec![],
            count: 1,
        };
        assert_eq!(scope_label(&all), "all entitled desks");
        let scoped = NotifyReq {
            desks: vec!["g10".to_owned(), "em".to_owned()],
            ..all
        };
        assert_eq!(scope_label(&scoped), "desks=[g10,em]");
    }
}
