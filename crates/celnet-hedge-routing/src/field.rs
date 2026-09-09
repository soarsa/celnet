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
/// cost (`docs/hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §5.2).
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
    /// Originating counterparty of the fill being hedged — party id / name (string).
    ///
    /// A property of the *incoming flow*, not a per-counterparty net position: a rule
    /// `counterparty == "X"` matches when the fill that triggered this evaluation came
    /// from `X` (e.g. back-to-back all of one client's flow, warehouse the rest). Modelled
    /// as free-form [`FieldKind::String`] (like [`HedgeField::InstrumentId`]) so a rule may
    /// use `eq`/`ne`/`in`/`contains` — the `contains` operator lets a prefix such as
    /// `"CITADEL"` match a family of session ids.
    Counterparty,

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

/// Whether the production hedge-context builder actually computes a [`HedgeField`], and —
/// when it does not — why it cannot.
///
/// A rule condition is only meaningful if the field on its left-hand side carries a real
/// value at evaluation time. Several fields have **no production source**: the one production
/// [`HedgeContext`](crate::HedgeContext) builder (the server's rates booking seam) leaves them
/// at their `Default`, so a condition on such a field compares against a constant `0.0` /
/// `""` forever. The rule is not merely wrong — it is *dead*, and silently so: it validates,
/// it persists, it renders in the UI, and it never fires.
///
/// Declaring provenance here is what lets
/// [`HedgeGraph::validate`](crate::HedgeGraph::validate) refuse such a rule at authoring time
/// (`HedgeError::UnprovidedField`) rather than accept one that can never fire. The
/// declaration lives beside the field set — not in the server — because the *field vocabulary*
/// is this crate's charter, and because the check must run everywhere a graph is validated,
/// including at identity-store load.
///
/// Note that `Unprovided` is not the same as "the value is wrong". [`HedgeField::NetVega`] and
/// [`HedgeField::NetGamma`] are honestly `0.0` for the linear-rates cells that are the only
/// cells ever evaluated — but a rule reading them is still unconditionally dead, which is the
/// thing being eliminated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldProvider {
    /// The production builder computes this field.
    Computed {
        /// Where the value comes from — the source, in the builder's own terms.
        basis: &'static str,
    },
    /// No production source exists for this field.
    Unprovided {
        /// Why the field cannot be populated. Surfaced verbatim in the validation error so
        /// the rule author sees the cause, not merely a rejection.
        reason: &'static str,
    },
}

impl HedgeField {
    /// Every [`HedgeField`], in declaration order.
    ///
    /// Invariant: this array lists each variant exactly once. It is the enumeration a
    /// validator or field palette iterates. The load-bearing guard against a variant being
    /// forgotten is that [`HedgeField::kind`] and [`HedgeField::provider`] are **exhaustive
    /// matches with no wildcard arm** — a new variant cannot compile until both declare it —
    /// and the `all_lists_every_variant` test pins this array against a third exhaustive
    /// match.
    pub const ALL: [HedgeField; 19] = [
        HedgeField::InstrumentId,
        HedgeField::Ccy,
        HedgeField::Product,
        HedgeField::Book,
        HedgeField::Desk,
        HedgeField::Counterparty,
        HedgeField::NetDv01,
        HedgeField::NetNotional,
        HedgeField::NetVega,
        HedgeField::NetGamma,
        HedgeField::InventorySign,
        HedgeField::Threshold,
        HedgeField::Utilization,
        HedgeField::Overflow,
        HedgeField::Breached,
        HedgeField::CounterpartyToxicity,
        HedgeField::InventoryAgeSecs,
        HedgeField::InternalOffsetAvailable,
        HedgeField::HedgeCostBp,
    ];

