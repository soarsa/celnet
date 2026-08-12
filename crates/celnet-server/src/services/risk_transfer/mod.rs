//! Risk transfer — the server orchestration of the MANUAL move of EXISTING risk
//! between books / desks / traders (`docs/RISK-TRANSFER-REQUIREMENTS.md`; the
//! complement to risk routing). This module owns the **lifecycle** (initiate →
//! pending → accept/reject/cancel → booked), the **four-eyes** control (an accept
//! must be a different authenticated principal than the initiator, §7), the
//! **immutable audit** assembly (a [`RiskTransferProvenance`] stamped on every booked
//! record, §5.3), and the **inbox fan-out** (the counterparty push, §9.2). The pure
//! validation + leg arithmetic lives in the `celnet_risk_transfer` crate; the store
//! re-stamp / offsetting-booking apply lives in
//! [`TransferApplier`](crate::services::transfer_apply::TransferApplier); this layer
//! sequences them and never touches the pinned pricing core (guardrail 11 — it runs on
//! the async booking tier the booking sinks already run on).

// The service edge returns `tonic::Status` by value on the small-`Ok` paths, exactly
// as `services::auth` / `services::desk` do — the shared codebase convention (the
// `Status` Err dominates a small `Ok`, but boxing it would churn every call site for
// no runtime win off the pinned pricing core).
#![allow(clippy::result_large_err)]

pub mod broker;
pub mod registry;
pub mod wire;

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use celnet_entitlements::AssetClass;
use celnet_proto as pb;
use celnet_risk_transfer::{
    MovedRisk, PriceBasis, RiskTransfer, RiskTransferProvenance, TransferError, TransferKind,
    TransferState, check_transfer,
};
use tonic::Status;

use crate::config::identity::IdentityStore;
use crate::services::transfer_apply::{TransferApplier, TransferAsset};

pub use broker::{INBOX_QUEUE_DEPTH, InboxSubscription, RiskTransferBroker};
pub use registry::RiskTransferRegistry;

/// The risk-transfer service: registry + inbox broker + apply engine + the identity
/// registry it validates transfer ends against. Shared behind an [`Arc`] into the
/// `AuthService` RPC edge (the initiate/accept/reject/cancel/list handlers) and the
/// notification-stream layer (the inbox subscription).
#[derive(Clone, Debug)]
pub struct RiskTransferService {
    registry: Arc<RiskTransferRegistry>,
    broker: Arc<RiskTransferBroker>,
    applier: Arc<TransferApplier>,
    identity: Arc<Mutex<IdentityStore>>,
}

impl RiskTransferService {
    /// Assemble the service from its shared collaborators.
    #[must_use]
    pub fn new(
        registry: Arc<RiskTransferRegistry>,
        broker: Arc<RiskTransferBroker>,
        applier: Arc<TransferApplier>,
        identity: Arc<Mutex<IdentityStore>>,
    ) -> Self {
        Self {
            registry,
            broker,
            applier,
            identity,
        }
    }

    /// The shared inbox broker (for the notification-stream subscription layer).
    #[must_use]
    pub fn broker(&self) -> &Arc<RiskTransferBroker> {
        &self.broker
    }

    /// The asset class the selected positions live under — the axis the caller's
    /// `risk_transfer` capability is gated on (§7). Resolved by probing the live
    /// stores (FX vs rates); the handler gates BEFORE any mutation.
    ///
    /// # Errors
    /// `invalid_argument` when the positions resolve to no store or disagree.
    pub fn classify_asset_class(
        &self,
        source_book: &str,
        position_ids: &[u64],
    ) -> Result<AssetClass, Status> {
        Ok(match self.applier.classify(source_book, position_ids)? {
            TransferAsset::Fx => AssetClass::FxOptions,
            TransferAsset::Rates => AssetClass::FixedIncome,
        })
    }

    /// The asset class of an already-recorded transfer (for gating accept/reject/cancel).
    ///
    /// # Errors
    /// `not_found` when no transfer carries `id`; `invalid_argument` on an unresolvable
    /// store.
    pub fn asset_class_of(&self, id: &str) -> Result<AssetClass, Status> {
        let t = self.require(id)?;
        self.classify_asset_class(&t.source.risk_book_id, &t.source.position_ids)
    }

