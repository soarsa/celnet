//! The dedicated server→client **notification push channel**: a desk learns "an
//! RFQ/IOI requires pricing" (or "a quote was lifted / rejected / a request expired")
//! the instant it lands, without polling.
//!
//! # Design (CLAUDE.md §11: bounded-queue offload, never stall a publisher)
//!
//! [`NotificationBroker`] is a shared registry mapping each subscriber (one per
//! connected client / WS connection, or one gRPC `StreamNotifications` call) to its
//! **entitlement-resolved** desk filter and a **bounded**
//! [`tokio::sync::mpsc`] sender. [`NotificationBroker::publish`] fans a notification
//! out to every subscriber whose filter admits the notification's desk, using
//! [`try_send`](tokio::sync::mpsc::Sender::try_send) — it **never blocks** and
//! **never awaits**: if a slow subscriber's bounded queue is full the notification
//! is **dropped for that subscriber** (skip-on-full), so one lagging client can
//! never apply back-pressure to a publisher (the desk-response path) or to any other
//! subscriber. A closed channel (the client went away) is pruned lazily on the next
//! publish. The queue bound is [`NOTIFY_QUEUE_DEPTH`]; this is a quiet, low-rate
//! control channel held entirely off the pinned pricing core.

use std::collections::HashSet;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_proto::Notification;
use tokio::sync::mpsc;

/// The bounded per-subscriber queue depth. A control-plane channel: deep enough to
/// absorb a burst of inbound RFQs, shallow enough that a wedged client is shed fast
/// rather than buffering unboundedly. Full ⇒ the notification is skipped for that
/// subscriber (the publisher never blocks).
pub const NOTIFY_QUEUE_DEPTH: usize = 256;

/// The entitlement-resolved desk filter a subscriber receives notifications under —
/// already intersected with what the caller is allowed to see, so [`publish`] does
/// no further authorization (it is a pure desk-string match).
///
/// [`publish`]: NotificationBroker::publish
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeskFilter {
    /// Every desk (an admin / no-session caller who requested no narrowing).
    All,
    /// Only these specific desks (a desk-bound caller, or an explicit scope).
    Desks(HashSet<String>),
}

impl DeskFilter {
    /// Whether a notification targeting `desk` is delivered under this filter.
    #[must_use]
    pub fn allows(&self, desk: &str) -> bool {
        match self {
            DeskFilter::All => true,
            DeskFilter::Desks(set) => set.contains(desk),
        }
    }
}

/// One registered subscriber: a stable id, its desk filter, and its bounded sender.
#[derive(Debug)]
struct Subscriber {
    id: u64,
    filter: DeskFilter,
    tx: mpsc::Sender<Notification>,
}

/// A live subscription handle: the assigned id (used to deregister) and the bounded
/// receiver the connection drains and writes to its socket. Dropping the receiver
/// closes the channel; the broker prunes the entry lazily on the next publish, or
/// the connection layer deregisters it explicitly via
/// [`NotificationBroker::unsubscribe`].
#[derive(Debug)]
pub struct Subscription {
    /// The broker-assigned subscriber id (deregister key).
    pub id: u64,
    /// The bounded receiver to drain notifications from.
    pub rx: mpsc::Receiver<Notification>,
}

/// The shared notification fan-out registry. Created once at boot and shared (behind
/// an [`Arc`](std::sync::Arc)) into both the publishers (the
/// [`RfqDeskService`](super) edge) and the subscribers (the WS connection layer /
/// the gRPC `StreamNotifications` handler).
#[derive(Debug)]
pub struct NotificationBroker {
    subscribers: RwLock<Vec<Subscriber>>,
    next_id: AtomicU64,
}

impl Default for NotificationBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationBroker {
    /// An empty broker (no subscribers), id counter primed at `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Register a subscriber under an already entitlement-resolved `filter`,
    /// returning its [`Subscription`] (id + bounded receiver). The queue is bounded
    /// to [`NOTIFY_QUEUE_DEPTH`].
    #[must_use]
    pub fn subscribe(&self, filter: DeskFilter) -> Subscription {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(NOTIFY_QUEUE_DEPTH);
        let mut g = self
            .subscribers
            .write()
            .expect("notification broker lock poisoned");
        g.push(Subscriber { id, filter, tx });
        Subscription { id, rx }
    }

    /// Deregister a subscriber by id (idempotent — an already-removed id is a no-op).
    pub fn unsubscribe(&self, id: u64) {
        let mut g = self
            .subscribers
            .write()
            .expect("notification broker lock poisoned");
        g.retain(|s| s.id != id);
    }

