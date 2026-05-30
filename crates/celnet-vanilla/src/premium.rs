//! Premium expressed in each [`PremiumStyle`].
//!
//! The Garman-Kohlhagen [`crate::price`] returns the present value in *domestic
//! pips* — domestic (quote) currency per one unit of base (foreign) notional.
//! That single canonical number is re-expressed in the four market quotation
//! styles here. The conversions follow the standard FX-options treatment
//! (Clark, *Foreign Exchange Option Pricing*, 2011, §2.3; Reiswich & Wystup,
//! "FX Volatility Smile Construction", 2010, §2): given spot `S`, strike `K`
//! and the domestic-pips premium `V_dpips`,
//!
//! ```text
//!   domestic pips (V_d)   = V_dpips                         [DOM per 1 FOR]
//!   % foreign  (V_%f)     = V_dpips / S                     [fraction of FOR notional]
//!   % domestic (V_%d)     = V_dpips / K                     [fraction of DOM notional]
//!   foreign pips (V_fpips)= V_dpips / (S · K)               [FOR per 1 DOM notional]
//! ```
//!
//! `% foreign` and `foreign pips` are the FOR-currency styles whose premium
//! carries FX risk, hence the premium-adjusted delta
//! ([`PremiumStyle::is_premium_adjusted`]).

use celnet_types::{OptionType, PremiumStyle, VanillaInputs};

use crate::price;

/// Present value re-expressed in the requested [`PremiumStyle`].
///
/// `price(opt, inputs)` is the canonical domestic-pips premium; this scales it
/// into the quotation style the market uses for the `(pair, tenor)`.
///
/// All four styles are pure functions of the same domestic-pips premium and the
/// `(spot, strike)`; see the module docs for the exact factors.
#[must_use]
pub fn premium(style: PremiumStyle, opt: OptionType, inputs: &VanillaInputs) -> f64 {
    premium_from_domestic_pips(style, price(opt, inputs), inputs.spot, inputs.strike)
}

/// Re-express an already-computed domestic-pips premium `v_dpips` in `style`.
///
/// Split out so callers that have priced once (e.g. a smile calibration loop)
/// do not re-enter the pricer; [`premium`] is the convenience wrapper.
#[must_use]
pub fn premium_from_domestic_pips(
    style: PremiumStyle,
    v_dpips: f64,
    spot: f64,
    strike: f64,
) -> f64 {
    match style {
        PremiumStyle::DomesticPips => v_dpips,
        PremiumStyle::PercentForeign => v_dpips / spot,
        PremiumStyle::PercentDomestic => v_dpips / strike,
        PremiumStyle::ForeignPips => v_dpips / (spot * strike),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn inputs() -> VanillaInputs {
        VanillaInputs::new(1.20, 1.25, 0.11, 0.75, 0.03, 0.01)
    }

    #[test]
    fn domestic_pips_is_the_raw_pv() {
        let i = inputs();
        let v = price(OptionType::Call, &i);
        assert_close!(premium(PremiumStyle::DomesticPips, OptionType::Call, &i), v);
    }

    #[test]
    fn style_factors_match_definitions() {
        let i = inputs();
        let v = price(OptionType::Call, &i);
        assert_close!(
            premium(PremiumStyle::PercentForeign, OptionType::Call, &i),
            v / i.spot
        );
        assert_close!(
            premium(PremiumStyle::PercentDomestic, OptionType::Call, &i),
            v / i.strike
        );
        assert_close!(
            premium(PremiumStyle::ForeignPips, OptionType::Call, &i),
            v / (i.spot * i.strike)
        );
    }

    #[test]
    fn from_domestic_pips_matches_full_path() {
        let i = inputs();
        let v = price(OptionType::Put, &i);
        for style in [
            PremiumStyle::DomesticPips,
            PremiumStyle::PercentForeign,
            PremiumStyle::PercentDomestic,
            PremiumStyle::ForeignPips,
        ] {
            assert_close!(
                premium(style, OptionType::Put, &i),
                premium_from_domestic_pips(style, v, i.spot, i.strike)
            );
        }
    }
}
