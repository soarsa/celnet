//! The dedicated server→client notification push channel — the typed SDK face of
//! the `NotificationService.StreamNotifications` contract.
//!
//! A connected client opens ONE long-lived notification subscription scoped to the
//! desks it watches ([`crate::Client::stream_notifications`]); the server fans out
//! entitlement-filtered [`Notification`] events to every matching subscriber the
//! instant a desk-lifecycle event occurs (an RFQ/IOI lands, a request is
//! withdrawn/expires, a quote is accepted/rejected) — no polling. The channel is
//! independent of the price (RFS) stream by operator choice: a quiet, low-rate
//! control channel that never shares back-pressure with pricing.
//!
//! The SDK collapses the wire streaming RPC into a typed [`NotificationStream`]: a
//! caller awaits [`NotificationStream::next`] (or drives it as a [`futures_util::Stream`])
//! and branches on the typed [`NotificationKind`] — the raw `tonic::Streaming`, the
//! `Option`-wrapped fields, and the wire enum tags never appear in caller code.

use std::pin::Pin;
use std::task::{Context, Poll};

use celnet_proto::convert::WireError;
use celnet_proto::notification_service_client::NotificationServiceClient;
use celnet_proto::{
    Notification as WireNotification, NotificationKind as WireNotificationKind, NotificationScope,
};
use futures_util::{Stream, StreamExt};
use tonic::transport::Channel;

use crate::desk::DeskRequestKind;
use crate::error::{ClientError, ClientResult};
use crate::risk::{Entitlements, principal_or_grant_all};

/// What a pushed notification concerns — the typed form of the wire
/// `NotificationKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    /// A new RFQ landed and needs pricing.
    RfqReceived,
    /// A new IOI landed for the desk to work.
    IoiReceived,
    /// A pending request was withdrawn by the counterparty.
    RequestWithdrawn,
    /// A pending request expired before the desk responded.
    RequestExpired,
    /// A desk quote was lifted by the counterparty → a deal booked.
    QuoteAccepted,
    /// A desk quote was rejected by the counterparty.
    QuoteRejected,
}

impl NotificationKind {
    fn from_wire(tag: i32) -> ClientResult<Self> {
        match WireNotificationKind::try_from(tag) {
            Ok(WireNotificationKind::RfqReceived) => Ok(NotificationKind::RfqReceived),
            Ok(WireNotificationKind::IoiReceived) => Ok(NotificationKind::IoiReceived),
            Ok(WireNotificationKind::RequestWithdrawn) => Ok(NotificationKind::RequestWithdrawn),
            Ok(WireNotificationKind::RequestExpired) => Ok(NotificationKind::RequestExpired),
            Ok(WireNotificationKind::QuoteAccepted) => Ok(NotificationKind::QuoteAccepted),
            Ok(WireNotificationKind::QuoteRejected) => Ok(NotificationKind::QuoteRejected),
            _ => Err(ClientError::Wire(WireError::UnknownEnum {
                kind: "NotificationKind",
                tag,
            })),
        }
    }
}

/// The desks a notification subscription covers. The default ([`NotificationScope::all`])
/// receives every desk the caller is entitled to; [`NotificationScope::desks`] narrows
/// to a named set (intersected server-side with the caller's entitlement).
#[derive(Debug, Clone, Default)]
pub struct NotificationScopeSpec {
    desks: Vec<String>,
}

impl NotificationScopeSpec {
    /// Every desk the caller is entitled to (no narrowing).
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Only these specific desks (intersected with the caller's entitlement).
    #[must_use]
    pub fn desks(desks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            desks: desks.into_iter().map(Into::into).collect(),
        }
    }

    fn to_wire(&self) -> Option<NotificationScope> {
        if self.desks.is_empty() {
            None
        } else {
            Some(NotificationScope {
                desks: self.desks.clone(),
            })
        }
    }
}

/// One pushed notification event the desk surfaces as a toast / inbox item — the
/// typed form of the wire `Notification`.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// The server-assigned notification identity (a de-dup key on the client).
    pub notification_id: String,
    /// What happened.
    pub kind: NotificationKind,
    /// When the event occurred (epoch nanos, UTC).
    pub at_nanos: i64,
    /// The request/deal this concerns, for click-through.
    pub request_id: Option<String>,
    /// The desk the event is scoped to.
    pub desk: String,
    /// The counterparty involved (display/attribution).
    pub counterparty: String,
    /// Whether the underlying request was an RFQ or an IOI.
    pub request_kind: DeskRequestKind,
    /// A one-line human headline (rendered in the toast / inbox).
    pub headline: String,
    /// An optional longer detail line.
    pub detail: Option<String>,
}

