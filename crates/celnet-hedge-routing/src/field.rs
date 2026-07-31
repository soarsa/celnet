//! The risk-state fields an exit-policy rule branches on — the typed vocabulary a
//! hedge condition's left-hand side is drawn from.
//!
//! Every rule is `IF <field> <op> <value> THEN <exit-action>`. The operator
//! ([`RouteOp`]) and literal ([`RouteValue`]) come **verbatim** from
//! `celnet-risk-routing` — the same total, never-panicking comparison matrix that
//! risk routing uses. This module owns only the field set: [`HedgeField`] (which
//! risk-state attribute) and its [`FieldKind`] classification, which drives which
//! operators are legal (`RouteOp::valid_for`) exactly as in routing. A malformed
//! rule such as `net_dv01 contains "x"` is rejected by validation and, in the UI,
//! unrepresentable.

use celnet_risk_routing::FieldKind;
use serde::{Deserialize, Serialize};

/// A `celnet-limits`-style alias kept local for clarity in this crate's docs; the
/// underlying type is `celnet-risk-routing`'s [`FieldKind`] so the operator matrix
/// (`RouteOp::valid_for`) applies unchanged.
pub type HedgeFieldKind = FieldKind;

/// A risk-state field an exit-policy rule can match against. The one-to-one image
/// of [`crate::HedgeContext`]'s fields. New routable fields are added by extending
/// both in lock-step.
///
/// Where `celnet-risk-routing`'s `RouteField` snapshots one *fill*, [`HedgeField`]
/// snapshots one `(book × instrument)` *risk state*: the net risk, its budget
/// band, the flow quality that built it, and the market's current offset/hedge
/// cost (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HedgeField {
    // ---- identity -----------------------------------------------------------
    /// Instrument identifier / symbol (string).
    InstrumentId,
    /// Currency or pair (enum).
    Ccy,
    /// Product family — vanilla / swap / bond / … (enum).
    Product,
    /// Owning risk book / portfolio (enum).
    Book,
    /// Owning desk (enum).
    Desk,

    // ---- risk state ---------------------------------------------------------
    /// Signed net DV01 of the book (numeric, FI budget metric).
    NetDv01,
    /// Signed net notional / base-currency delta (numeric, FX budget metric).
    NetNotional,
    /// Signed net vega per `1.0` vol (numeric).
    NetVega,
    /// Signed net gamma (numeric).
    NetGamma,
    /// Inventory sign — `+1` long / `−1` short (numeric).
    InventorySign,

    // ---- budget state -------------------------------------------------------
    /// The resolved warehouse threshold (the "100") for this scope (numeric).
    Threshold,
    /// `|net_risk| / threshold` — the RAG utilisation ratio (numeric).
    Utilization,
    /// `max(0, |net_risk| − target)` — the overflow beyond the band edge (numeric).
    Overflow,
    /// Whether the risk is at/over the red band — the hedge trigger. Modelled as
    /// an **enum** yielding the text `"true"` / `"false"` so a rule reads
    /// `breached == false` (no boolean field kind exists; enum equality suffices).
    Breached,

    // ---- flow quality -------------------------------------------------------
    /// Markout / residual-toxicity of the flow that built this inventory
    /// (numeric, from analytics §11.1); high ⇒ hedge sooner.
    CounterpartyToxicity,
    /// How long this risk has sat, in seconds (numeric, inventory aging).
    InventoryAgeSecs,

    // ---- market state -------------------------------------------------------
    /// Opposing internal flow the aggregator could cross now (numeric).
    InternalOffsetAvailable,
    /// Current external hedge-cost estimate — spread + impact, in bp (numeric).
    HedgeCostBp,
}

impl HedgeField {
    /// The [`FieldKind`] of this field — the single source of truth for the
    /// operator/value type matrix used by `RouteOp::valid_for` and validation.
    ///
    /// `Breached` is an [`FieldKind::Enum`] (compared by `== "true"/"false"`);
    /// all other risk-state numbers are [`FieldKind::Numeric`]; only
    /// `InstrumentId` is free-form [`FieldKind::String`].
    #[must_use]
    pub fn kind(self) -> FieldKind {
        match self {
            HedgeField::Ccy
            | HedgeField::Product
            | HedgeField::Book
            | HedgeField::Desk
            | HedgeField::Breached => FieldKind::Enum,
            HedgeField::NetDv01
            | HedgeField::NetNotional
            | HedgeField::NetVega
            | HedgeField::NetGamma
            | HedgeField::InventorySign
            | HedgeField::Threshold
            | HedgeField::Utilization
            | HedgeField::Overflow
            | HedgeField::CounterpartyToxicity
            | HedgeField::InventoryAgeSecs
            | HedgeField::InternalOffsetAvailable
            | HedgeField::HedgeCostBp => FieldKind::Numeric,
            HedgeField::InstrumentId => FieldKind::String,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_routing::RouteOp;

    #[test]
    fn field_kinds_are_correct() {
        for f in [
            HedgeField::Ccy,
            HedgeField::Product,
            HedgeField::Book,
            HedgeField::Desk,
            HedgeField::Breached,
        ] {
            assert_eq!(f.kind(), FieldKind::Enum, "{f:?} should be Enum");
        }
        for f in [
            HedgeField::NetDv01,
            HedgeField::NetNotional,
            HedgeField::NetVega,
            HedgeField::NetGamma,
            HedgeField::InventorySign,
            HedgeField::Threshold,
            HedgeField::Utilization,
            HedgeField::Overflow,
            HedgeField::CounterpartyToxicity,
            HedgeField::InventoryAgeSecs,
            HedgeField::InternalOffsetAvailable,
            HedgeField::HedgeCostBp,
        ] {
            assert_eq!(f.kind(), FieldKind::Numeric, "{f:?} should be Numeric");
        }
        assert_eq!(HedgeField::InstrumentId.kind(), FieldKind::String);
    }

    #[test]
    fn breached_supports_enum_equality_only() {
        // The example `breached == false` uses Eq (valid for Enum); ordering is not.
        assert!(RouteOp::Eq.valid_for(HedgeField::Breached.kind()));
        assert!(!RouteOp::Gt.valid_for(HedgeField::Breached.kind()));
    }

    #[test]
    fn numeric_fields_support_ordering() {
        for op in [
            RouteOp::Gt,
            RouteOp::Ge,
            RouteOp::Lt,
            RouteOp::Le,
            RouteOp::Between,
        ] {
            assert!(op.valid_for(HedgeField::Overflow.kind()), "{op:?}");
        }
    }
}
