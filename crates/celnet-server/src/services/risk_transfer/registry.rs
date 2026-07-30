//! The in-memory **risk-transfer registry**: the append-and-update store of every
//! [`RiskTransfer`] record with a monotonic id mint and the immutable-audit query
//! surface (`docs/RISK-TRANSFER-REQUIREMENTS.md` §5, §7). Mirrors the desk
//! `DeskRequestStore` shape (an `RwLock<Vec<_>>` + an `AtomicU64` id source).
//!
//! The registry holds the pure-domain [`celnet_risk_transfer::RiskTransfer`] value
//! (validated + leg-computed by that crate); the service layer converts to/from the
//! wire. State transitions replace a record in place (one current record per id); the
//! provenance stamped on a `Booked` record is never mutated thereafter — the
//! append-only audit invariant (§7). A minted id is a stable, human-legible slug used
//! as the audit key.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_risk_transfer::{RiskTransfer, TransferState};

/// The shared risk-transfer registry. Cheap to share behind an [`Arc`](std::sync::Arc);
/// every mutation takes the write lock briefly (off the pinned pricing core).
#[derive(Debug)]
pub struct RiskTransferRegistry {
    records: RwLock<Vec<RiskTransfer>>,
    next_id: AtomicU64,
}

impl Default for RiskTransferRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskTransferRegistry {
    /// An empty registry, id counter primed at `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Mint the next stable transfer id — a human-legible audit-key slug.
    #[must_use]
    pub fn mint_id(&self) -> String {
        format!(
            "risk-transfer-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Append a new transfer record (the caller has already minted its id).
    pub fn insert(&self, transfer: RiskTransfer) {
        self.records
            .write()
            .expect("risk-transfer registry lock poisoned")
            .push(transfer);
    }

    /// Fetch a clone of the record with `id`, if present.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<RiskTransfer> {
        self.records
            .read()
            .expect("risk-transfer registry lock poisoned")
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    /// Replace the record with `updated.id` in place (its lifecycle-state transition).
    /// Returns whether a record was found and replaced.
    pub fn replace(&self, updated: RiskTransfer) -> bool {
        let mut g = self
            .records
            .write()
            .expect("risk-transfer registry lock poisoned");
        if let Some(slot) = g.iter_mut().find(|t| t.id == updated.id) {
            *slot = updated;
            true
        } else {
            false
        }
    }

    /// Every record, newest first (descending `initiated_at`, ties broken by id).
    #[must_use]
    pub fn snapshot(&self) -> Vec<RiskTransfer> {
        let mut all: Vec<RiskTransfer> = self
            .records
            .read()
            .expect("risk-transfer registry lock poisoned")
            .clone();
        all.sort_by(|a, b| {
            b.initiated_at
                .cmp(&a.initiated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        all
    }

    /// The `Pending` records (awaiting a counterparty accept/reject), newest first.
    #[must_use]
    pub fn pending(&self) -> Vec<RiskTransfer> {
        self.snapshot()
            .into_iter()
            .filter(|t| t.state == TransferState::Pending)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_transfer::{TransferKind, TransferLeg, TransferPrice, TransferQuantity};

    fn transfer(id: &str, at: i64, state: TransferState) -> RiskTransfer {
        RiskTransfer {
            id: id.to_owned(),
            kind: TransferKind::DeskToDesk,
            source: TransferLeg {
                risk_book_id: "src".to_owned(),
                desk_id: "fx".to_owned(),
                trader: "alice".to_owned(),
                position_ids: vec![1],
            },
            target: TransferLeg {
                risk_book_id: "tgt".to_owned(),
                desk_id: "em".to_owned(),
                trader: String::new(),
                position_ids: vec![],
            },
            quantity: TransferQuantity::Full,
            price: TransferPrice::Mid,
            reason: String::new(),
            initiated_by: "alice".to_owned(),
            initiated_at: at,
            state,
            approver: None,
            decided_at: None,
            provenance: None,
        }
    }

    #[test]
    fn mints_distinct_monotonic_ids() {
        let reg = RiskTransferRegistry::new();
        let a = reg.mint_id();
        let b = reg.mint_id();
        assert_ne!(a, b);
        assert_eq!(a, "risk-transfer-1");
        assert_eq!(b, "risk-transfer-2");
    }

    #[test]
    fn insert_get_replace_roundtrip() {
        let reg = RiskTransferRegistry::new();
        reg.insert(transfer("t1", 10, TransferState::Pending));
        assert_eq!(reg.get("t1").unwrap().state, TransferState::Pending);

        let mut booked = reg.get("t1").unwrap();
        booked.state = TransferState::Booked;
        assert!(reg.replace(booked));
        assert_eq!(reg.get("t1").unwrap().state, TransferState::Booked);
        assert!(!reg.replace(transfer("nope", 1, TransferState::Draft)));
    }

    #[test]
    fn snapshot_is_newest_first_and_pending_filters() {
        let reg = RiskTransferRegistry::new();
        reg.insert(transfer("old", 10, TransferState::Booked));
        reg.insert(transfer("new", 20, TransferState::Pending));
        let snap = reg.snapshot();
        assert_eq!(snap[0].id, "new", "newest first");
        assert_eq!(snap[1].id, "old");
        let pending = reg.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "new");
    }
}
