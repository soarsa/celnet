//! The `perpetual` subcommand core — the perpetual (no-expiry) American vanilla
//! on the command line.
//!
//! This prices the dedicated `celnet-exotics` perpetual leaf locally, exactly as
//! the server's `pricer` routes proto arm 30 — the same stationary-ODE closed
//! form (free boundary by value matching + smooth pasting) and the same carry
//! branching: an FX underlying (`--asset fx`, the default) prices over the FX
//! two-rate carry, a cross-asset class over the generalized cost-of-carry seam
//! with `b = r_dom − r_for` (where `--r-for` is the asset's carry yield —
//! ADR-0008, identical to the `price` command's cross-asset semantics). The CLI
//! is a local-compute client, so it reaches an identical price to the SDK-via-
//! server path without a running edge — gated CLI == server == golden corpus by
//! `tests/conformance.rs`.
//!
//! A perpetual has **no expiry**: the command takes no `--t`/tenor (the wire
//! contract encodes the no-expiry shape as `expiry_years = 0`), the value is
//! time-homogeneous, and theta is identically zero — reported as the exact zero
//! it is, never a computed decay.

use celnet_exotics::{
    PerpetualError, PerpetualInputs, perpetual_exercise_boundary, perpetual_greeks,
};
use celnet_types::{Carry, OptionType, RateSensitivities};

use crate::args::CliAsset;

/// The market state a perpetual prices against: spot, flat vol, and the two
/// rates. No `t` — a perpetual is the one expiryless product.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PerpetualMarket {
    /// Spot price (quote per 1 unit of base/asset).
    pub(crate) spot: f64,
    /// Annualized volatility (absolute, e.g. 0.10 = 10 vol).
    pub(crate) vol: f64,
    /// Continuously-compounded domestic / discount rate.
    pub(crate) r_dom: f64,
    /// Continuously-compounded carry yield (FX foreign rate / equity dividend
    /// yield / commodity cost-of-carry / crypto funding).
    pub(crate) r_for: f64,
}

impl PerpetualMarket {
    /// The carry model for the chosen asset class: FX prices over the two-rate
    /// arm; every other class over the generalized cost-of-carry arm with
    /// `r = r_dom`, `b = r_dom − r_for` — the same two-way bijection the server
    /// applies, so the value is identical and only the rho tagging differs.
    fn carry(self, asset: CliAsset) -> Carry {
        match asset {
            CliAsset::Fx => Carry::FxRates {
                r_dom: self.r_dom,
                r_for: self.r_for,
            },
            CliAsset::Equity | CliAsset::Commodity | CliAsset::Crypto => Carry::CostOfCarry {
                r: self.r_dom,
                b: self.r_dom - self.r_for,
            },
        }
    }
}

/// The priced result of a perpetual: the free early-exercise boundary plus the
/// exact analytic strip (price, delta, gamma, vega, carry-tagged rhos). No
/// standard error: the closed form is exact, not Monte-Carlo.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PerpetualResult {
    /// The free early-exercise boundary (`S*` call / `S**` put; `inf` for the
    /// never-exercised call arm, `0` for the collapsed put arm).
    pub(crate) boundary: f64,
    /// The exact analytic Greek strip from the perpetual leaf.
    pub(crate) greeks: celnet_exotics::PerpetualGreeks,
}

/// Price a perpetual American vanilla and its exact analytic Greek strip.
/// Domain validation (positive spot/strike/vol, non-negative discount rate)
/// happens in the dispatch before this is called; the leaf's own carry-domain
/// law is surfaced here as the typed [`PerpetualError`] — a CALL with carry
/// strictly exceeding the discount rate (`b > r`; FX form: `r_for < 0`) has
/// NO finite value and is refused, exactly as the server refuses it.
pub(crate) fn run(
    option: OptionType,
    strike: f64,
    market: PerpetualMarket,
    asset: CliAsset,
) -> Result<PerpetualResult, PerpetualError> {
    let inputs = PerpetualInputs::new(market.spot, strike, market.vol, market.carry(asset));
    Ok(PerpetualResult {
        boundary: perpetual_exercise_boundary(option, &inputs)?,
        greeks: perpetual_greeks(option, &inputs)?,
    })
}

