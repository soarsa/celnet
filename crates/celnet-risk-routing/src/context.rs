//! The rule-evaluation input: a flat snapshot of one accepted fill's routable
//! field values.
//!
//! [`RoutingContext`] is deliberately decoupled from the server's booked-position
//! type — the server maps a `BookedPosition` + its attribution + the originating
//! order into this struct at the booking seam, and this crate stays pure. Each
//! field mirrors exactly one [`RouteField`]; [`RoutingContext::get`] is the single
//! bridge that projects a field selector onto its value.

use crate::field::{CtxValue, RouteField};
use serde::{Deserialize, Serialize};

/// One accepted fill's values for every routable field. The input to a routing
/// graph walk.
///
/// Numeric fields carry their natural units (`notional` a base-currency amount,
/// `tenor` in years, `strike`/`price` in price space). `notional` is expected to
/// be the **absolute** base notional (the server passes `|notional_base|`) so a
/// rule like `notional > 50_000_000` is side-agnostic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingContext {
    /// Instrument identifier / symbol.
    pub instrument_id: String,
    /// Currency or pair.
    pub ccy: String,
    /// Product family (vanilla / swap / bond / …).
    pub product: String,
    /// Trade side (e.g. `Buy` / `Sell`).
    pub side: String,
    /// Absolute base notional of the fill.
    pub notional: f64,
    /// Tenor / expiry in years.
    pub tenor: f64,
    /// Strike / resolved level.
    pub strike: f64,
    /// Counterparty / originating FIX session.
    pub counterparty: String,
    /// Booking user.
    pub user: String,
    /// Owning desk.
    pub desk: String,
    /// Fill price / premium.
    pub price: f64,
}

impl RoutingContext {
    /// Project this context onto one [`RouteField`], yielding the [`CtxValue`]
    /// handed to [`crate::RouteOp::eval`]. Numeric fields yield
    /// [`CtxValue::Number`]; string/enum fields yield [`CtxValue::Text`].
    ///
    /// The `Text` arm clones a small field string; booking is off the pinned
    /// zero-alloc pricing core, and a graph walk touches only `O(depth)` fields.
    pub fn get(&self, field: RouteField) -> CtxValue {
        match field {
            RouteField::InstrumentId => CtxValue::Text(self.instrument_id.clone()),
            RouteField::Ccy => CtxValue::Text(self.ccy.clone()),
            RouteField::Product => CtxValue::Text(self.product.clone()),
            RouteField::Side => CtxValue::Text(self.side.clone()),
            RouteField::Counterparty => CtxValue::Text(self.counterparty.clone()),
            RouteField::User => CtxValue::Text(self.user.clone()),
            RouteField::Desk => CtxValue::Text(self.desk.clone()),
            RouteField::Notional => CtxValue::Number(self.notional),
            RouteField::Tenor => CtxValue::Number(self.tenor),
            RouteField::Strike => CtxValue::Number(self.strike),
            RouteField::Price => CtxValue::Number(self.price),
        }
    }
}

impl Default for RoutingContext {
    fn default() -> Self {
        Self {
            instrument_id: String::new(),
            ccy: String::new(),
            product: String::new(),
            side: String::new(),
            notional: 0.0,
            tenor: 0.0,
            strike: 0.0,
            counterparty: String::new(),
            user: String::new(),
            desk: String::new(),
            price: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RoutingContext {
        RoutingContext {
            instrument_id: "EURUSD-1Y".into(),
            ccy: "EUR".into(),
            product: "swap".into(),
            side: "Buy".into(),
            notional: 60_000_000.0,
            tenor: 10.0,
            strike: 1.1,
            counterparty: "HF-1".into(),
            user: "trader.a".into(),
            desk: "RATES".into(),
            price: 0.25,
        }
    }

    #[test]
    fn get_projects_string_fields() {
        let c = sample();
        assert_eq!(c.get(RouteField::Ccy), CtxValue::Text("EUR".into()));
        assert_eq!(
            c.get(RouteField::InstrumentId),
            CtxValue::Text("EURUSD-1Y".into())
        );
        assert_eq!(c.get(RouteField::Side), CtxValue::Text("Buy".into()));
        assert_eq!(
            c.get(RouteField::Counterparty),
            CtxValue::Text("HF-1".into())
        );
        assert_eq!(c.get(RouteField::Desk), CtxValue::Text("RATES".into()));
    }

    #[test]
    fn get_projects_numeric_fields() {
        let c = sample();
        assert_eq!(c.get(RouteField::Notional), CtxValue::Number(60_000_000.0));
        assert_eq!(c.get(RouteField::Tenor), CtxValue::Number(10.0));
        assert_eq!(c.get(RouteField::Strike), CtxValue::Number(1.1));
        assert_eq!(c.get(RouteField::Price), CtxValue::Number(0.25));
    }

    #[test]
    fn default_is_all_empty_zero() {
        let d = RoutingContext::default();
        assert_eq!(d.get(RouteField::Ccy), CtxValue::Text(String::new()));
        assert_eq!(d.get(RouteField::Notional), CtxValue::Number(0.0));
    }
}
