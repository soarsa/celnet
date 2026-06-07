//! The `basket` subcommand: price a correlated multi-asset FX option (weighted
//! basket / best-of / worst-of) over N currency-pair legs.
//!
//! The price is the Cholesky-correlated multi-asset GBM Monte-Carlo estimate
//! from [`celnet_exotics::price_basket`] (the same engine the server pricer arm
//! drives — api-first parity). Multi-asset Greeks are deferred, so the report
//! shows the price and its Monte-Carlo standard error only.

use celnet_exotics::{
    BasketKind, BasketLeg, BasketMcConfig, BasketSpec, CorrelationError, price_basket,
};
use celnet_types::OptionType;

/// A parsed leg: a label plus its market data and weight.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedLeg {
    /// The pair label (e.g. `EURUSD`), purely for the report.
    pub(crate) label: String,
    /// The leg engine inputs.
    pub(crate) leg: BasketLeg,
}

/// Parse a `--leg PAIR:WEIGHT:SPOT:VOL:R_FOR` flag value.
///
/// # Errors
///
/// A human-readable message if the field count is wrong or a number does not
/// parse / is out of domain.
pub(crate) fn parse_leg(s: &str) -> Result<ParsedLeg, String> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 5 {
        return Err(format!(
            "--leg must be PAIR:WEIGHT:SPOT:VOL:R_FOR (got `{s}`)"
        ));
    }
    let label = parts[0].to_owned();
    if label.is_empty() {
        return Err("--leg pair label must be non-empty".to_owned());
    }
    let num = |i: usize, name: &str| -> Result<f64, String> {
        parts[i]
            .parse::<f64>()
            .map_err(|_| format!("--leg {name} `{}` is not a number", parts[i]))
    };
    let weight = num(1, "weight")?;
    let spot = num(2, "spot")?;
    let vol = num(3, "vol")?;
    let r_for = num(4, "r_for")?;
    if !(spot.is_finite() && spot > 0.0) {
        return Err("--leg spot must be positive".to_owned());
    }
    if !(vol.is_finite() && vol >= 0.0) {
        return Err("--leg vol must be non-negative".to_owned());
    }
    if !weight.is_finite() {
        return Err("--leg weight must be finite".to_owned());
    }
    if !r_for.is_finite() {
        return Err("--leg r_for must be finite".to_owned());
    }
    Ok(ParsedLeg {
        label,
        leg: BasketLeg::new(spot, vol, r_for, weight),
    })
}

/// The aggregation kind on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliBasketKind {
    /// Weighted arithmetic basket.
    Basket,
    /// Best-of-N (rainbow max).
    BestOf,
    /// Worst-of-N (rainbow min).
    WorstOf,
}

impl From<CliBasketKind> for BasketKind {
    fn from(v: CliBasketKind) -> Self {
        match v {
            CliBasketKind::Basket => BasketKind::Basket,
            CliBasketKind::BestOf => BasketKind::BestOf,
            CliBasketKind::WorstOf => BasketKind::WorstOf,
        }
    }
}

/// A fully-validated basket pricing request.
pub(crate) struct BasketRequest {
    /// The parsed legs (label + engine inputs).
    pub(crate) legs: Vec<ParsedLeg>,
    /// The row-major N×N correlation matrix.
    pub(crate) correlation: Vec<Vec<f64>>,
    /// Call or put on the aggregated underlying.
    pub(crate) option: OptionType,
    /// The strike `K`.
    pub(crate) strike: f64,
    /// The aggregation kind.
    pub(crate) kind: BasketKind,
    /// The shared domestic (numeraire / settlement) rate.
    pub(crate) r_dom: f64,
    /// The expiry in vol-time years.
    pub(crate) t: f64,
    /// The Monte-Carlo configuration.
    pub(crate) cfg: BasketMcConfig,
}

/// The result of a basket run.
pub(crate) struct BasketResult {
    /// The discounted price estimate.
    pub(crate) price: f64,
    /// The measured Monte-Carlo standard error.
    pub(crate) std_error: f64,
}