impl Notification {
    pub(crate) fn from_wire(w: &WireNotification) -> ClientResult<Self> {
        Ok(Self {
            notification_id: w.notification_id.clone(),
            kind: NotificationKind::from_wire(w.kind)?,
            at_nanos: w.at_nanos,
            request_id: w.request_id.clone(),
            desk: w.desk.clone(),
            counterparty: w.counterparty.clone(),
            request_kind: DeskRequestKind::from_wire_tag(w.request_kind)?,
            headline: w.headline.clone(),
            detail: w.detail.clone(),
        })
    }
}

/// A live notification subscription: a typed async stream of [`Notification`]s over
/// the one long-lived `StreamNotifications` call. Await [`NotificationStream::next`]
/// (or drive it as a [`Stream`]); dropping it closes the subscription and the server
/// deregisters it.
#[derive(Debug)]
pub struct NotificationStream {
    inner: tonic::Streaming<WireNotification>,
}

impl NotificationStream {
    pub(crate) fn new(inner: tonic::Streaming<WireNotification>) -> Self {
        Self { inner }
    }

    /// Await the next pushed notification, or `None` once the server closed the
    /// subscription (edge shutdown / drain, or the caller went away).
    ///
    /// # Errors
    ///
    /// A [`ClientError`] item if a pushed message could not be decoded or the server
    /// returned a status mid-stream.
    pub async fn next(&mut self) -> Option<ClientResult<Notification>> {
        match self.inner.next().await {
            Some(Ok(n)) => Some(Notification::from_wire(&n)),
            Some(Err(status)) => Some(Err(ClientError::from(status))),
            None => None,
        }
    }
}

impl Stream for NotificationStream {
    type Item = ClientResult<Notification>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok(n))) => Poll::Ready(Some(Notification::from_wire(&n))),
            Poll::Ready(Some(Err(status))) => Poll::Ready(Some(Err(ClientError::from(status)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Open the `StreamNotifications` server-stream over `channel` for `scope`, threading
/// the client's bearer `session_token` (else the audited grant-all principal). The
/// subscription is `ReadAny`-gated and registered server-side before the response
/// resolves, so an event published after this returns is delivered.
pub(crate) async fn open_stream(
    channel: Channel,
    scope: &NotificationScopeSpec,
    session_token: Option<String>,
    principal: Option<&Entitlements>,
) -> ClientResult<NotificationStream> {
    let mut svc = NotificationServiceClient::new(channel);
    let request = celnet_proto::StreamNotificationsRequest {
        session_token,
        scope: scope.to_wire(),
        principal: Some(principal_or_grant_all(principal)),
        correlation_id: None,
    };
    let inbound = svc.stream_notifications(request).await?.into_inner();
    Ok(NotificationStream::new(inbound))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_all_carries_no_wire_scope() {
        assert!(NotificationScopeSpec::all().to_wire().is_none());
        let scoped = NotificationScopeSpec::desks(["g10", "em"])
            .to_wire()
            .expect("scoped");
        assert_eq!(scoped.desks, vec!["g10".to_owned(), "em".to_owned()]);
    }

    #[test]
    fn notification_decodes_the_typed_kind() {
        let wire = WireNotification {
            notification_id: "notif-1".to_owned(),
            kind: WireNotificationKind::RfqReceived as i32,
            at_nanos: 1,
            request_id: Some("req-1".to_owned()),
            desk: "g10".to_owned(),
            counterparty: "ACME".to_owned(),
            request_kind: celnet_proto::DeskRequestKind::Rfq as i32,
            headline: "New RFQ".to_owned(),
            detail: Some("5y OIS".to_owned()),
        };
        let n = Notification::from_wire(&wire).expect("decodes");
        assert_eq!(n.kind, NotificationKind::RfqReceived);
        assert_eq!(n.request_kind, DeskRequestKind::Rfq);
        assert_eq!(n.desk, "g10");
        assert_eq!(n.detail.as_deref(), Some("5y OIS"));
    }

    #[test]
    fn unknown_kind_tag_is_a_typed_error() {
        assert!(NotificationKind::from_wire(999).is_err());
        assert!(NotificationKind::from_wire(WireNotificationKind::Unspecified as i32).is_err());
    }
}
