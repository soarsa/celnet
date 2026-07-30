//! Wire ⇄ domain conversion for risk transfer — the boundary between the generated
//! `celnet_proto` messages and the pure `celnet_risk_transfer` domain types. The two
//! flat sum-type carriers (`quantity_full` + `partial_notional` ⇒ [`TransferQuantity`];
//! `price_basis` + `agreed_price` ⇒ [`TransferPrice`]) are (de)composed here so the
//! rest of the service works in the domain vocabulary (guardrail 9: one contract, the
//! proto is a pure projection).

// `tonic::Status` returned by value on small-`Ok` decode paths — the shared codebase
// convention (`services::auth` et al.).
#![allow(clippy::result_large_err)]

use celnet_proto as pb;
use celnet_risk_transfer::{
    MovedRisk, PriceBasis, RiskTransfer, RiskTransferProvenance, RiskVector, TransferKind,
    TransferLeg, TransferPrice, TransferQuantity, TransferState,
};
use tonic::Status;

/// Decode the wire transfer kind, rejecting the unspecified default (never valid).
pub fn kind_from_wire(v: i32) -> Result<TransferKind, Status> {
    match pb::TransferKind::try_from(v) {
        Ok(pb::TransferKind::ReAttribute) => Ok(TransferKind::ReAttribute),
        Ok(pb::TransferKind::DeskToDesk) => Ok(TransferKind::DeskToDesk),
        Ok(pb::TransferKind::TraderToTrader) => Ok(TransferKind::TraderToTrader),
        _ => Err(Status::invalid_argument("transfer kind is required")),
    }
}

/// Encode a domain transfer kind to its wire tag.
#[must_use]
pub fn kind_to_wire(kind: TransferKind) -> i32 {
    let w = match kind {
        TransferKind::ReAttribute => pb::TransferKind::ReAttribute,
        TransferKind::DeskToDesk => pb::TransferKind::DeskToDesk,
        TransferKind::TraderToTrader => pb::TransferKind::TraderToTrader,
    };
    w as i32
}

/// Encode a domain lifecycle state to its wire tag.
#[must_use]
pub fn state_to_wire(state: TransferState) -> i32 {
    let w = match state {
        TransferState::Draft => pb::TransferState::Draft,
        TransferState::Pending => pb::TransferState::Pending,
        TransferState::Accepted => pb::TransferState::Accepted,
        TransferState::Rejected => pb::TransferState::Rejected,
        TransferState::Booked => pb::TransferState::Booked,
        TransferState::Cancelled => pb::TransferState::Cancelled,
    };
    w as i32
}

/// Decode a wire lifecycle-state filter value, if it names a real state.
#[must_use]
pub fn state_from_wire(v: i32) -> Option<TransferState> {
    match pb::TransferState::try_from(v) {
        Ok(pb::TransferState::Draft) => Some(TransferState::Draft),
        Ok(pb::TransferState::Pending) => Some(TransferState::Pending),
        Ok(pb::TransferState::Accepted) => Some(TransferState::Accepted),
        Ok(pb::TransferState::Rejected) => Some(TransferState::Rejected),
        Ok(pb::TransferState::Booked) => Some(TransferState::Booked),
        Ok(pb::TransferState::Cancelled) => Some(TransferState::Cancelled),
        _ => None,
    }
}

/// Compose the flat wire quantity carriers into the domain quantity.
///
/// `quantity_full` wins; otherwise `partial_notional` must be present and is carried
/// as [`TransferQuantity::Partial`] (its bounds are validated by `check_transfer`).
pub fn quantity_from_wire(
    quantity_full: bool,
    partial_notional: Option<f64>,
) -> Result<TransferQuantity, Status> {
    if quantity_full {
        Ok(TransferQuantity::Full)
    } else {
        partial_notional
            .map(TransferQuantity::Partial)
            .ok_or_else(|| {
                Status::invalid_argument(
                    "a partial transfer requires `partial_notional` (or set `quantity_full`)",
                )
            })
    }
}

/// Decompose a domain quantity into the flat wire carriers `(quantity_full, partial)`.
#[must_use]
pub fn quantity_to_wire(quantity: TransferQuantity) -> (bool, Option<f64>) {
    match quantity {
        TransferQuantity::Full => (true, None),
        TransferQuantity::Partial(q) => (false, Some(q)),
    }
}

/// Compose the flat wire price carriers into the domain transfer price.
///
/// `AGREED` requires the accompanying `agreed_price`; `MID`/`MARK_TO_MARKET` ignore it.
pub fn price_from_wire(
    price_basis: i32,
    agreed_price: Option<f64>,
) -> Result<TransferPrice, Status> {
    match pb::TransferPriceBasis::try_from(price_basis) {
        Ok(pb::TransferPriceBasis::Mid) => Ok(TransferPrice::Mid),
        Ok(pb::TransferPriceBasis::MarkToMarket) => Ok(TransferPrice::MarkToMarket),
        Ok(pb::TransferPriceBasis::Agreed) => {
            agreed_price.map(TransferPrice::Agreed).ok_or_else(|| {
                Status::invalid_argument("an agreed transfer price requires `agreed_price`")
            })
        }
        _ => Err(Status::invalid_argument("transfer price basis is required")),
    }
}

