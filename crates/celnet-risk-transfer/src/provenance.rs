//! The immutable audit record — mirrors the `PricingProvenance` waterfall
//! discipline (structured, additive, carried on the record) so the blotter and
//! dashboard can show exactly what moved, at what price, by whom, approved by
//! whom (§5.3).

use crate::risk::MovedRisk;
use crate::transfer::{TransferKind, TransferQuantity};

/// Which basis the recorded transfer price was resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceBasis {
    /// The live composite mid.
    Mid,
    /// The current independent mark.
    MarkToMarket,
    /// An operator-agreed override (flagged for control).
    Agreed,
}

/// The immutable, append-only audit record stamped on a `Booked` transfer and
/// carried on both booking legs / the deal record.
///
/// Same discipline as the pricing waterfall: system-generated, attributable to
/// the authenticated initiator + approver, tied to the exact
/// positions/price/quantity, with the crystallised P&L and moved risk recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskTransferProvenance {
    /// The transfer's stable slug.
    pub transfer_id: String,
    /// The kind of move.
    pub kind: TransferKind,
    /// The authenticated initiator.
    pub initiated_by: String,
    /// Trusted-source initiation timestamp.
    pub initiated_at: i64,
    /// The accepting/approving user (four-eyes); `None` for single-control
    /// re-attribution.
    pub approver: Option<String>,
    /// When the decision was made.
    pub decided_at: Option<i64>,
    /// The source book the risk left.
    pub source_book_id: String,
    /// The target book the risk arrived in.
    pub target_book_id: String,
    /// The positions moved.
    pub position_ids: Vec<u64>,
    /// How much was moved.
    pub quantity: TransferQuantity,
    /// The numeric transfer price the legs booked at.
    pub transfer_price: f64,
    /// Which basis that price came from.
    pub price_basis: PriceBasis,
    /// The rationale (required for an agreed price).
    pub reason: String,
    /// P&L crystallised in the source at the transfer price versus each slice's
    /// mark.
    pub realized_pnl_source: f64,
    /// The aggregate risk moved from source to target.
    pub risk_moved: MovedRisk,
}
