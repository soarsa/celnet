//! The in-memory **desk-inbox** and **received-deals** stores the dealer-quoting
//! [`RfqDeskService`](super) reads and writes.
//!
//! Both mirror the FX [`PositionStore`](crate::services::risk::store::PositionStore)
//! shape: [`RwLock`]-guarded, RAII-clean, a deterministic monotonic id counter, and
//! `snapshot` returning an owned `Vec`. They hold the canonical wire facts
//! ([`DeskRequest`] / [`Deal`]) directly — one current fact per id, a re-state of a
//! request superseding the prior one — so the gRPC reads, the WS mirror, and a
//! federating frontend all read one coherent inbox/blotter.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_proto::{Deal, DeskRequest};

/// The shared desk inbox: one current [`DeskRequest`] per `request_id` across its
/// lifecycle (PENDING → QUOTED → ACCEPTED/REJECTED/…).
#[derive(Debug)]
pub struct DeskRequestStore {
    inner: RwLock<Vec<DeskRequest>>,
    /// Monotonic id source for `request_id` (formatted `desk-req-{n}`), starting at
    /// `1` — deterministic, no wall-clock/randomness, so tests are reproducible.
    next_id: AtomicU64,
}

impl Default for DeskRequestStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DeskRequestStore {
    /// An empty inbox with the id counter primed at `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocate the next stable `request_id` (e.g. `desk-req-7`).
    #[must_use]
    pub fn next_request_id(&self) -> String {
        format!("desk-req-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Insert a brand-new request (its id already assigned via
    /// [`Self::next_request_id`]).
    pub fn insert(&self, request: DeskRequest) {
        let mut g = self
            .inner
            .write()
            .expect("desk request store lock poisoned");
        g.push(request);
    }

    /// The current request for `request_id`, if any.
    #[must_use]
    pub fn get(&self, request_id: &str) -> Option<DeskRequest> {
        self.inner
            .read()
            .expect("desk request store lock poisoned")
            .iter()
            .find(|r| r.request_id == request_id)
            .cloned()
    }

    /// Replace the current fact for `request.request_id` (a lifecycle transition).
    /// Returns the stored request, or `None` if no such id exists (a stale/unknown
    /// `request_id` is never silently inserted as a new row).
    #[must_use]
    pub fn replace(&self, request: DeskRequest) -> Option<DeskRequest> {
        let mut g = self
            .inner
            .write()
            .expect("desk request store lock poisoned");
        let slot = g.iter_mut().find(|r| r.request_id == request.request_id)?;
        *slot = request.clone();
        Some(request)
    }

    /// A deterministic snapshot of the whole inbox, **newest first** (descending
    /// `received_at_nanos`, ties broken by descending `request_id` insertion via the
    /// id ordinal) — the order `ListDeskRequests` reports.
    #[must_use]
    pub fn snapshot(&self) -> Vec<DeskRequest> {
        let g = self.inner.read().expect("desk request store lock poisoned");
        let mut out = g.clone();
        out.sort_by(|a, b| {
            b.received_at_nanos
                .cmp(&a.received_at_nanos)
                .then_with(|| b.request_id.cmp(&a.request_id))
        });
        out
    }

    /// The number of requests in the inbox.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .expect("desk request store lock poisoned")
            .len()
    }

    /// Whether the inbox is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The shared received-deals blotter: one [`Deal`] per accepted quote.
#[derive(Debug)]
pub struct DealStore {
    inner: RwLock<Vec<Deal>>,
    /// Monotonic id source for `deal_id` (formatted `deal-{n}`), starting at `1`.
    next_id: AtomicU64,
}

impl Default for DealStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DealStore {
    /// An empty blotter with the id counter primed at `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocate the next stable `deal_id` (e.g. `deal-3`).
    #[must_use]
    pub fn next_deal_id(&self) -> String {
        format!("deal-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Record a booked deal (its id already assigned via [`Self::next_deal_id`]).
    pub fn insert(&self, deal: Deal) {
        let mut g = self.inner.write().expect("deal store lock poisoned");
        g.push(deal);
    }

    /// A deterministic snapshot of the whole blotter, **newest first** (descending
    /// `executed_at_nanos`, ties broken by descending `deal_id`).
    #[must_use]
    pub fn snapshot(&self) -> Vec<Deal> {
        let g = self.inner.read().expect("deal store lock poisoned");
        let mut out = g.clone();
        out.sort_by(|a, b| {
            b.executed_at_nanos
                .cmp(&a.executed_at_nanos)
                .then_with(|| b.deal_id.cmp(&a.deal_id))
        });
        out
    }

    /// The number of booked deals.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().expect("deal store lock poisoned").len()
    }

    /// Whether the blotter is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{DeskRequestKind, DeskRequestState, Side};

    fn request(id: &str, received_at_nanos: i64) -> DeskRequest {
        DeskRequest {
            request_id: id.to_owned(),
            kind: DeskRequestKind::Rfq as i32,
            counterparty: "cp-a".to_owned(),
            desk: "g10".to_owned(),
            instrument: None,
            curve_set: None,
            side: Side::Buy as i32,
            notional: 10_000_000.0,
            received_at_nanos,
            expires_at_nanos: received_at_nanos + 1_000,
            state: DeskRequestState::Pending as i32,
            quote: None,
            correlation_id: None,
        }
    }

    /// Ids are monotonic and well-formatted; insert + get + snapshot newest-first.
    #[test]
    fn request_ids_are_monotonic_and_snapshot_is_newest_first() {
        let store = DeskRequestStore::new();
        let id1 = store.next_request_id();
        let id2 = store.next_request_id();
        assert_eq!(id1, "desk-req-1");
        assert_eq!(id2, "desk-req-2");
        store.insert(request(&id1, 100));
        store.insert(request(&id2, 200));
        assert_eq!(store.len(), 2);
        let snap = store.snapshot();
        // Newest (200) first.
        assert_eq!(snap[0].request_id, id2);
        assert_eq!(snap[1].request_id, id1);
        assert_eq!(store.get(&id1).unwrap().request_id, id1);
    }

    /// `replace` supersedes the current fact; an unknown id is rejected (None).
    #[test]
    fn replace_supersedes_known_rejects_unknown() {
        let store = DeskRequestStore::new();
        let id = store.next_request_id();
        store.insert(request(&id, 100));
        let mut updated = request(&id, 100);
        updated.state = DeskRequestState::Quoted as i32;
        assert!(store.replace(updated).is_some());
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.get(&id).unwrap().state,
            DeskRequestState::Quoted as i32
        );
        // Unknown id ⇒ no insert, None.
        assert!(store.replace(request("desk-req-999", 1)).is_none());
        assert_eq!(store.len(), 1);
    }

    /// Deal ids are monotonic; the blotter snapshots newest-first.
    #[test]
    fn deal_blotter_is_newest_first() {
        let store = DealStore::new();
        assert_eq!(store.next_deal_id(), "deal-1");
        assert_eq!(store.next_deal_id(), "deal-2");
        let mut a = Deal {
            deal_id: "deal-a".to_owned(),
            request_id: "desk-req-1".to_owned(),
            kind: DeskRequestKind::Rfq as i32,
            counterparty: "cp".to_owned(),
            desk: "g10".to_owned(),
            instrument: None,
            curve_set: None,
            side: Side::Sell as i32,
            notional: 1.0,
            price: 0.04,
            executed_at_nanos: 10,
            trader: "t".to_owned(),
            position_id: Some(1),
            correlation_id: None,
            pricing_provenance: None,
            risk_book_id: None,
        };
        store.insert(a.clone());
        a.deal_id = "deal-b".to_owned();
        a.executed_at_nanos = 20;
        store.insert(a);
        let snap = store.snapshot();
        assert_eq!(snap[0].deal_id, "deal-b");
        assert_eq!(snap[1].deal_id, "deal-a");
    }
}