    /// Initiate a transfer (the ticket). Re-attribution (same desk, single-control)
    /// books immediately; a desk-to-desk / trader-to-trader transfer is recorded
    /// `Pending` for the counterparty inbox. The caller has already been
    /// capability-gated by the handler.
    ///
    /// # Errors
    /// Maps every `check_transfer` invariant failure + apply failure to a typed
    /// `tonic::Status`.
    pub fn initiate(
        &self,
        caller: &str,
        req: pb::InitiateRiskTransferRequest,
    ) -> Result<pb::RiskTransfer, Status> {
        let kind = wire::kind_from_wire(req.kind)?;
        let mut source = wire::leg_from_wire(req.source.unwrap_or_default());
        let mut target = wire::leg_from_wire(req.target.unwrap_or_default());
        // Desks are authoritative from the identity registry (never client-asserted):
        // this makes the kind ↔ desk consistency check meaningful (a ReAttribute whose
        // books sit on different desks is rejected as a cross-desk move).
        if let Some(desk) = self.book_desk(&source.risk_book_id) {
            source.desk_id = desk;
        }
        if let Some(desk) = self.book_desk(&target.risk_book_id) {
            target.desk_id = desk;
        }
        // The initiating trader is the authenticated caller (attribution).
        source.trader = caller.to_owned();

        let quantity = wire::quantity_from_wire(req.quantity_full, req.partial_notional)?;
        let price = wire::price_from_wire(req.price_basis, req.agreed_price)?;

        let transfer = RiskTransfer {
            id: self.registry.mint_id(),
            kind,
            source,
            target,
            quantity,
            price,
            reason: req.reason,
            initiated_by: caller.to_owned(),
            initiated_at: now_nanos(),
            state: TransferState::Draft,
            approver: None,
            decided_at: None,
            provenance: None,
        };

        let source_book = transfer.source.risk_book_id.clone();
        let pids = transfer.source.position_ids.clone();
        let asset = self.applier.classify(&source_book, &pids)?;

        // Validate against the live store/registry snapshot (§5.1 invariants).
        let ctx = self
            .applier
            .build_context(&self.books_map(), &source_book, &pids, asset);
        check_transfer(&transfer, &ctx).map_err(transfer_error_status)?;

        match kind {
            // Single-control administrative move: re-stamp and book immediately (§6.1).
            TransferKind::ReAttribute => {
                let mut booked = transfer;
                // Resolve the price BEFORE the re-stamp — the re-stamp moves the position
                // out of the source book, after which its slice can no longer be read
                // against the source.
                let (price_num, basis) =
                    self.applier
                        .resolve_price(booked.price, &source_book, &pids, asset)?;
                let moved = self.applier.apply_reattribute(
                    &source_book,
                    &booked.target.risk_book_id,
                    &pids,
                    booked.quantity,
                    asset,
                )?;
                booked.decided_at = Some(now_nanos());
                booked.state = TransferState::Booked;
                // A re-attribution crosses NO P&L (economics unchanged, §3.1): the
                // realised source P&L is exactly zero; only the risk re-buckets.
                booked.provenance = Some(build_provenance(&booked, price_num, basis, 0.0, moved));
                // TRANSFER (class=Transfer → orders sink): a single-control administrative
                // re-attribution booked immediately (§6.1) — the risk re-buckets, no P&L.
                tracing::info!(
                    class = celnet_observability::LogClass::Transfer.label(),
                    transfer_id = %booked.id,
                    kind = "reattribute",
                    from_book = %booked.source.risk_book_id,
                    to_book = %booked.target.risk_book_id,
                    quantity = ?booked.quantity,
                    initiator = %booked.initiated_by,
                    moved_risk = ?moved,
                    "risk transfer re-attributed (booked)",
                );
                self.registry.insert(booked.clone());
                self.publish_inbox();
                Ok(wire::transfer_to_wire(&booked))
            }
            // Economic cross across the org boundary: park it Pending for the
            // counterparty's four-eyes acceptance (§6.2 / §7). No booking yet — the
            // risk stays in the source until accepted (no limbo).
            TransferKind::DeskToDesk | TransferKind::TraderToTrader => {
                let mut pending = transfer;
                pending.state = TransferState::Pending;
                // TRANSFER (class=Transfer → orders sink): an economic cross parked Pending
                // for the counterparty's four-eyes acceptance (§6.2) — no legs booked yet.
                tracing::info!(
                    class = celnet_observability::LogClass::Transfer.label(),
                    transfer_id = %pending.id,
                    kind = ?pending.kind,
                    from_book = %pending.source.risk_book_id,
                    to_book = %pending.target.risk_book_id,
                    quantity = ?pending.quantity,
                    initiator = %pending.initiated_by,
                    "risk transfer initiated (pending four-eyes)",
                );
                self.registry.insert(pending.clone());
                self.publish_inbox();
                Ok(wire::transfer_to_wire(&pending))
            }
        }
    }

