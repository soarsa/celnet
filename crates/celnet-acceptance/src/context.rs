//! The rule-evaluation input: a flat snapshot of one inbound counterparty lift at
//! the acceptance point.
//!
//! [`AcceptanceContext`] is deliberately decoupled from the server's FIX / desk
//! types — the server builds it off-core on the async FIX edge from the lift + the
//! desk request + the engine mid, and this crate stays pure. Each field mirrors
//! exactly one [`AcceptanceField`]; [`AcceptanceContext::get`] is the single bridge
//! that projects a field selector onto its value.

use crate::field::AcceptanceField;
use celnet_risk_routing::CtxValue;
use serde::{Deserialize, Serialize};

/// One inbound lift's values for every matchable field. The input to an
/// acceptance-graph walk.
///
/// `edge_bps` keeps its sign so a rule may branch on profitability; `notional_usd`
/// and `tenor_years` are magnitudes; `quote_age_ms` is the freshness of the quote
/// being lifted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceContext {
    /// Counterparty / originating party id.
    pub counterparty: String,
    /// Absolute USD notional of the lift.
    pub notional_usd: f64,
    /// Tenor / years to maturity of the lifted instrument.
    pub tenor_years: f64,
    /// Instrument identifier / curve / security symbol.
    pub instrument: String,
    /// Lift side — pay/receive/buy/sell (rendered as text, e.g. `"Buy"` / `"Sell"`).
    pub side: String,
    /// The dealer edge of the lift versus the engine mid, in basis points (signed).
    pub edge_bps: f64,
    /// Age of the quote at lift time, in milliseconds.
    pub quote_age_ms: f64,
    /// Owning asset class — `"fx_options"` / `"fixed_income"`.
    pub asset: String,
    /// Owning / target desk.
    pub desk: String,
}

impl AcceptanceContext {
    /// Project this context onto one [`AcceptanceField`], yielding the [`CtxValue`]
    /// handed to `RouteOp::eval`. Numeric fields yield [`CtxValue::Number`];
    /// string/enum fields yield [`CtxValue::Text`].
    ///
    /// The `Text` arm clones a small field string; evaluation is off the pinned
    /// zero-alloc pricing core (it runs on the async FIX edge), and a graph walk
    /// touches only `O(depth)` fields.
    #[must_use]
    pub fn get(&self, field: AcceptanceField) -> CtxValue {
        match field {
            AcceptanceField::Counterparty => CtxValue::Text(self.counterparty.clone()),
            AcceptanceField::NotionalUsd => CtxValue::Number(self.notional_usd),
            AcceptanceField::TenorYears => CtxValue::Number(self.tenor_years),
            AcceptanceField::InstrumentSymbol => CtxValue::Text(self.instrument.clone()),
            AcceptanceField::Side => CtxValue::Text(self.side.clone()),
            AcceptanceField::EdgeBps => CtxValue::Number(self.edge_bps),
            AcceptanceField::QuoteAgeMs => CtxValue::Number(self.quote_age_ms),
            AcceptanceField::AssetClass => CtxValue::Text(self.asset.clone()),
            AcceptanceField::Desk => CtxValue::Text(self.desk.clone()),
        }
    }
}

impl Default for AcceptanceContext {
    fn default() -> Self {
        Self {
            counterparty: String::new(),
            notional_usd: 0.0,
            tenor_years: 0.0,
            instrument: String::new(),
            side: String::new(),
            edge_bps: 0.0,
            quote_age_ms: 0.0,
            asset: String::new(),
            desk: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AcceptanceContext {
        AcceptanceContext {
            counterparty: "HF-2".into(),
            notional_usd: 60_000_000.0,
            tenor_years: 5.0,
            instrument: "OIS".into(),
            side: "Buy".into(),
            edge_bps: 1.5,
            quote_age_ms: 320.0,
            asset: "fixed_income".into(),
            desk: "RATES".into(),
        }
    }

    #[test]
    fn get_projects_string_and_enum_fields() {
        let c = sample();
        assert_eq!(
            c.get(AcceptanceField::Counterparty),
            CtxValue::Text("HF-2".into())
        );
        assert_eq!(c.get(AcceptanceField::Side), CtxValue::Text("Buy".into()));
        assert_eq!(
            c.get(AcceptanceField::InstrumentSymbol),
            CtxValue::Text("OIS".into())
        );
        assert_eq!(
            c.get(AcceptanceField::AssetClass),
            CtxValue::Text("fixed_income".into())
        );
    }

    #[test]
    fn get_projects_numeric_fields() {
        let c = sample();
        assert_eq!(
            c.get(AcceptanceField::NotionalUsd),
            CtxValue::Number(60_000_000.0)
        );
        assert_eq!(c.get(AcceptanceField::TenorYears), CtxValue::Number(5.0));
        assert_eq!(c.get(AcceptanceField::EdgeBps), CtxValue::Number(1.5));
        assert_eq!(c.get(AcceptanceField::QuoteAgeMs), CtxValue::Number(320.0));
    }

    #[test]
    fn default_is_all_empty_zero() {
        let d = AcceptanceContext::default();
        assert_eq!(
            d.get(AcceptanceField::Counterparty),
            CtxValue::Text(String::new())
        );
        assert_eq!(d.get(AcceptanceField::NotionalUsd), CtxValue::Number(0.0));
        assert_eq!(d.get(AcceptanceField::EdgeBps), CtxValue::Number(0.0));
    }
}
