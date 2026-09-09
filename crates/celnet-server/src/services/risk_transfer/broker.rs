//! The dedicated server→client **risk-transfer inbox push channel**: the desk /
//! trader on the receiving end of a `Pending` desk-to-desk / trader-to-trader
//! transfer learns of it the instant it lands (or is withdrawn / decided), without
//! polling — the four-eyes counterparty side of `docs/hedging/RISK-TRANSFER-REQUIREMENTS.md`
//! §9.2.
//!
//! # Design (CLAUDE.md §11: bounded-queue offload, never stall a publisher)
//!
//! [`RiskTransferBroker`] mirrors the desk [`NotificationBroker`](super::super::desk::notify)
//! arm-for-arm: a shared registry mapping each subscriber (one per connected client /
//! WS connection or gRPC `StreamRiskTransferInbox` call) to its **entitlement-resolved**
//! desk filter and a **bounded** [`tokio::sync::mpsc`] sender. Every transfer state
//! change re-publishes the current `Pending` roster; [`RiskTransferBroker::publish`]
//! filters that roster to each subscriber's desks (matching a transfer's **target**
//! desk — the side that must accept/reject it), wraps the subset in a
//! [`RiskTransferInbox`] frame, and [`try_send`](tokio::sync::mpsc::Sender::try_send)s
//! it — it **never blocks** and **never awaits**: a slow subscriber's full queue skips
//! that subscriber, a closed channel is pruned lazily. Held entirely off the pinned
//! pricing core.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_proto::{RiskTransfer, RiskTransferInbox};
use tokio::sync::mpsc;

use crate::services::desk::notify::DeskFilter;

/// The bounded per-subscriber queue depth — a quiet control-plane channel (mirrors
/// [`NOTIFY_QUEUE_DEPTH`](super::super::desk::notify::NOTIFY_QUEUE_DEPTH)). Full ⇒ the
/// inbox frame is skipped for that subscriber (the publisher never blocks).
pub const INBOX_QUEUE_DEPTH: usize = 128;

/// One registered subscriber: a stable id, its entitlement-resolved desk filter, and
/// its bounded sender.
#[derive(Debug)]
struct Subscriber {
    id: u64,
    filter: DeskFilter,
    tx: mpsc::Sender<RiskTransferInbox>,
}

/// A live subscription handle: the assigned id (deregister key) and the bounded
/// receiver the connection drains and writes to its socket.
#[derive(Debug)]
pub struct InboxSubscription {
    /// The broker-assigned subscriber id (deregister key).
    pub id: u64,
    /// The bounded receiver to drain inbox frames from.
    pub rx: mpsc::Receiver<RiskTransferInbox>,
}

/// The shared risk-transfer-inbox fan-out registry. Created once at boot and shared
/// (behind an [`Arc`](std::sync::Arc)) into both the publisher (the transfer service)
/// and the subscribers (the WS connection layer / the gRPC stream handler).
#[derive(Debug)]
pub struct RiskTransferBroker {
    subscribers: RwLock<Vec<Subscriber>>,
    next_id: AtomicU64,
}

impl Default for RiskTransferBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskTransferBroker {
    /// An empty broker (no subscribers), id counter primed at `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Register a subscriber under an already entitlement-resolved `filter`, returning
    /// its [`InboxSubscription`] (id + bounded receiver, depth [`INBOX_QUEUE_DEPTH`]).
    #[must_use]
    pub fn subscribe(&self, filter: DeskFilter) -> InboxSubscription {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(INBOX_QUEUE_DEPTH);
        let mut g = self
            .subscribers
            .write()
            .expect("risk-transfer broker lock poisoned");
        g.push(Subscriber { id, filter, tx });
        InboxSubscription { id, rx }
    }

    /// Deregister a subscriber by id (idempotent — an already-removed id is a no-op).
    pub fn unsubscribe(&self, id: u64) {
        let mut g = self
            .subscribers
            .write()
            .expect("risk-transfer broker lock poisoned");
        g.retain(|s| s.id != id);
    }