    /// Accept a `Pending` transfer (target side). Four-eyes: the approver must be a
    /// DIFFERENT authenticated principal than the initiator (§7). Books the two
    /// offsetting legs atomically and stamps provenance.
    ///
    /// # Errors
    /// `not_found` / `failed_precondition` (not pending) / `permission_denied`
    /// (approver == initiator) / the apply failure.
    pub fn accept(&self, caller: &str, id: &str) -> Result<pb::RiskTransfer, Status> {
        let mut t = self.require_pending(id)?;
        if caller == t.initiated_by {
            return Err(Status::permission_denied(
                "four-eyes: a transfer must be accepted by a different principal than its initiator",
            ));
        }
        let source_book = t.source.risk_book_id.clone();
        let pids = t.source.position_ids.clone();
        let asset = self.applier.classify(&source_book, &pids)?;
        let (price_num, basis) = self
            .applier
            .resolve_price(t.price, &source_book, &pids, asset)?;

        let legs = self.applier.apply_economic(
            &source_book,
            &t.target.risk_book_id,
            &t.source.desk_id,
            &t.source.trader,
            &t.target.desk_id,
            &t.target.trader,
            &pids,
            t.quantity,
            price_num,
            asset,
        )?;

        t.approver = Some(caller.to_owned());
        t.decided_at = Some(now_nanos());
        t.state = TransferState::Booked;
        t.provenance = Some(build_provenance(
            &t,
            price_num,
            basis,
            legs.realized_pnl_source,
            legs.moved_risk,
        ));
        // TRANSFER (class=Transfer → orders sink): the four-eyes accept booked the two
        // offsetting legs. From/to book, quantity, approver + initiator, realised source
        // P&L. Async control-plane edge — never the pinned pricing core.
        tracing::info!(
            class = celnet_observability::LogClass::Transfer.label(),
            transfer_id = %id,
            from_book = %t.source.risk_book_id,
            to_book = %t.target.risk_book_id,
            quantity = ?t.quantity,
            price = ?t.price,
            approver = %caller,
            initiator = %t.initiated_by,
            realized_pnl_source = legs.realized_pnl_source,
            moved_risk = ?legs.moved_risk,
            "risk transfer accepted (booked)",
        );
        self.registry.replace(t.clone());
        self.publish_inbox();
        Ok(wire::transfer_to_wire(&t))
    }

    /// Reject a `Pending` transfer (target side). Like accept, only the counterparty
    /// (not the initiator) may reject; the initiator withdraws via [`Self::cancel`].
    ///
    /// # Errors
    /// `not_found` / `failed_precondition` / `permission_denied`.
    pub fn reject(
        &self,
        caller: &str,
        id: &str,
        _reason: &str,
    ) -> Result<pb::RiskTransfer, Status> {
        let mut t = self.require_pending(id)?;
        if caller == t.initiated_by {
            return Err(Status::permission_denied(
                "a transfer is rejected by its counterparty; the initiator cancels instead",
            ));
        }
        t.approver = Some(caller.to_owned());
        t.decided_at = Some(now_nanos());
        t.state = TransferState::Rejected;
        // TRANSFER (class=Transfer → orders sink): the counterparty rejected the pending
        // transfer — WARN (the risk stays in the source book, no legs booked).
        tracing::warn!(
            class = celnet_observability::LogClass::Transfer.label(),
            transfer_id = %id,
            from_book = %t.source.risk_book_id,
            to_book = %t.target.risk_book_id,
            quantity = ?t.quantity,
            rejected_by = %caller,
            initiator = %t.initiated_by,
            reason = %_reason,
            "risk transfer rejected",
        );
        self.registry.replace(t.clone());
        self.publish_inbox();
        Ok(wire::transfer_to_wire(&t))
    }