/// Price the requested basket.
///
/// # Errors
///
/// [`CorrelationError`] if the correlation matrix is not valid SPD for the leg
/// count.
pub(crate) fn run(req: &BasketRequest) -> Result<BasketResult, CorrelationError> {
    let spec = BasketSpec {
        legs: req.legs.iter().map(|p| p.leg).collect(),
        correlation: req.correlation.clone(),
        option_type: req.option,
        strike: req.strike,
        kind: req.kind,
    };
    let est = price_basket(&spec, req.r_dom, req.t, req.cfg)?;
    Ok(BasketResult {
        price: est.price,
        std_error: est.std_error,
    })
}

/// Format the report for a basket run.
#[must_use]
pub(crate) fn format_report(req: &BasketRequest, r: &BasketResult) -> String {
    let kind = match req.kind {
        BasketKind::Basket => "basket",
        BasketKind::BestOf => "best-of",
        BasketKind::WorstOf => "worst-of",
    };
    let opt = match req.option {
        OptionType::Call => "call",
        OptionType::Put => "put",
    };
    let mut s = String::new();
    s.push_str(&format!(
        "{kind} {opt}  strike {:.6}  expiry {:.4}y  legs {}\n",
        req.strike,
        req.t,
        req.legs.len()
    ));
    for p in &req.legs {
        s.push_str(&format!(
            "  leg {:<8} weight {:+.4}  spot {:.6}  vol {:.4}  r_for {:.4}\n",
            p.label, p.leg.weight, p.leg.spot, p.leg.vol, p.leg.r_for
        ));
    }
    s.push_str(&format!(
        "price        {:.6}\nstd_error    {:.6}  (Monte-Carlo; multi-asset Greeks deferred)\n",
        r.price, r.std_error
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_leg_round_trips() {
        let p = parse_leg("EURUSD:0.5:1.10:0.11:0.015").unwrap();
        assert_eq!(p.label, "EURUSD");
        assert_eq!(p.leg.weight.to_bits(), 0.5_f64.to_bits());
        assert_eq!(p.leg.spot.to_bits(), 1.10_f64.to_bits());
        assert_eq!(p.leg.vol.to_bits(), 0.11_f64.to_bits());
        assert_eq!(p.leg.r_for.to_bits(), 0.015_f64.to_bits());
    }

    #[test]
    fn parse_leg_rejects_bad_arity() {
        assert!(parse_leg("EURUSD:0.5:1.10").is_err());
    }

    #[test]
    fn run_prices_a_two_asset_basket() {
        let req = BasketRequest {
            legs: vec![
                parse_leg("EURUSD:0.5:1.10:0.11:0.015").unwrap(),
                parse_leg("GBPUSD:0.5:1.27:0.13:0.02").unwrap(),
            ],
            correlation: vec![vec![1.0, 0.4], vec![0.4, 1.0]],
            option: OptionType::Call,
            strike: 1.18,
            kind: BasketKind::Basket,
            r_dom: 0.02,
            t: 1.0,
            cfg: BasketMcConfig {
                budget: 2048,
                replications: 8,
                steps: 1,
                seed: 7,
            },
        };
        let r = run(&req).unwrap();
        assert!(r.price > 0.0 && r.price.is_finite());
        assert!(r.std_error >= 0.0 && r.std_error.is_finite());
        let report = format_report(&req, &r);
        assert!(report.contains("basket call"));
        assert!(report.contains("price"));
        assert!(report.contains("std_error"));
    }

    #[test]
    fn run_rejects_non_psd_correlation() {
        let req = BasketRequest {
            legs: vec![
                parse_leg("EURUSD:1.0:1.10:0.11:0.0").unwrap(),
                parse_leg("GBPUSD:1.0:1.27:0.13:0.0").unwrap(),
            ],
            correlation: vec![vec![1.0, 1.01], vec![1.01, 1.0]],
            option: OptionType::Call,
            strike: 1.18,
            kind: BasketKind::Basket,
            r_dom: 0.02,
            t: 1.0,
            cfg: BasketMcConfig::default(),
        };
        match run(&req) {
            Err(CorrelationError::NotPositiveDefinite) => {}
            other => panic!("expected NotPositiveDefinite, got {:?}", other.is_ok()),
        }
    }
}
