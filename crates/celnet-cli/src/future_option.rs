//! The `future-option` subcommand core — an option on a listed future on the
//! command line.
//!
//! This prices the `celnet-commodity-vanilla` leaf's `on_future` representation
//! locally, exactly as the server's `pricer` routes proto arm 31 — the quoted
//! futures price already embodies the underlying's carry (any asset class), so
//! the value is the futures-measure closed form under the chosen premium
//! [`CliMargining`] convention: equity-style (premium-upfront) is the discounted
//! expectation, futures-style (daily-margined) the undiscounted one, each its
//! own closed form (never a divide-by-discount-factor workaround). The CLI is a
//! local-compute client, so it reaches an identical price to the SDK-via-server
//! path without a running edge — gated CLI == server == golden corpus by
//! `tests/conformance.rs`.
//!
//! The future's contract identity (`--symbol`/`--venue`) and its own expiry
//! (`--future-expiry`, which must be ≥ the option's `--t` — the future outlives
//! the option) are booked terms: the identity labels the report and the expiry
//! ordering is validated, but neither enters the closed form (the quoted future
//! is the complete carry-bearing market input).

use celnet_commodity_vanilla::{CommodityInputs, Margining, greeks_with_margining};
use celnet_core::carry::CarryGreeks;
use celnet_types::{OptionType, RateSensitivities};

/// The premium margining convention on the command line — how the option's
/// premium settles against the exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub(crate) enum CliMargining {
    /// Equity-style (premium-upfront): the premium is paid at trade date, so the
    /// value carries the discount factor. The default (the wire meaningful-zero).
    #[default]
    EquityStyle,
    /// Futures-style (daily-margined): the premium is margined daily like the
    /// future, so the value is the undiscounted expectation (the daily sweep
    /// removes the financing leg; the discount rate never enters).
    FuturesStyle,
}

impl CliMargining {
    /// The user-facing token, for the report header.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            CliMargining::EquityStyle => "equity-style",
            CliMargining::FuturesStyle => "futures-style",
        }
    }
}

impl From<CliMargining> for Margining {
    fn from(v: CliMargining) -> Self {
        match v {
            CliMargining::EquityStyle => Margining::EquityStyle,
            CliMargining::FuturesStyle => Margining::FuturesStyle,
        }
    }
}

/// The market and terms a listed-future option prices against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FutureOptionMarket {
    /// The quoted futures price `F` (the complete carry-bearing market input).
    pub(crate) future: f64,
    /// Annualized volatility of the future (absolute, e.g. 0.28 = 28 vol).
    pub(crate) vol: f64,
    /// The OPTION's time to expiry in years.
    pub(crate) t: f64,
    /// Continuously-compounded discount (settlement-currency) rate.
    pub(crate) r_dom: f64,
}

/// Price an option on a listed future and its full Greek strip under the given
/// premium margining convention. Domain validation (positive future/strike/vol,
/// positive option expiry, future outliving the option) happens in the dispatch
/// before this is called.
pub(crate) fn run(
    option: OptionType,
    strike: f64,
    market: FutureOptionMarket,
    margining: CliMargining,
) -> CarryGreeks {
    let inputs =
        CommodityInputs::on_future(market.future, strike, market.vol, market.t, market.r_dom);
    greeks_with_margining(option, margining.into(), &inputs)
}