/// Format a perpetual report (the `price` line keyed exactly like every other
/// CLI report so the conformance harness parses it identically; values print
/// shortest-round-trip so a parse recovers the exact `f64`). The rho lines
/// follow the carry tagging: the FX arm reports `rho_dom`/`rho_for`, the
/// generalized arm `discount_rho`/`carry_rho`. Theta prints the exact zero of
/// the time-homogeneous value — never a computed decay.
#[must_use]
pub(crate) fn format_report(
    option: OptionType,
    asset: CliAsset,
    underlying: &str,
    strike: f64,
    r: &PerpetualResult,
) -> String {
    let side = match option {
        OptionType::Call => "call",
        OptionType::Put => "put",
    };
    let g = &r.greeks;
    let mut out = String::new();
    out.push_str(&format!("perpetual {side}\n"));
    out.push_str(&format!("  asset             {}\n", asset.label()));
    out.push_str(&format!("  underlying        {underlying}\n"));
    out.push_str(&format!("  strike            {strike}\n"));
    out.push_str(&format!("  exercise_boundary {}\n", r.boundary));
    out.push_str(&format!("  price             {}\n", g.price));
    out.push_str(&format!("  delta_spot        {}\n", g.delta));
    out.push_str(&format!("  gamma             {}\n", g.gamma));
    out.push_str(&format!("  vega              {}\n", g.vega));
    match g.rates {
        RateSensitivities::Fx { rho_dom, rho_for } => {
            out.push_str(&format!("  rho_dom           {rho_dom}\n"));
            out.push_str(&format!("  rho_for           {rho_for}\n"));
        }
        RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        } => {
            out.push_str(&format!("  discount_rho      {discount_rho}\n"));
            out.push_str(&format!("  carry_rho         {carry_rho}\n"));
        }
    }
    out.push_str("  theta             0  (exact: the perpetual value is time-homogeneous)\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    /// The CLI core reproduces the perpetual leaf exactly on both carry arms —
    /// the FX arm bit-for-bit against a directly-constructed two-rate input,
    /// and the cross-asset arm against the `b = r_dom − r_for` bijection.
    #[test]
    fn run_matches_the_leaf_on_both_carry_arms() {
        let market = PerpetualMarket {
            spot: 1.10,
            vol: 0.105,
            r_dom: 0.05,
            r_for: 0.01,
        };
        for opt in [OptionType::Call, OptionType::Put] {
            let r = run(opt, 1.1, market, CliAsset::Fx).unwrap();
            let direct = perpetual_greeks(
                opt,
                &PerpetualInputs::new(
                    1.10,
                    1.1,
                    0.105,
                    Carry::FxRates {
                        r_dom: 0.05,
                        r_for: 0.01,
                    },
                ),
            )
            .unwrap();
            assert_eq!(r.greeks.price.to_bits(), direct.price.to_bits());
            assert_eq!(r.greeks.delta.to_bits(), direct.delta.to_bits());
            assert!(matches!(r.greeks.rates, RateSensitivities::Fx { .. }));

            // The generalized arm prices the SAME value (the carry bijection is
            // lossless) and tags the rhos as discount/carry.
            let x = run(opt, 1.1, market, CliAsset::Equity).unwrap();
            assert!(is_close(x.greeks.price, direct.price, 1e-15, 1e-15));
            assert!(matches!(x.greeks.rates, RateSensitivities::Carry { .. }));
        }
    }

    /// The leaf's carry-domain law surfaces typed through `run`: an FX CALL
    /// with `r_for < 0` (`b = r_dom − r_for > r_dom`) has no finite value and
    /// is refused on BOTH carry arms; the PUT on the same market still prices.
    #[test]
    fn run_refuses_a_call_with_carry_exceeding_discount() {
        let market = PerpetualMarket {
            spot: 1.25,
            vol: 0.10,
            r_dom: 0.02,
            r_for: -0.005, // b = 0.025 > r = 0.02
        };
        for asset in [CliAsset::Fx, CliAsset::Equity] {
            assert_eq!(
                run(OptionType::Call, 1.10, market, asset).unwrap_err(),
                PerpetualError::CallCarryExceedsDiscount
            );
            let put = run(OptionType::Put, 1.10, market, asset).unwrap();
            assert!(put.greeks.price.is_finite() && put.greeks.price >= 0.0);
        }
    }

    /// The report parses back the exact price (shortest-round-trip printing)
    /// and prints the time-homogeneous theta as the exact zero.
    #[test]
    fn report_round_trips_the_price() {
        let market = PerpetualMarket {
            spot: 1.10,
            vol: 0.105,
            r_dom: 0.03,
            r_for: 0.015,
        };
        let r = run(OptionType::Put, 1.15, market, CliAsset::Fx).unwrap();
        let report = format_report(OptionType::Put, CliAsset::Fx, "EURUSD", 1.15, &r);
        let printed: f64 = report
            .lines()
            .find_map(|l| l.trim().strip_prefix("price"))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|tok| tok.parse().ok())
            .expect("price line parses");
        assert_eq!(printed.to_bits(), r.greeks.price.to_bits());
        assert!(report.contains("theta             0  (exact"));
        assert!(report.contains("rho_dom"));
    }
}