    /// Whether the production hedge-context builder computes this field, and the basis or the
    /// reason it cannot — see [`FieldProvider`].
    ///
    /// **This match is deliberately exhaustive with no wildcard arm.** A new [`HedgeField`]
    /// therefore cannot be added without stating, at the point of addition, whether anything
    /// actually populates it. That is precisely the mistake this guards: five fields shipped
    /// with a full type, doc comment, operator matrix and UI chip, and no producer at all.
    #[must_use]
    pub const fn provider(self) -> FieldProvider {
        match self {
            HedgeField::InstrumentId => FieldProvider::Computed {
                basis: "the fill's product-family label",
            },
            HedgeField::Ccy => FieldProvider::Computed {
                basis: "the settlement currency on the fill's risk-routing attribution",
            },
            HedgeField::Product => FieldProvider::Computed {
                basis: "the fill's product-family label",
            },
            HedgeField::Book => FieldProvider::Computed {
                basis: "the risk book the fill routed into",
            },
            // The desk is a property of the SESSION that priced the quote, not of the
            // position that resulted. It is unrecoverable from a booked fill, which is why
            // `resolve_hedging_model` and `resolve_hedge_threshold` are both already called
            // with an empty desk operand on this path.
            HedgeField::Desk => FieldProvider::Unprovided {
                reason: "a booked rates fill carries no desk — the desk belongs to the \
                         FIX/RFQ session that priced it, not to the resulting position; \
                         scope the rule by `book` instead",
            },
            HedgeField::Counterparty => FieldProvider::Computed {
                basis: "the originating party id on the fill's risk-routing attribution",
            },
            HedgeField::NetDv01 => FieldProvider::Computed {
                basis: "the signed DV01 of the fill's book, or of the bucket subtree under a \
                        bucket-scoped policy",
            },
            HedgeField::NetNotional => FieldProvider::Computed {
                basis: "the signed face notional of the same scope",
            },
            // Honestly zero rather than wrong — but a rule on it is still dead.
            HedgeField::NetVega => FieldProvider::Unprovided {
                reason: "only linear-rates cells are ever evaluated and they carry no \
                         volatility risk, so this field is a constant zero and any rule \
                         reading it can never fire",
            },
            HedgeField::NetGamma => FieldProvider::Unprovided {
                reason: "only linear-rates cells are ever evaluated and they carry no \
                         convexity risk, so this field is a constant zero and any rule \
                         reading it can never fire",
            },
            HedgeField::InventorySign => FieldProvider::Computed {
                basis: "the three-way sign of the book's net risk, in the threshold's own \
                        budget metric",
            },
            HedgeField::Threshold => FieldProvider::Computed {
                basis: "the warehouse cap resolved for the fill's most-specific scope",
            },
            HedgeField::Utilization => FieldProvider::Computed {
                basis: "|net risk| / cap, in the threshold's metric",
            },
            HedgeField::Overflow => FieldProvider::Computed {
                basis: "max(0, |net risk| − target), in the threshold's metric",
            },
            HedgeField::Breached => FieldProvider::Computed {
                basis: "the warehouse band classification, in the threshold's metric",
            },
            HedgeField::CounterpartyToxicity => FieldProvider::Unprovided {
                reason: "no post-fill mark trajectory is retained, so markout cannot be \
                         derived and nothing computes a per-counterparty toxicity score",
            },
            HedgeField::InventoryAgeSecs => FieldProvider::Unprovided {
                reason: "a stored rates position carries no acquisition timestamp, so how \
                         long the risk has sat cannot be derived",
            },
            HedgeField::InternalOffsetAvailable => FieldProvider::Computed {
                basis: "the netted opposing risk held by sibling books under the same parent, \
                        within the same product family",
            },
            HedgeField::HedgeCostBp => FieldProvider::Computed {
                basis: "the live LP top-of-book crossing half-spread, falling back to the \
                        desk's configured composite spread",
            },
        }
    }