/// Format a listed-future option report (the `price` line keyed exactly like
/// every other CLI report so the conformance harness parses it identically;
/// values print shortest-round-trip so a parse recovers the exact `f64`). In
/// the on-future representation the spot IS the quoted future, so the single
/// reported `delta` is `∂V/∂F`. The rho pair is carry-tagged: under
/// futures-style margining the `discount_rho` is the exact zero of the
/// daily-margined premium (the financing leg is swept away), never a computed
/// residual.
#[must_use]
pub(crate) fn format_report(
    option: OptionType,
    symbol: &str,
    venue: &str,
    future_expiry: f64,
    margining: CliMargining,
    strike: f64,
    g: &CarryGreeks,
) -> String {
    let side = match option {
        OptionType::Call => "call",
        OptionType::Put => "put",
    };
    // The listed contract identity: `TICKER@MIC`, or the bare ticker when no
    // venue was supplied.
    let contract = if venue.is_empty() {
        symbol.to_owned()
    } else {
        format!("{symbol}@{venue}")
    };
    let (discount_rho, carry_rho) = match g.rates {
        RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        } => (discount_rho, carry_rho),
        // Structurally unreachable: the on-future leaf always tags as Carry.
        RateSensitivities::Fx { rho_dom, rho_for } => (rho_dom + rho_for, -rho_for),
    };
    let mut out = String::new();
    out.push_str(&format!("future-option {side}\n"));
    out.push_str(&format!("  contract        {contract}\n"));
    out.push_str(&format!("  future_expiry   {future_expiry}\n"));
    out.push_str(&format!("  margining       {}\n", margining.label()));
    out.push_str(&format!("  strike          {strike}\n"));
    out.push_str(&format!("  price           {}\n", g.price));
    out.push_str(&format!("  delta           {}\n", g.delta_forward));
    out.push_str(&format!("  gamma           {}\n", g.gamma));
    out.push_str(&format!("  vega            {}\n", g.vega));
    out.push_str(&format!("  theta           {}\n", g.theta));
    out.push_str(&format!("  discount_rho    {discount_rho}\n"));
    out.push_str(&format!("  carry_rho       {carry_rho}\n"));
    out.push_str(&format!("  vanna           {}\n", g.vanna));
    out.push_str(&format!("  volga           {}\n", g.volga));
    out.push_str(&format!("  charm           {}\n", g.charm));
    out.push_str(&format!("  speed           {}\n", g.speed));
    out.push_str(&format!("  zomma           {}\n", g.zomma));
    out.push_str(&format!("  color           {}\n", g.color));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_commodity_vanilla::{futures_style_price, price};

    /// The CLI core IS the leaf's margining dispatch bit-for-bit: equity-style
    /// reproduces the discounted closed form, futures-style the undiscounted
    /// one. Kills any drift between the command core and the engine.
    #[test]
    fn run_matches_the_leaf_bitwise_under_both_margining_styles() {
        let market = FutureOptionMarket {
            future: 19.0,
            vol: 0.28,
            t: 0.75,
            r_dom: 0.10,
        };
        let inputs = CommodityInputs::on_future(19.0, 19.0, 0.28, 0.75, 0.10);
        for opt in [OptionType::Call, OptionType::Put] {
            let eq = run(opt, 19.0, market, CliMargining::EquityStyle);
            assert_eq!(eq.price.to_bits(), price(opt, &inputs).to_bits());
            let fut = run(opt, 19.0, market, CliMargining::FuturesStyle);
            assert_eq!(
                fut.price.to_bits(),
                futures_style_price(opt, &inputs).to_bits()
            );
            // Futures-style: discount_rho is the exact zero of the daily-margined
            // premium (a financial statement, not a rounding artifact).
            match fut.rates {
                RateSensitivities::Carry { discount_rho, .. } => {
                    assert_eq!(discount_rho.to_bits(), 0.0_f64.to_bits());
                }
                RateSensitivities::Fx { .. } => panic!("on-future strip must tag as Carry"),
            }
        }
    }

    /// The report parses back the exact price (shortest-round-trip printing)
    /// and labels the contract identity `TICKER@MIC`.
    #[test]
    fn report_round_trips_the_price() {
        let market = FutureOptionMarket {
            future: 45.0,
            vol: 0.35,
            t: 0.5,
            r_dom: 0.04,
        };
        let g = run(OptionType::Call, 40.0, market, CliMargining::FuturesStyle);
        let report = format_report(
            OptionType::Call,
            "ES",
            "XCME",
            0.75,
            CliMargining::FuturesStyle,
            40.0,
            &g,
        );
        let printed: f64 = report
            .lines()
            .find_map(|l| l.trim().strip_prefix("price"))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|tok| tok.parse().ok())
            .expect("price line parses");
        assert_eq!(printed.to_bits(), g.price.to_bits());
        assert!(report.contains("contract        ES@XCME"));
        assert!(report.contains("margining       futures-style"));
    }
}
