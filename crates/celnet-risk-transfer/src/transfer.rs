//! The transfer request/record object and its enums.
//!
//! Mirrors `docs/hedging/RISK-TRANSFER-REQUIREMENTS.md` §5.1. Plain owned types; no
//! serde/prost here — the proto + server phases mirror these shapes on the wire.

use crate::provenance::RiskTransferProvenance;

/// The kind of move, which fixes the economics and the approval model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    /// Portfolio → portfolio **within the same desk**. Economics unchanged; a
    /// re-stamp of the routing dimension. Single-control (same owner both ends).
    ReAttribute,
    /// Portfolio/desk → **another desk's** portfolio. An economic internal cross
    /// that realises P&L in the source and opens risk in the target at the
    /// transfer price. Requires target-desk acceptance (four-eyes).
    DeskToDesk,
    /// Trader → trader hand-off (same or different book). Economic if the books
    /// differ; a light re-attribution if they do not. Requires recipient
    /// acceptance.
    TraderToTrader,
}

/// One end of a transfer — a book, its owning desk, the trader, and (on the
/// source) the specific positions selected.
#[derive(Debug, Clone, PartialEq)]
pub struct TransferLeg {
    /// The risk portfolio (internally a risk-book id).
    pub risk_book_id: String,
    /// The desk that owns the book.
    pub desk_id: String,
    /// The trader on this end (may be empty on a desk-level target).
    pub trader: String,
    /// The positions selected (populated on the source; empty on the target).
    pub position_ids: Vec<u64>,
}

/// How much of the selected positions to move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransferQuantity {
    /// Move the whole selected position(s).
    Full,
    /// Move a base-currency notional magnitude, pro-rata across the selected
    /// slices. Must satisfy `0 < q <= |aggregate signed notional|`.
    Partial(f64),
}

/// The internal cross level the economic legs book at (§3.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransferPrice {
    /// The live composite mid.
    Mid,
    /// The current independent mark.
    MarkToMarket,
    /// An operator-agreed override. Requires a non-empty
    /// [`RiskTransfer::reason`] so off-mark internal crosses are visible to
    /// control.
    Agreed(f64),
}

impl TransferPrice {
    /// The [`crate::PriceBasis`] this price resolves to (drops the agreed
    /// value), for stamping the audit record.
    #[must_use]
    pub fn basis(&self) -> crate::PriceBasis {
        match self {
            TransferPrice::Mid => crate::PriceBasis::Mid,
            TransferPrice::MarkToMarket => crate::PriceBasis::MarkToMarket,
            TransferPrice::Agreed(_) => crate::PriceBasis::Agreed,
        }
    }
}

/// The transfer lifecycle state (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferState {
    /// Being composed; not yet submitted.
    Draft,
    /// Submitted and awaiting counterparty acceptance (desk-to-desk /
    /// trader-to-trader).
    Pending,
    /// Accepted by the counterparty; ready to book.
    Accepted,
    /// Declined (rejected or expired).
    Rejected,
    /// Booked — the legs applied, provenance stamped.
    Booked,
    /// Withdrawn by the initiator before acceptance.
    Cancelled,
}

/// A risk-transfer request/record (§5.1).
///
/// Validated by [`crate::check_transfer`]; its economic legs computed by
/// [`crate::compute_legs`]; its immutable audit record built from
/// [`RiskTransferProvenance`].
#[derive(Debug, Clone, PartialEq)]
pub struct RiskTransfer {
    /// Stable slug (audit key).
    pub id: String,
    /// The kind of move (fixes economics + approval).
    pub kind: TransferKind,
    /// The source end — the book/desk/trader the risk leaves, with the selected
    /// positions.
    pub source: TransferLeg,
    /// The target end — the book/desk/trader the risk arrives in.
    pub target: TransferLeg,
    /// How much to move.
    pub quantity: TransferQuantity,
    /// The transfer price basis.
    pub price: TransferPrice,
    /// Free-text rationale (required when `price` is [`TransferPrice::Agreed`]).
    pub reason: String,
    /// The authenticated user who initiated the transfer.
    pub initiated_by: String,
    /// Trusted-source initiation timestamp (epoch millis / nanos, server's
    /// choice — this crate treats it opaquely).
    pub initiated_at: i64,
    /// The current lifecycle state.
    pub state: TransferState,
    /// The accepting/approving user (four-eyes); `None` until decided.
    pub approver: Option<String>,
    /// When the accept/reject decision was made; `None` until decided.
    pub decided_at: Option<i64>,
    /// The immutable audit record, stamped on `Booked`.
    pub provenance: Option<RiskTransferProvenance>,
}