    /// The reason this field has no production source, or `None` when it is computed.
    /// The convenience form of [`HedgeField::provider`] used by validation.
    #[must_use]
    pub const fn unprovided_reason(self) -> Option<&'static str> {
        match self.provider() {
            FieldProvider::Computed { .. } => None,
            FieldProvider::Unprovided { reason } => Some(reason),
        }
    }

    /// The [`FieldKind`] of this field — the single source of truth for the
    /// operator/value type matrix used by `RouteOp::valid_for` and validation.
    ///
    /// `Breached` is an [`FieldKind::Enum`] (compared by `== "true"/"false"`);
    /// all other risk-state numbers are [`FieldKind::Numeric`]; `InstrumentId` and
    /// `Counterparty` are free-form [`FieldKind::String`].
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
            HedgeField::InstrumentId | HedgeField::Counterparty => FieldKind::String,
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
        assert_eq!(HedgeField::Counterparty.kind(), FieldKind::String);
    }

    #[test]
    fn counterparty_supports_string_operators() {
        // A string/identity field: eq / ne / in / contains are valid; ordering is not.
        let k = HedgeField::Counterparty.kind();
        assert!(RouteOp::Eq.valid_for(k));
        assert!(RouteOp::Ne.valid_for(k));
        assert!(RouteOp::In.valid_for(k));
        assert!(RouteOp::Contains.valid_for(k));
        assert!(!RouteOp::Gt.valid_for(k));
        assert!(!RouteOp::Between.valid_for(k));
    }

    #[test]
    fn breached_supports_enum_equality_only() {
        // The example `breached == false` uses Eq (valid for Enum); ordering is not.
        assert!(RouteOp::Eq.valid_for(HedgeField::Breached.kind()));
        assert!(!RouteOp::Gt.valid_for(HedgeField::Breached.kind()));
    }

    #[test]
    fn all_lists_every_variant() {
        // A compile-time pin: this match has NO wildcard arm, so adding a `HedgeField`
        // variant fails to compile here until it is given a position. Each arm yields the
        // variant's index in `HedgeField::ALL`; the round-trip below then requires that
        // index to be the position it actually occupies, so a reordering or a duplicate is
        // caught rather than passing silently.
        fn declared_index(f: HedgeField) -> usize {
            match f {
                HedgeField::InstrumentId => 0,
                HedgeField::Ccy => 1,
                HedgeField::Product => 2,
                HedgeField::Book => 3,
                HedgeField::Desk => 4,
                HedgeField::Counterparty => 5,
                HedgeField::NetDv01 => 6,
                HedgeField::NetNotional => 7,
                HedgeField::NetVega => 8,
                HedgeField::NetGamma => 9,
                HedgeField::InventorySign => 10,
                HedgeField::Threshold => 11,
                HedgeField::Utilization => 12,
                HedgeField::Overflow => 13,
                HedgeField::Breached => 14,
                HedgeField::CounterpartyToxicity => 15,
                HedgeField::InventoryAgeSecs => 16,
                HedgeField::InternalOffsetAvailable => 17,
                HedgeField::HedgeCostBp => 18,
            }
        }
        for (i, f) in HedgeField::ALL.iter().enumerate() {
            assert_eq!(
                declared_index(*f),
                i,
                "ALL[{i}] = {f:?} is not at its declared index"
            );
        }
        let unique: std::collections::HashSet<_> = HedgeField::ALL.iter().collect();
        assert_eq!(
            unique.len(),
            HedgeField::ALL.len(),
            "ALL contains a duplicate variant"
        );
    }

    #[test]
    fn every_field_declares_a_provider_with_a_non_empty_justification() {
        // Neither arm may be a rubber stamp: a `Computed` field must say what computes it,
        // and an `Unprovided` field must say why it cannot be. An empty string would let a
        // future field slip through the declaration without stating anything.
        for f in HedgeField::ALL {
            match f.provider() {
                FieldProvider::Computed { basis } => {
                    assert!(!basis.trim().is_empty(), "{f:?} declares an empty basis");
                    assert_eq!(f.unprovided_reason(), None, "{f:?} is computed");
                }
                FieldProvider::Unprovided { reason } => {
                    assert!(!reason.trim().is_empty(), "{f:?} declares an empty reason");
                    assert_eq!(
                        f.unprovided_reason(),
                        Some(reason),
                        "{f:?} reason must round-trip"
                    );
                }
            }
        }
    }

    #[test]
    fn the_five_fields_with_no_production_source_are_declared_unprovided() {
        // The exact set found to have no producer. This is a CHANGE-DETECTOR by design: if
        // a source is later wired (an acquisition timestamp added to the position record, a
        // markout series retained, an options cell that builds a context), this test fails
        // and forces the declaration — and the newly-live rules — to be revisited together.
        let expected = [
            HedgeField::Desk,
            HedgeField::NetVega,
            HedgeField::NetGamma,
            HedgeField::CounterpartyToxicity,
            HedgeField::InventoryAgeSecs,
        ];
        let actual: Vec<HedgeField> = HedgeField::ALL
            .into_iter()
            .filter(|f| f.unprovided_reason().is_some())
            .collect();
        assert_eq!(actual, expected);
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