    /// Fan a notification out to every subscriber whose desk filter admits
    /// `notification.desk`. Non-blocking: a full subscriber queue **skips** that
    /// subscriber (the notification is dropped for it), and a closed channel marks
    /// the subscriber for lazy pruning — a publisher is never stalled by a slow or
    /// gone client.
    pub fn publish(&self, notification: &Notification) {
        // Snapshot the matching senders under the read lock, then send outside it so
        // a `try_send` never holds the registry lock; collect any closed ids to prune.
        let mut closed: Vec<u64> = Vec::new();
        {
            let g = self
                .subscribers
                .read()
                .expect("notification broker lock poisoned");
            for sub in g.iter() {
                if !sub.filter.allows(&notification.desk) {
                    continue;
                }
                match sub.tx.try_send(notification.clone()) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        // Skip-on-full: never block the publisher (CLAUDE.md §11).
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => closed.push(sub.id),
                }
            }
        }
        if !closed.is_empty() {
            let mut g = self
                .subscribers
                .write()
                .expect("notification broker lock poisoned");
            g.retain(|s| !closed.contains(&s.id));
        }
    }

    /// The number of live subscribers (after the most recent prune). Test/observe aid.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .read()
            .expect("notification broker lock poisoned")
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{DeskRequestKind, NotificationKind};

    fn notification(desk: &str) -> Notification {
        Notification {
            notification_id: "notif-1".to_owned(),
            kind: NotificationKind::RfqReceived as i32,
            at_nanos: 1,
            request_id: Some("desk-req-1".to_owned()),
            desk: desk.to_owned(),
            counterparty: "cp".to_owned(),
            request_kind: DeskRequestKind::Rfq as i32,
            headline: "RFQ".to_owned(),
            detail: None,
            alert_worthy: false,
            reason: None,
        }
    }

    /// A subscriber receives a notification for a desk its filter admits.
    #[tokio::test]
    async fn publish_reaches_a_matching_subscriber() {
        let broker = NotificationBroker::new();
        let mut sub = broker.subscribe(DeskFilter::All);
        broker.publish(&notification("g10"));
        let got = sub
            .rx
            .try_recv()
            .expect("a matching subscriber receives it");
        assert_eq!(got.desk, "g10");
    }

    /// Desk-scope filtering: a subscriber scoped to `g10` does not receive an `em`
    /// notification, but does receive a `g10` one.
    #[tokio::test]
    async fn publish_filters_by_desk_scope() {
        let broker = NotificationBroker::new();
        let mut g10 = broker.subscribe(DeskFilter::Desks(["g10".to_owned()].into()));
        broker.publish(&notification("em"));
        assert!(
            g10.rx.try_recv().is_err(),
            "an out-of-scope desk must not be delivered"
        );
        broker.publish(&notification("g10"));
        assert_eq!(
            g10.rx.try_recv().expect("in-scope desk delivered").desk,
            "g10"
        );
    }

    /// Many-to-many routing: a subscriber scoped to the SET {A, B} receives
    /// notifications routed to A and to B, but NOT to a third desk C. An all-desks
    /// subscriber receives A/B/C; a deskless subscriber (empty set) receives none.
    #[tokio::test]
    async fn publish_routes_to_multi_desk_subscriber() {
        let broker = NotificationBroker::new();
        let mut ab = broker.subscribe(DeskFilter::Desks(["a".to_owned(), "b".to_owned()].into()));
        let mut all = broker.subscribe(DeskFilter::All);
        let mut none = broker.subscribe(DeskFilter::Desks(std::collections::HashSet::new()));

        for desk in ["a", "b", "c"] {
            broker.publish(&notification(desk));
        }

        // The {a,b} subscriber gets a and b, never c.
        assert_eq!(ab.rx.try_recv().expect("a delivered").desk, "a");
        assert_eq!(ab.rx.try_recv().expect("b delivered").desk, "b");
        assert!(
            ab.rx.try_recv().is_err(),
            "c must not reach an {{a,b}} subscriber"
        );

        // The all-desks subscriber gets every desk.
        for desk in ["a", "b", "c"] {
            assert_eq!(all.rx.try_recv().expect("all-desks delivered").desk, desk);
        }

        // The deskless subscriber gets nothing.
        assert!(
            none.rx.try_recv().is_err(),
            "a deskless subscriber receives none"
        );
    }

    /// Fan-out: two subscribers both matching a desk both receive the notification.
    #[tokio::test]
    async fn publish_fans_out_to_all_matching() {
        let broker = NotificationBroker::new();
        let mut a = broker.subscribe(DeskFilter::All);
        let mut b = broker.subscribe(DeskFilter::Desks(["g10".to_owned()].into()));
        broker.publish(&notification("g10"));
        assert!(a.rx.try_recv().is_ok());
        assert!(b.rx.try_recv().is_ok());
        assert_eq!(broker.subscriber_count(), 2);
    }

    /// A dropped receiver (gone client) is pruned lazily on the next publish, and the
    /// publisher is never blocked.
    #[tokio::test]
    async fn closed_subscriber_is_pruned_on_publish() {
        let broker = NotificationBroker::new();
        let sub = broker.subscribe(DeskFilter::All);
        assert_eq!(broker.subscriber_count(), 1);
        drop(sub); // client went away
        broker.publish(&notification("g10"));
        assert_eq!(broker.subscriber_count(), 0, "closed subscriber pruned");
    }

    /// Explicit unsubscribe deregisters; a later publish reaches no one.
    #[tokio::test]
    async fn unsubscribe_deregisters() {
        let broker = NotificationBroker::new();
        let sub = broker.subscribe(DeskFilter::All);
        broker.unsubscribe(sub.id);
        assert_eq!(broker.subscriber_count(), 0);
        // Idempotent.
        broker.unsubscribe(sub.id);
    }

    /// Skip-on-full: once the bounded queue is saturated, further publishes are
    /// dropped for that subscriber and the publisher returns immediately.
    #[tokio::test]
    async fn full_queue_skips_without_blocking() {
        let broker = NotificationBroker::new();
        let _sub = broker.subscribe(DeskFilter::All);
        // Saturate the bounded queue plus several extra publishes.
        for _ in 0..(NOTIFY_QUEUE_DEPTH + 16) {
            broker.publish(&notification("g10"));
        }
        // The publisher never blocked (the test would hang otherwise) and the
        // subscriber is still registered (full is not closed).
        assert_eq!(broker.subscriber_count(), 1);
    }
}