    /// Cancel a `Pending` transfer — the initiator withdraws it pre-acceptance.
    ///
    /// # Errors
    /// `not_found` / `failed_precondition` / `permission_denied` (only the initiator
    /// may cancel).
    pub fn cancel(&self, caller: &str, id: &str) -> Result<pb::RiskTransfer, Status> {
        let mut t = self.require_pending(id)?;
        if caller != t.initiated_by {
            return Err(Status::permission_denied(
                "only the initiator may cancel a transfer",
            ));
        }
        t.decided_at = Some(now_nanos());
        t.state = TransferState::Cancelled;
        // TRANSFER (class=Transfer → orders sink): the initiator withdrew the pending
        // transfer pre-acceptance.
        tracing::info!(
            class = celnet_observability::LogClass::Transfer.label(),
            transfer_id = %id,
            from_book = %t.source.risk_book_id,
            to_book = %t.target.risk_book_id,
            quantity = ?t.quantity,
            initiator = %caller,
            "risk transfer cancelled by initiator",
        );
        self.registry.replace(t.clone());
        self.publish_inbox();
        Ok(wire::transfer_to_wire(&t))
    }

    /// The transfer blotter / audit trail, filtered by desk / trader / book / state
    /// (each absent filter is unrestricted). Newest first.
    #[must_use]
    pub fn list(&self, req: &pb::ListRiskTransfersRequest) -> Vec<pb::RiskTransfer> {
        let states: Vec<TransferState> = req
            .states
            .iter()
            .filter_map(|s| wire::state_from_wire(*s))
            .collect();
        self.registry
            .snapshot()
            .iter()
            .filter(|t| {
                req.desk
                    .as_ref()
                    .is_none_or(|d| &t.source.desk_id == d || &t.target.desk_id == d)
                    && req
                        .trader
                        .as_ref()
                        .is_none_or(|tr| &t.source.trader == tr || &t.target.trader == tr)
                    && req
                        .risk_book_id
                        .as_ref()
                        .is_none_or(|b| &t.source.risk_book_id == b || &t.target.risk_book_id == b)
                    && (states.is_empty() || states.contains(&t.state))
            })
            .map(wire::transfer_to_wire)
            .collect()
    }

    /// The current `Pending` roster, as wire records — the initial snapshot a fresh
    /// inbox subscriber receives.
    #[must_use]
    pub fn pending_wire(&self) -> Vec<pb::RiskTransfer> {
        self.registry
            .pending()
            .iter()
            .map(wire::transfer_to_wire)
            .collect()
    }

    /// Publish the current `Pending` roster to every inbox subscriber (each sees only
    /// the transfers whose target desk it is entitled to). Called on every lifecycle
    /// transition.
    pub fn publish_inbox(&self) {
        self.broker.publish(&self.pending_wire(), now_nanos());
    }

    // --- internals ----------------------------------------------------------

    fn require(&self, id: &str) -> Result<RiskTransfer, Status> {
        self.registry
            .get(id)
            .ok_or_else(|| Status::not_found(format!("no risk transfer with id `{id}`")))
    }

    fn require_pending(&self, id: &str) -> Result<RiskTransfer, Status> {
        let t = self.require(id)?;
        if t.state != TransferState::Pending {
            return Err(Status::failed_precondition(format!(
                "risk transfer `{id}` is not pending (state {:?})",
                t.state
            )));
        }
        Ok(t)
    }

    /// The owning desk of a risk book from the identity registry (`None` when the
    /// book is unknown or unowned).
    fn book_desk(&self, book_id: &str) -> Option<String> {
        let g = self.identity.lock().expect("identity store lock poisoned");
        g.risk_books
            .iter()
            .find(|b| b.id == book_id)
            .and_then(|b| b.desk_id.clone())
    }