    /// Fan the current `Pending` transfer roster out to every subscriber, filtered to
    /// the transfers whose **target desk** the subscriber's filter admits (the desk
    /// that must accept/reject them). Non-blocking: a full subscriber queue **skips**
    /// that subscriber, a closed channel marks the subscriber for lazy pruning — a
    /// publisher is never stalled by a slow or gone client.
    pub fn publish(&self, pending: &[RiskTransfer], at_nanos: i64) {
        let mut closed: Vec<u64> = Vec::new();
        {
            let g = self
                .subscribers
                .read()
                .expect("risk-transfer broker lock poisoned");
            for sub in g.iter() {
                let mine: Vec<RiskTransfer> = pending
                    .iter()
                    .filter(|t| sub.filter.allows(target_desk(t)))
                    .cloned()
                    .collect();
                let frame = RiskTransferInbox {
                    pending: mine,
                    at_nanos,
                };
                match sub.tx.try_send(frame) {
                    Ok(()) => {}
                    // Skip-on-full: never block the publisher (CLAUDE.md §11).
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => closed.push(sub.id),
                }
            }
        }
        if !closed.is_empty() {
            let mut g = self
                .subscribers
                .write()
                .expect("risk-transfer broker lock poisoned");
            g.retain(|s| !closed.contains(&s.id));
        }
    }

    /// The number of live subscribers (after the most recent prune). Test/observe aid.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .read()
            .expect("risk-transfer broker lock poisoned")
            .len()
    }
}

/// The desk that must action a transfer — its **target** leg's desk (the four-eyes
/// counterparty side the inbox is scoped to).
fn target_desk(t: &RiskTransfer) -> &str {
    t.target.as_ref().map_or("", |l| l.desk_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{TransferKind, TransferLeg, TransferPriceBasis, TransferState};

    fn pending_transfer(id: &str, target_desk: &str) -> RiskTransfer {
        RiskTransfer {
            id: id.to_owned(),
            kind: TransferKind::DeskToDesk as i32,
            source: Some(TransferLeg {
                risk_book_id: "src".to_owned(),
                desk_id: "fx-desk".to_owned(),
                trader: "alice".to_owned(),
                position_ids: vec![1],
            }),
            target: Some(TransferLeg {
                risk_book_id: "tgt".to_owned(),
                desk_id: target_desk.to_owned(),
                trader: String::new(),
                position_ids: vec![],
            }),
            quantity_full: true,
            partial_notional: None,
            price_basis: TransferPriceBasis::Mid as i32,
            agreed_price: None,
            reason: String::new(),
            initiated_by: "alice".to_owned(),
            initiated_at: 1,
            state: TransferState::Pending as i32,
            approver: None,
            decided_at: None,
            transfer_price: None,
            provenance: None,
        }
    }

    /// A subscriber receives an inbox frame carrying the pending transfers whose
    /// target desk its filter admits.
    #[tokio::test]
    async fn publish_delivers_target_desk_scoped_inbox() {
        let broker = RiskTransferBroker::new();
        let mut sub = broker.subscribe(DeskFilter::Desks(["em-desk".to_owned()].into()));
        broker.publish(
            &[
                pending_transfer("t1", "em-desk"),
                pending_transfer("t2", "g10-desk"),
            ],
            42,
        );
        let frame = sub
            .rx
            .try_recv()
            .expect("a matching subscriber receives it");
        assert_eq!(frame.at_nanos, 42);
        assert_eq!(
            frame.pending.len(),
            1,
            "only the em-desk transfer is in scope"
        );
        assert_eq!(frame.pending[0].id, "t1");
    }

    /// An all-desks subscriber sees every pending transfer; fan-out reaches both.
    #[tokio::test]
    async fn publish_fans_out_and_all_sees_everything() {
        let broker = RiskTransferBroker::new();
        let mut all = broker.subscribe(DeskFilter::All);
        let mut scoped = broker.subscribe(DeskFilter::Desks(["em-desk".to_owned()].into()));
        broker.publish(
            &[
                pending_transfer("t1", "em-desk"),
                pending_transfer("t2", "g10-desk"),
            ],
            7,
        );
        assert_eq!(all.rx.try_recv().expect("all delivered").pending.len(), 2);
        assert_eq!(
            scoped
                .rx
                .try_recv()
                .expect("scoped delivered")
                .pending
                .len(),
            1
        );
    }

    /// A dropped receiver is pruned lazily on the next publish; the publisher never blocks.
    #[tokio::test]
    async fn closed_subscriber_pruned_on_publish() {
        let broker = RiskTransferBroker::new();
        let sub = broker.subscribe(DeskFilter::All);
        assert_eq!(broker.subscriber_count(), 1);
        drop(sub);
        broker.publish(&[pending_transfer("t1", "em-desk")], 1);
        assert_eq!(broker.subscriber_count(), 0);
    }
}
