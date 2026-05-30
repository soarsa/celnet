//! The `price` subcommand: Garman-Kohlhagen vanilla price + the full Greek set
//! from command-line inputs, with convention-aware strike resolution.
//!
//! The strike can be supplied directly, or solved from a target delta under the
//! chosen delta convention, or taken as the ATM strike (forward / delta-neutral
//! straddle) for the chosen ATM convention. The convention delta of the resulting
//! option is reported alongside the Greeks. All numerics route through
//! `celnet-vanilla`; this module only marshals arguments and formats output.

use celnet_types::{AtmConvention, DeltaConvention, Greeks, OptionType};

use crate::args::Market;

/// How the strike is determined for a `price` run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum StrikeSpec {
    /// An explicit strike (quote per 1 unit of base).
    Outright(f64),
    /// Solve the strike from a signed target delta under `convention`.
    Delta {
        /// Signed target delta (call ≥ 0, put ≤ 0).
        target: f64,
        /// Delta convention the target is quoted in.
        convention: DeltaConvention,
    },
    /// The ATM strike for the given ATM/delta conventions.
    Atm {
        /// ATM strike convention.
        atm: AtmConvention,
        /// Delta convention used by the delta-neutral straddle ATM.
        convention: DeltaConvention,
    },
}

/// The resolved inputs and outputs of a `price` run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PriceResult {
    /// The strike actually priced (after any delta/ATM resolution).
    pub(crate) strike: f64,
    /// The full Garman-Kohlhagen Greek set.
    pub(crate) greeks: Greeks,
    /// The convention delta of the priced option under `delta_convention`.
    pub(crate) convention_delta: f64,
    /// The delta convention the convention delta was taken in.
    pub(crate) delta_convention: DeltaConvention,
}

/// A failure resolving or pricing a `price` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PriceError {
    /// The delta→strike inversion failed (e.g. an unreachable target delta).
    DeltaSolve(celnet_vanilla::DeltaSolveError),
}

impl core::fmt::Display for PriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PriceError::DeltaSolve(e) => write!(f, "delta→strike solve failed: {e:?}"),
        }
    }
}

impl std::error::Error for PriceError {}

/// Resolve the strike, then price the option and its convention delta.
///
/// The single flat `market.vol` is used for both the (optional) delta/ATM strike
/// resolution and the pricing — a self-consistent quote. `delta_convention` is the
/// convention the reported convention delta is taken in.
///
/// # Errors
///
/// [`PriceError::DeltaSolve`] if a [`StrikeSpec::Delta`] target cannot be
/// inverted to a strike.
pub(crate) fn run(
    option: OptionType,
    spec: StrikeSpec,
    market: Market,
    delta_convention: DeltaConvention,
) -> Result<PriceResult, PriceError> {
    // A template at a placeholder strike for the resolvers (they ignore strike
    // except as a starting scale; the solver overwrites it).
    let template = market.inputs(market.spot);
    let strike = match spec {
        StrikeSpec::Outright(k) => k,
        StrikeSpec::Delta { target, convention } => {
            celnet_vanilla::strike_from_delta(convention, option, target, &template)
                .map_err(PriceError::DeltaSolve)?
        }
        StrikeSpec::Atm { atm, convention } => {
            celnet_vanilla::atm_strike_from_inputs(atm, convention, &template)
        }
    };

    let inputs = market.inputs(strike);
    let greeks = celnet_vanilla::greeks(option, &inputs);
    let convention_delta = celnet_vanilla::convention_delta(delta_convention, option, &inputs);

    Ok(PriceResult {
        strike,
        greeks,
        convention_delta,
        delta_convention,
    })
}