    /// The book map `check_transfer` validates against — every risk book keyed by id.
    fn books_map(&self) -> std::collections::HashMap<String, celnet_risk_transfer::BookRef> {
        let g = self.identity.lock().expect("identity store lock poisoned");
        g.risk_books
            .iter()
            .map(|b| {
                (
                    b.id.clone(),
                    celnet_risk_transfer::BookRef {
                        desk_id: b.desk_id.clone().unwrap_or_default(),
                        enabled: b.enabled,
                    },
                )
            })
            .collect()
    }
}

/// Stamp the immutable audit record from a transfer + its resolved economics.
fn build_provenance(
    t: &RiskTransfer,
    transfer_price: f64,
    price_basis: PriceBasis,
    realized_pnl_source: f64,
    risk_moved: MovedRisk,
) -> RiskTransferProvenance {
    RiskTransferProvenance {
        transfer_id: t.id.clone(),
        kind: t.kind,
        initiated_by: t.initiated_by.clone(),
        initiated_at: t.initiated_at,
        approver: t.approver.clone(),
        decided_at: t.decided_at,
        source_book_id: t.source.risk_book_id.clone(),
        target_book_id: t.target.risk_book_id.clone(),
        position_ids: t.source.position_ids.clone(),
        quantity: t.quantity,
        transfer_price,
        price_basis,
        reason: t.reason.clone(),
        realized_pnl_source,
        risk_moved,
    }
}

/// Map a pure-domain [`TransferError`] to its wire [`Status`] — one variant per
/// invariant so the client sees a precise, trader-facing reason.
fn transfer_error_status(e: TransferError) -> Status {
    match &e {
        TransferError::PositionNotFound(_)
        | TransferError::SourceBookNotFound(_)
        | TransferError::TargetBookNotFound(_) => Status::not_found(e.to_string()),
        TransferError::SourceBookDisabled(_)
        | TransferError::TargetBookDisabled(_)
        | TransferError::PositionNotInSourceBook { .. } => {
            Status::failed_precondition(e.to_string())
        }
        _ => Status::invalid_argument(e.to_string()),
    }
}

