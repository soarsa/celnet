//! The lift fields an acceptance rule branches on — the typed vocabulary an
//! acceptance condition's left-hand side is drawn from.
//!
//! Every rule is `IF <field> <op> <value> THEN <acceptance-action>`. The operator
//! ([`RouteOp`]) and literal ([`RouteValue`]) come **verbatim** from
//! `celnet-risk-routing` — the same total, never-panicking comparison matrix that
//! risk routing and hedge routing use. This module owns only the field set:
//! [`AcceptanceField`] (which attribute of the inbound lift) and its [`FieldKind`]
//! classification, which drives which operators are legal (`RouteOp::valid_for`)
//! exactly as in routing. A malformed rule such as `notional_usd contains "x"` is
//! rejected by validation and, in the UI, unrepresentable.

use celnet_risk_routing::FieldKind;
use serde::{Deserialize, Serialize};

/// A lift field an acceptance rule can match against. The one-to-one image of
//  [`crate::AcceptanceContext`]'s fields. New matchable fields are added by
/// extending both in lock-step.
///
/// Where `celnet-risk-routing`'s `RouteField` snapshots a *fill after it has
/// booked*, [`AcceptanceField`] snapshots the inbound *lift at the acceptance
/// point* — before we commit — so a rule can accept, reject, or hold a
/// counterparty's request on its economics, freshness, and captured edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AcceptanceField {
    /// Counterparty / originating party id (enum — matched by equality / membership).
    Counterparty,
    /// Absolute USD notional of the lift (numeric).
    NotionalUsd,
    /// Tenor / years to maturity of the lifted instrument (numeric).
    TenorYears,
    /// Instrument identifier / curve / security symbol (free-form string — supports
    /// substring so a rule can match a family prefix).
    InstrumentSymbol,
    /// Lift side — pay/receive/buy/sell (enum).
    Side,
    /// The dealer edge of the lift versus the engine mid, in basis points (numeric):
    /// positive ⇒ the desk deals better than mid (captured spread); non-positive ⇒
    /// dealt at or through mid. A rule such as `edge_bps < 0.2` rejects an
    /// unprofitable lift.
    EdgeBps,
    /// Age of the quote at lift time, in milliseconds (numeric): `now − mint_time`.
    /// A rule such as `quote_age_ms > 800` rejects a stale lift.
    QuoteAgeMs,
    /// Owning asset class — `"fx_options"` / `"fixed_income"` (enum).
    AssetClass,
    /// Owning / target desk (enum).
    Desk,
}

impl AcceptanceField {
    /// The [`FieldKind`] of this field — the single source of truth for the
    /// operator/value type matrix used by `RouteOp::valid_for` and validation.
    ///
    /// `NotionalUsd` / `TenorYears` / `EdgeBps` / `QuoteAgeMs` are
    /// [`FieldKind::Numeric`] (ordering + range); `InstrumentSymbol` is free-form
    /// [`FieldKind::String`] (equality / membership / substring); `Counterparty` /
    /// `Side` / `AssetClass` / `Desk` are [`FieldKind::Enum`] (equality / membership,
    /// no substring).
    #[must_use]
    pub fn kind(self) -> FieldKind {
        match self {
            AcceptanceField::NotionalUsd
            | AcceptanceField::TenorYears
            | AcceptanceField::EdgeBps
            | AcceptanceField::QuoteAgeMs => FieldKind::Numeric,
            AcceptanceField::InstrumentSymbol => FieldKind::String,
            AcceptanceField::Counterparty
            | AcceptanceField::Side
            | AcceptanceField::AssetClass
            | AcceptanceField::Desk => FieldKind::Enum,
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
            AcceptanceField::NotionalUsd,
            AcceptanceField::TenorYears,
            AcceptanceField::EdgeBps,
            AcceptanceField::QuoteAgeMs,
        ] {
            assert_eq!(f.kind(), FieldKind::Numeric, "{f:?} should be Numeric");
        }
        assert_eq!(AcceptanceField::InstrumentSymbol.kind(), FieldKind::String);
        for f in [
            AcceptanceField::Counterparty,
            AcceptanceField::Side,
            AcceptanceField::AssetClass,
            AcceptanceField::Desk,
        ] {
            assert_eq!(f.kind(), FieldKind::Enum, "{f:?} should be Enum");
        }
    }

    #[test]
    fn numeric_fields_support_ordering_and_range() {
        for op in [
            RouteOp::Gt,
            RouteOp::Ge,
            RouteOp::Lt,
            RouteOp::Le,
            RouteOp::Between,
        ] {
            assert!(op.valid_for(AcceptanceField::NotionalUsd.kind()), "{op:?}");
            assert!(op.valid_for(AcceptanceField::EdgeBps.kind()), "{op:?}");
        }
    }

    #[test]
    fn enum_fields_reject_ordering_and_substring() {
        for op in [
            RouteOp::Gt,
            RouteOp::Lt,
            RouteOp::Contains,
            RouteOp::Between,
        ] {
            assert!(
                !op.valid_for(AcceptanceField::Counterparty.kind()),
                "{op:?}"
            );
        }
        // Enum equality / membership are valid.
        assert!(RouteOp::Eq.valid_for(AcceptanceField::Side.kind()));
        assert!(RouteOp::In.valid_for(AcceptanceField::Counterparty.kind()));
    }

    #[test]
    fn string_field_supports_substring() {
        assert!(RouteOp::Contains.valid_for(AcceptanceField::InstrumentSymbol.kind()));
        assert!(RouteOp::Eq.valid_for(AcceptanceField::InstrumentSymbol.kind()));
        assert!(!RouteOp::Gt.valid_for(AcceptanceField::InstrumentSymbol.kind()));
    }
}