/// Render a [`PriceResult`] as a stable, aligned key/value report.
#[must_use]
pub(crate) fn format_report(option: OptionType, r: &PriceResult) -> String {
    let g = &r.greeks;
    let mut out = String::new();
    let side = match option {
        OptionType::Call => "call",
        OptionType::Put => "put",
    };
    out.push_str(&format!("vanilla {side}\n"));
    out.push_str(&format!("  strike          {:.10}\n", r.strike));
    out.push_str(&format!("  price           {:.10}\n", g.price));
    out.push_str(&format!("  delta_spot      {:.10}\n", g.delta_spot));
    out.push_str(&format!("  delta_forward   {:.10}\n", g.delta_forward));
    out.push_str(&format!(
        "  conv_delta      {:.10}  ({:?})\n",
        r.convention_delta, r.delta_convention
    ));
    out.push_str(&format!("  gamma           {:.10}\n", g.gamma));
    out.push_str(&format!("  vega            {:.10}\n", g.vega));
    out.push_str(&format!("  theta           {:.10}\n", g.theta));
    out.push_str(&format!("  rho_dom         {:.10}\n", g.rho_dom));
    out.push_str(&format!("  rho_for         {:.10}\n", g.rho_for));
    out.push_str(&format!("  vanna           {:.10}\n", g.vanna));
    out.push_str(&format!("  volga           {:.10}\n", g.volga));
    out.push_str(&format!("  charm           {:.10}\n", g.charm));
    out.push_str(&format!("  speed           {:.10}\n", g.speed));
    out.push_str(&format!("  zomma           {:.10}\n", g.zomma));
    out.push_str(&format!("  color           {:.10}\n", g.color));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_types::VanillaInputs;

    #[test]
    fn outright_matches_direct_vanilla() {
        let opt = OptionType::Call;
        let (spot, strike, vol, t, rd, rf) = (1.10, 1.12, 0.10, 1.0, 0.02, 0.01);
        let market = Market {
            spot,
            vol,
            t,
            r_dom: rd,
            r_for: rf,
        };
        let r = run(
            opt,
            StrikeSpec::Outright(strike),
            market,
            DeltaConvention::SpotUnadjusted,
        )
        .unwrap();

        let inputs = VanillaInputs::new(spot, strike, vol, t, rd, rf);
        let direct = celnet_vanilla::greeks(opt, &inputs);
        assert!(is_close(r.greeks.price, direct.price, 1e-14, 1e-14));
        assert!(is_close(r.greeks.vega, direct.vega, 1e-14, 1e-14));
        assert!(is_close(r.greeks.gamma, direct.gamma, 1e-14, 1e-14));
        // The convention delta matches the underlying solver on identical inputs.
        let cd = celnet_vanilla::convention_delta(DeltaConvention::SpotUnadjusted, opt, &inputs);
        assert!(is_close(r.convention_delta, cd, 1e-14, 1e-14));
    }

    #[test]
    fn delta_spec_round_trips_to_target_delta() {
        // Solving a strike from a 25Δ target and then reporting that convention
        // delta must return the target — the solver is its own inverse.
        let opt = OptionType::Call;
        let conv = DeltaConvention::SpotUnadjusted;
        let (spot, vol, t, rd, rf) = (1.10, 0.10, 1.0, 0.02, 0.01);
        let market = Market {
            spot,
            vol,
            t,
            r_dom: rd,
            r_for: rf,
        };
        let r = run(
            opt,
            StrikeSpec::Delta {
                target: 0.25,
                convention: conv,
            },
            market,
            conv,
        )
        .unwrap();
        assert!(
            is_close(r.convention_delta, 0.25, 1e-9, 1e-9),
            "recovered delta {} != target 0.25",
            r.convention_delta
        );
        // And the strike equals the direct solver result.
        let template = VanillaInputs::new(spot, spot, vol, t, rd, rf);
        let k = celnet_vanilla::strike_from_delta(conv, opt, 0.25, &template).unwrap();
        assert!(is_close(r.strike, k, 1e-14, 1e-14));
    }

    #[test]
    fn atm_forward_strike_equals_forward() {
        let opt = OptionType::Call;
        let (spot, vol, t, rd, rf) = (1.10, 0.10, 1.0, 0.02, 0.01);
        let market = Market {
            spot,
            vol,
            t,
            r_dom: rd,
            r_for: rf,
        };
        let r = run(
            opt,
            StrikeSpec::Atm {
                atm: AtmConvention::AtmForward,
                convention: DeltaConvention::SpotUnadjusted,
            },
            market,
            DeltaConvention::SpotUnadjusted,
        )
        .unwrap();
        let forward = VanillaInputs::new(spot, spot, vol, t, rd, rf).forward();
        assert!(is_close(r.strike, forward, 1e-12, 1e-12));
    }
}