/// The current wall-clock time in nanoseconds since the Unix epoch — the trusted
/// source timestamp on a transfer's initiation / decision (§7).
fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto as pb;
    use celnet_proto::{BookId, Owner, owner};
    use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

    use crate::config::identity::{IdentityStore, RiskBookDef};
    use crate::services::rates_book::RatesPositionStore;
    use crate::services::risk::store::{BookedPosition, PositionStore, RiskBookLimitDef};

    fn booked(id: u64, notional: f64) -> BookedPosition {
        BookedPosition {
            position_id: id,
            pair: CcyPair::new(Ccy::EUR, Ccy::USD),
            option: OptionType::Call,
            notional_base: notional,
            inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        }
    }

    fn attribution(book: &str) -> celnet_proto::AttributionRecord {
        celnet_proto::AttributionRecord {
            quoted_by: None,
            held_by: Some(BookId {
                book: book.to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::Trader("t1".to_owned())),
                }),
            }),
            won: None,
            lp_count: None,
        }
    }

    fn book_limit(id: &str) -> RiskBookLimitDef {
        RiskBookLimitDef {
            id: id.to_owned(),
            parent_id: None,
            limits: None,
        }
    }

    fn risk_book(id: &str, desk: &str) -> RiskBookDef {
        RiskBookDef {
            id: id.to_owned(),
            name: id.to_owned(),
            parent_id: None,
            desk_id: Some(desk.to_owned()),
            description: String::new(),
            limits: None,
            asset_class: crate::config::identity::default_risk_book_asset_class(),
            enabled: true,
        }
    }

    /// A service over a fresh FX store with position 1 (10mm EUR call) booked into
    /// `BOOK-A`, and an identity registry with `BOOK-A`@desk-fx, `BOOK-B`@desk-em, and
    /// `BOOK-C`@desk-fx (all enabled, uncapped).
    fn fixture() -> (RiskTransferService, Arc<PositionStore>) {
        let store = Arc::new(PositionStore::new());
        store.set_risk_books(vec![
            book_limit("BOOK-A"),
            book_limit("BOOK-B"),
            book_limit("BOOK-C"),
        ]);
        store
            .book_into_risk_book(booked(1, 10_000_000.0), &attribution("BOOK-A"), "BOOK-A")
            .expect("seed position into BOOK-A");
        let rates = Arc::new(RatesPositionStore::new());
        let applier = Arc::new(TransferApplier::new(Arc::clone(&store), rates));
        let mut identity = IdentityStore::default();
        identity.risk_books.push(risk_book("BOOK-A", "desk-fx"));
        identity.risk_books.push(risk_book("BOOK-B", "desk-em"));
        identity.risk_books.push(risk_book("BOOK-C", "desk-fx"));
        let service = RiskTransferService::new(
            Arc::new(RiskTransferRegistry::new()),
            Arc::new(RiskTransferBroker::new()),
            applier,
            Arc::new(Mutex::new(identity)),
        );
        (service, store)
    }

    fn initiate_req(
        kind: pb::TransferKind,
        source_book: &str,
        target_book: &str,
    ) -> pb::InitiateRiskTransferRequest {
        pb::InitiateRiskTransferRequest {
            session_token: String::new(),
            kind: kind as i32,
            source: Some(pb::TransferLeg {
                risk_book_id: source_book.to_owned(),
                desk_id: String::new(),
                trader: String::new(),
                position_ids: vec![1],
            }),
            target: Some(pb::TransferLeg {
                risk_book_id: target_book.to_owned(),
                desk_id: String::new(),
                trader: String::new(),
                position_ids: vec![],
            }),
            quantity_full: true,
            partial_notional: None,
            price_basis: pb::TransferPriceBasis::Mid as i32,
            agreed_price: None,
            reason: String::new(),
            correlation_id: None,
        }
    }

    /// A same-desk re-attribution books IMMEDIATELY (single control): the position
    /// re-stamps into the target book, provenance is stamped, and the risk version bumps.
    #[test]
    fn reattribute_initiates_booked_with_provenance() {
        let (service, store) = fixture();
        let v0 = store.risk_version();
        let wire = service
            .initiate(
                "alice",
                initiate_req(pb::TransferKind::ReAttribute, "BOOK-A", "BOOK-C"),
            )
            .expect("a same-desk re-attribution books");
        assert_eq!(wire.state, pb::TransferState::Booked as i32);
        assert!(
            wire.provenance.is_some(),
            "a booked transfer stamps provenance"
        );
        assert_eq!(
            store.risk_book_of(1).as_deref(),
            Some("BOOK-C"),
            "the position re-stamped into the target book"
        );
        assert!(
            store.risk_version() > v0,
            "the re-stamp bumps the risk version"
        );
    }

    /// A cross-desk economic transfer parks `Pending`; four-eyes then forbids the
    /// INITIATOR from accepting their own transfer, but a DIFFERENT principal books it
    /// (initiate → Pending → accept → Booked + provenance + approver).
    #[test]
    fn four_eyes_forbids_self_accept_then_counterparty_books() {
        let (service, _store) = fixture();
        let pending = service
            .initiate(
                "alice",
                initiate_req(pb::TransferKind::DeskToDesk, "BOOK-A", "BOOK-B"),
            )
            .expect("a cross-desk transfer parks pending");
        assert_eq!(
            pending.state,
            pb::TransferState::Pending as i32,
            "an economic transfer needs acceptance"
        );
        assert!(pending.provenance.is_none(), "not booked yet");
        let id = pending.id;

        // Four-eyes: the initiator cannot accept their own transfer.
        let denied = service
            .accept("alice", &id)
            .expect_err("self-accept is refused");
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);

        // A different authenticated principal books it.
        let booked = service
            .accept("bob", &id)
            .expect("a distinct counterparty accepts");
        assert_eq!(booked.state, pb::TransferState::Booked as i32);
        assert_eq!(booked.approver.as_deref(), Some("bob"));
        assert!(
            booked.provenance.is_some(),
            "the booked transfer stamps its audit provenance"
        );
    }
}
