//! The rule-evaluation input: a flat snapshot of one `(book × instrument)` risk
//! state.
//!
//! [`HedgeContext`] is deliberately decoupled from the server's position stores —
//! the server builds it off-core on each `risk_version` bump from the position /
//! rates stores + aggregation + analytics, and this crate stays pure. Each field
//! mirrors exactly one [`HedgeField`]; [`HedgeContext::get`] is the single bridge
//! that projects a field selector onto its value.

use crate::field::HedgeField;
use celnet_risk_routing::CtxValue;
use serde::{Deserialize, Serialize};

/// One `(book × instrument)` risk state's values for every routable field. The
/// input to a hedge-policy graph walk
/// (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §5.2).
///
/// Signed risk (`net_dv01`, `net_notional`, …) keeps its sign so a rule may branch
/// on direction; the band/budget fields (`threshold`, `utilization`, `overflow`,
/// `breached`) are pre-computed by the [`crate::WarehouseThreshold`] so the graph
/// can branch on the band without re-deriving it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HedgeContext {
    // ---- identity -----------------------------------------------------------
    /// Instrument identifier / symbol.
    pub instrument_id: String,
    /// The **executable security** this risk cell can be hedged in, when the cell resolves
    /// to one — the canonical `instrument_id` shared with the reference registry and the LP
    /// feed (the same id that names an aggregated-book instrument).
    ///
    /// DISTINCT from [`Self::instrument_id`], which for a rates cell is the product FAMILY
    /// ("BOND"/"OIS"/…) that warehouse thresholds and provenance are scoped by. A family
    /// label is not tradeable: asking the LP panel for a security called "BOND" can only
    /// miss, which silently backstops every shed to the synthetic composite and leaves the
    /// street-side league table with nothing to attribute. The executor therefore prices the
    /// LP lookup off THIS id when present.
    ///
    /// `None` when the cell has no security-master identity (an OIS/IRS/FRA cell, or a bond
    /// whose id never resolved against refdata) — honestly absent, never fabricated, and the
    /// executor falls back to the family label exactly as before.
    #[serde(default)]
    pub execution_instrument_id: Option<String>,
    /// Currency or pair.
    pub ccy: String,
    /// Product family (vanilla / swap / bond / …).
    pub product: String,
    /// Owning risk book / portfolio.
    pub book: String,
    /// Owning desk.
    pub desk: String,
    /// Originating counterparty of the fill that triggered this evaluation — the party
    /// id / name the Deal/blotter carries. A property of the *incoming flow* (not a
    /// per-counterparty net position), so a rule `counterparty == "X"` back-to-backs a
    /// given client's flow while the rest warehouses.
    pub counterparty: String,

    // ---- risk state ---------------------------------------------------------
    /// Signed net DV01 (FI budget metric).
    pub net_dv01: f64,
    /// Signed net notional / base-currency delta (FX budget metric).
    pub net_notional: f64,
    /// Signed net vega.
    pub net_vega: f64,
    /// Signed net gamma.
    pub net_gamma: f64,
    /// Inventory sign: `+1` long, `−1` short, `0` flat.
    pub inventory_sign: f64,

    // ---- budget state -------------------------------------------------------
    /// The resolved warehouse threshold (the "100") for this scope.
    pub threshold: f64,
    /// `|net_risk| / threshold` — the RAG utilisation ratio.
    pub utilization: f64,
    /// `max(0, |net_risk| − target)` — overflow beyond the band edge.
    pub overflow: f64,
    /// Whether the risk is at/over the red band (the hedge trigger).
    pub breached: bool,

    // ---- flow quality -------------------------------------------------------
    /// Markout / residual-toxicity of the flow that built this inventory.
    pub counterparty_toxicity: f64,
    /// How long this risk has sat, in seconds.
    pub inventory_age_secs: f64,

    // ---- market state -------------------------------------------------------
    /// Opposing internal flow the aggregator could cross now.
    pub internal_offset_available: f64,
    /// Current external hedge-cost estimate (spread + impact), in bp.
    pub hedge_cost_bp: f64,
}

