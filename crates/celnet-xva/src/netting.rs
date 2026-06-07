//! Synthetic netting set of vanilla FX options.
//!
//! A netting set is a portfolio of trades against a single counterparty that net
//! under one ISDA master agreement: at default, exposure is the *net* mark of the
//! whole set, not the sum of positive marks. We model trades as European vanilla
//! FX options priced by [`celnet_vanilla`].
//!
//! Synthetic only — no live CSA/collateral terms attach to the set (see the crate
//! docs' honest boundary).

use celnet_types::{OptionType, VanillaInputs};

/// One vanilla FX option in the netting set.
///
/// The mark at a future exposure date is the residual-maturity Garman-Kohlhagen
/// price scaled by `notional` (signed: positive = long the option from our side,
/// negative = short — a written option whose mark is a liability to us).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NettedTrade {
    /// Call or put.
    pub option: OptionType,
    /// Strike (quote per 1 unit of base).
    pub strike: f64,
    /// Option expiry in years from valuation (`t = 0`).
    pub expiry: f64,
    /// Flat lognormal vol used for the trade's mark.
    pub vol: f64,
    /// Signed notional in base units (`+` long / `−` short).
    pub notional: f64,
}

impl NettedTrade {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(
        option: OptionType,
        strike: f64,
        expiry: f64,
        vol: f64,
        notional: f64,
    ) -> Self {
        Self {
            option,
            strike,
            expiry,
            vol,
            notional,
        }
    }

    /// Signed mark-to-market of this trade at exposure time `t_obs` with spot
    /// `spot`, under domestic/foreign rates `r_dom`/`r_for`.
    ///
    /// Past expiry (`t_obs ≥ expiry`) the option has matured and contributes the
    /// (discounted-to-now-is-not-applied-here) **payoff is settled** ⇒ mark `0`
    /// to the *live* exposure (a settled cashflow leaves the netting set's live
    /// MtM). The residual maturity is `expiry − t_obs`.
    #[must_use]
    pub fn mark(&self, t_obs: f64, spot: f64, r_dom: f64, r_for: f64) -> f64 {
        let tau = self.expiry - t_obs;
        if tau <= 0.0 {
            return 0.0;
        }
        let inputs = VanillaInputs::new(spot, self.strike, self.vol, tau, r_dom, r_for);
        self.notional * celnet_vanilla::price(self.option, &inputs)
    }
}

/// A netting set: a collection of [`NettedTrade`]s plus the shared rate
/// environment used to mark them.
#[derive(Debug, Clone, PartialEq)]
pub struct NettingSet {
    trades: Vec<NettedTrade>,
    r_dom: f64,
    r_for: f64,
}

impl NettingSet {
    /// Build a netting set from its trades and the domestic/foreign rates.
    #[must_use]
    pub fn new(trades: Vec<NettedTrade>, r_dom: f64, r_for: f64) -> Self {
        Self {
            trades,
            r_dom,
            r_for,
        }
    }

    /// The trades in the set.
    #[must_use]
    pub fn trades(&self) -> &[NettedTrade] {
        &self.trades
    }

    /// Domestic (quote) rate.
    #[must_use]
    pub fn r_dom(&self) -> f64 {
        self.r_dom
    }

    /// Foreign (base) rate.
    #[must_use]
    pub fn r_for(&self) -> f64 {
        self.r_for
    }

    /// The **net** mark-to-market of the whole set at exposure time `t_obs` and
    /// spot `spot`. This is the value that nets at default — the sum of the signed
    /// trade marks (not the sum of positive marks).
    #[must_use]
    pub fn net_value(&self, t_obs: f64, spot: f64) -> f64 {
        self.trades
            .iter()
            .map(|tr| tr.mark(t_obs, spot, self.r_dom, self.r_for))
            .sum()
    }

    /// The longest trade expiry — the horizon over which exposure can be non-zero.
    #[must_use]
    pub fn horizon(&self) -> f64 {
        self.trades.iter().map(|t| t.expiry).fold(0.0_f64, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matured_trade_marks_zero() {
        let tr = NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.10, 1.0);
        assert_eq!(tr.mark(1.0, 1.20, 0.02, 0.01), 0.0);
        assert_eq!(tr.mark(1.5, 1.20, 0.02, 0.01), 0.0);
    }

    #[test]
    fn net_value_sums_signed_marks() {
        // Long a call, short a put — net is the difference of the two marks.
        let set = NettingSet::new(
            vec![
                NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.10, 1.0),
                NettedTrade::new(OptionType::Put, 1.10, 1.0, 0.10, -1.0),
            ],
            0.02,
            0.01,
        );
        let s = 1.15;
        let call =
            NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.10, 1.0).mark(0.0, s, 0.02, 0.01);
        let put = NettedTrade::new(OptionType::Put, 1.10, 1.0, 0.10, -1.0).mark(0.0, s, 0.02, 0.01);
        assert!((set.net_value(0.0, s) - (call + put)).abs() < 1e-14);
    }

    #[test]
    fn horizon_is_longest_expiry() {
        let set = NettingSet::new(
            vec![
                NettedTrade::new(OptionType::Call, 1.0, 0.5, 0.1, 1.0),
                NettedTrade::new(OptionType::Put, 1.0, 2.0, 0.1, 1.0),
            ],
            0.0,
            0.0,
        );
        assert_eq!(set.horizon(), 2.0);
    }
}