/// Decompose a domain transfer price into the flat wire carriers `(basis, agreed)`.
#[must_use]
pub fn price_to_wire(price: TransferPrice) -> (i32, Option<f64>) {
    match price {
        TransferPrice::Mid => (pb::TransferPriceBasis::Mid as i32, None),
        TransferPrice::MarkToMarket => (pb::TransferPriceBasis::MarkToMarket as i32, None),
        TransferPrice::Agreed(x) => (pb::TransferPriceBasis::Agreed as i32, Some(x)),
    }
}

/// Encode a domain audit price basis to its wire tag.
#[must_use]
pub fn price_basis_to_wire(basis: PriceBasis) -> i32 {
    let w = match basis {
        PriceBasis::Mid => pb::PriceBasis::Mid,
        PriceBasis::MarkToMarket => pb::PriceBasis::MarkToMarket,
        PriceBasis::Agreed => pb::PriceBasis::Agreed,
    };
    w as i32
}

/// Decode a wire transfer leg into the domain leg.
#[must_use]
pub fn leg_from_wire(leg: pb::TransferLeg) -> TransferLeg {
    TransferLeg {
        risk_book_id: leg.risk_book_id,
        desk_id: leg.desk_id,
        trader: leg.trader,
        position_ids: leg.position_ids,
    }
}

/// Encode a domain transfer leg to the wire.
#[must_use]
pub fn leg_to_wire(leg: &TransferLeg) -> pb::TransferLeg {
    pb::TransferLeg {
        risk_book_id: leg.risk_book_id.clone(),
        desk_id: leg.desk_id.clone(),
        trader: leg.trader.clone(),
        position_ids: leg.position_ids.clone(),
    }
}

/// Encode a domain risk vector to the wire.
#[must_use]
pub fn risk_vector_to_wire(v: &RiskVector) -> pb::RiskVectorDesc {
    pb::RiskVectorDesc {
        dv01: v.dv01,
        delta: v.delta,
        gamma: v.gamma,
        vega: v.vega,
        theta: v.theta,
    }
}

/// Encode a domain moved-risk aggregate to the wire.
#[must_use]
pub fn moved_risk_to_wire(m: &MovedRisk) -> pb::MovedRiskDesc {
    pb::MovedRiskDesc {
        notional_base: m.notional_base,
        risk: Some(risk_vector_to_wire(&m.risk)),
    }
}

/// Encode a domain provenance record to the wire.
#[must_use]
pub fn provenance_to_wire(p: &RiskTransferProvenance) -> pb::RiskTransferProvenance {
    let (quantity_full, partial_notional) = quantity_to_wire(p.quantity);
    pb::RiskTransferProvenance {
        transfer_id: p.transfer_id.clone(),
        kind: kind_to_wire(p.kind),
        initiated_by: p.initiated_by.clone(),
        initiated_at: p.initiated_at,
        approver: p.approver.clone(),
        decided_at: p.decided_at,
        source_book_id: p.source_book_id.clone(),
        target_book_id: p.target_book_id.clone(),
        position_ids: p.position_ids.clone(),
        quantity_full,
        partial_notional,
        transfer_price: p.transfer_price,
        price_basis: price_basis_to_wire(p.price_basis),
        reason: p.reason.clone(),
        realized_pnl_source: p.realized_pnl_source,
        risk_moved: Some(moved_risk_to_wire(&p.risk_moved)),
    }
}

/// Encode a whole domain transfer record to the wire, carrying the resolved numeric
/// transfer price (from the stamped provenance) once known.
#[must_use]
pub fn transfer_to_wire(t: &RiskTransfer) -> pb::RiskTransfer {
    let (quantity_full, partial_notional) = quantity_to_wire(t.quantity);
    let (price_basis, agreed_price) = price_to_wire(t.price);
    pb::RiskTransfer {
        id: t.id.clone(),
        kind: kind_to_wire(t.kind),
        source: Some(leg_to_wire(&t.source)),
        target: Some(leg_to_wire(&t.target)),
        quantity_full,
        partial_notional,
        price_basis,
        agreed_price,
        reason: t.reason.clone(),
        initiated_by: t.initiated_by.clone(),
        initiated_at: t.initiated_at,
        state: state_to_wire(t.state),
        approver: t.approver.clone(),
        decided_at: t.decided_at,
        // The resolved numeric price is recorded on the provenance once booked.
        transfer_price: t.provenance.as_ref().map(|p| p.transfer_price),
        provenance: t.provenance.as_ref().map(provenance_to_wire),
    }
}