impl HedgeContext {
    /// Project this context onto one [`HedgeField`], yielding the [`CtxValue`]
    /// handed to `RouteOp::eval`. Numeric fields yield [`CtxValue::Number`];
    /// string/enum fields yield [`CtxValue::Text`]. `Breached` renders as the enum
    /// text `"true"`/`"false"` so `breached == false` compares by string equality.
    ///
    /// The `Text` arm clones a small field string; evaluation is off the pinned
    /// zero-alloc pricing core, and a graph walk touches only `O(depth)` fields.
    #[must_use]
    pub fn get(&self, field: HedgeField) -> CtxValue {
        match field {
            HedgeField::InstrumentId => CtxValue::Text(self.instrument_id.clone()),
            HedgeField::Ccy => CtxValue::Text(self.ccy.clone()),
            HedgeField::Product => CtxValue::Text(self.product.clone()),
            HedgeField::Book => CtxValue::Text(self.book.clone()),
            HedgeField::Desk => CtxValue::Text(self.desk.clone()),
            HedgeField::Counterparty => CtxValue::Text(self.counterparty.clone()),
            HedgeField::Breached => {
                CtxValue::Text(if self.breached { "true" } else { "false" }.to_string())
            }
            HedgeField::NetDv01 => CtxValue::Number(self.net_dv01),
            HedgeField::NetNotional => CtxValue::Number(self.net_notional),
            HedgeField::NetVega => CtxValue::Number(self.net_vega),
            HedgeField::NetGamma => CtxValue::Number(self.net_gamma),
            HedgeField::InventorySign => CtxValue::Number(self.inventory_sign),
            HedgeField::Threshold => CtxValue::Number(self.threshold),
            HedgeField::Utilization => CtxValue::Number(self.utilization),
            HedgeField::Overflow => CtxValue::Number(self.overflow),
            HedgeField::CounterpartyToxicity => CtxValue::Number(self.counterparty_toxicity),
            HedgeField::InventoryAgeSecs => CtxValue::Number(self.inventory_age_secs),
            HedgeField::InternalOffsetAvailable => CtxValue::Number(self.internal_offset_available),
            HedgeField::HedgeCostBp => CtxValue::Number(self.hedge_cost_bp),
        }
    }
}

impl Default for HedgeContext {
    fn default() -> Self {
        Self {
            instrument_id: String::new(),
            execution_instrument_id: None,
            ccy: String::new(),
            product: String::new(),
            book: String::new(),
            desk: String::new(),
            counterparty: String::new(),
            net_dv01: 0.0,
            net_notional: 0.0,
            net_vega: 0.0,
            net_gamma: 0.0,
            inventory_sign: 0.0,
            threshold: 0.0,
            utilization: 0.0,
            overflow: 0.0,
            breached: false,
            counterparty_toxicity: 0.0,
            inventory_age_secs: 0.0,
            internal_offset_available: 0.0,
            hedge_cost_bp: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HedgeContext {
        HedgeContext {
            instrument_id: "EURUSD".into(),
            execution_instrument_id: None,
            ccy: "EUR".into(),
            product: "swap".into(),
            book: "RATES-EUR".into(),
            desk: "RATES".into(),
            counterparty: "CITADEL".into(),
            net_dv01: 125_000.0,
            net_notional: 60_000_000.0,
            net_vega: -12_000.0,
            net_gamma: 900.0,
            inventory_sign: 1.0,
            threshold: 100_000.0,
            utilization: 1.25,
            overflow: 45_000.0,
            breached: true,
            counterparty_toxicity: 0.7,
            inventory_age_secs: 320.0,
            internal_offset_available: 20_000.0,
            hedge_cost_bp: 1.5,
        }
    }

    #[test]
    fn get_projects_string_and_enum_fields() {
        let c = sample();
        assert_eq!(c.get(HedgeField::Ccy), CtxValue::Text("EUR".into()));
        assert_eq!(c.get(HedgeField::Book), CtxValue::Text("RATES-EUR".into()));
        assert_eq!(
            c.get(HedgeField::InstrumentId),
            CtxValue::Text("EURUSD".into())
        );
        assert_eq!(
            c.get(HedgeField::Counterparty),
            CtxValue::Text("CITADEL".into())
        );
    }

    #[test]
    fn breached_projects_to_enum_text() {
        let c = sample();
        assert_eq!(c.get(HedgeField::Breached), CtxValue::Text("true".into()));
        let flat = HedgeContext {
            breached: false,
            ..HedgeContext::default()
        };
        assert_eq!(
            flat.get(HedgeField::Breached),
            CtxValue::Text("false".into())
        );
    }

    #[test]
    fn get_projects_numeric_fields() {
        let c = sample();
        assert_eq!(c.get(HedgeField::NetDv01), CtxValue::Number(125_000.0));
        assert_eq!(c.get(HedgeField::Overflow), CtxValue::Number(45_000.0));
        assert_eq!(c.get(HedgeField::Utilization), CtxValue::Number(1.25));
        assert_eq!(
            c.get(HedgeField::CounterpartyToxicity),
            CtxValue::Number(0.7)
        );
    }

    #[test]
    fn default_is_all_empty_zero_flat() {
        let d = HedgeContext::default();
        assert_eq!(d.get(HedgeField::Ccy), CtxValue::Text(String::new()));
        assert_eq!(d.get(HedgeField::NetDv01), CtxValue::Number(0.0));
        assert_eq!(d.get(HedgeField::Breached), CtxValue::Text("false".into()));
    }
}
