//! The clap command tree and the dispatch from parsed arguments to the
//! subcommand core functions.
//!
//! `main` parses [`Cli`] and calls [`dispatch`], which validates / converts the
//! arguments and invokes the relevant module's `run` (the same functions the unit
//! tests assert against the underlying crates). Output is written to the supplied
//! sink so the dispatch is testable without capturing global stdout.

use std::io::Write;

use celnet_types::{AtmConvention, CcyPair};
use clap::{Args, Parser, Subcommand};

use crate::args::{CliBarrier, CliDeltaConvention, CliDigital, CliOptionType, Market};
use crate::tenor::parse_tenor;
use crate::{convention, exotic, price, surface};

/// The Celnet operator/quant CLI.
#[derive(Debug, Parser)]
#[command(
    name = "celnet",
    about = "Celnet FX-options CLI: price vanillas & exotics, build smiles, inspect conventions.",
    version
)]
pub(crate) struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// The top-level subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Price a vanilla option (Garman-Kohlhagen) and its full Greek set.
    Price(PriceArgs),
    /// Build a smile from broker quotes and print vols + the arbitrage report.
    Surface(SurfaceArgs),
    /// Price a digital / one-touch / double-no-touch / single-barrier exotic.
    Exotic(ExoticArgs),
    /// Resolve and print the convention record for a pair and tenor.
    Convention(ConventionArgs),
}

/// Shared Garman-Kohlhagen market inputs accepted by `price` and `exotic`.
#[derive(Debug, Args)]
pub(crate) struct MarketArgs {
    /// Spot FX rate (quote per 1 unit of base).
    #[arg(long)]
    pub(crate) spot: f64,
    /// Annualized volatility (absolute, e.g. 0.10 = 10 vol).
    #[arg(long)]
    pub(crate) vol: f64,
    /// Time to expiry in years.
    #[arg(long)]
    pub(crate) t: f64,
    /// Continuously-compounded domestic (quote) rate.
    #[arg(long)]
    pub(crate) r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    #[arg(long)]
    pub(crate) r_for: f64,
}

impl MarketArgs {
    /// The cohesive [`Market`] these flat market flags describe.
    fn to_market(&self) -> Market {
        Market {
            spot: self.spot,
            vol: self.vol,
            t: self.t,
            r_dom: self.r_dom,
            r_for: self.r_for,
        }
    }
}

/// Arguments to `price`.
#[derive(Debug, Args)]
pub(crate) struct PriceArgs {
    /// Call or put.
    #[arg(long, value_enum)]
    pub(crate) option: CliOptionType,
    /// Shared market inputs.
    #[command(flatten)]
    pub(crate) market: MarketArgs,
    /// Explicit strike (mutually exclusive with `--delta` / `--atm`).
    #[arg(long, group = "strike_spec")]
    pub(crate) strike: Option<f64>,
    /// Solve the strike from this signed target delta under `--delta-convention`.
    #[arg(long, group = "strike_spec")]
    pub(crate) delta: Option<f64>,
    /// Use the ATM strike (forward / delta-neutral straddle) instead of a strike.
    #[arg(long, group = "strike_spec")]
    pub(crate) atm: bool,
    /// The delta convention for the reported convention delta and for `--delta`.
    #[arg(long, value_enum, default_value = "spot-unadj")]
    pub(crate) delta_convention: CliDeltaConvention,
    /// ATM strike convention (used only with `--atm`).
    #[arg(long, value_enum, default_value = "atm-forward")]
    pub(crate) atm_convention: CliAtmConvention,
}

/// ATM strike convention on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliAtmConvention {
    /// ATM-forward: `K = F`.
    AtmForward,
    /// Delta-neutral straddle.
    DeltaNeutral,
}

impl From<CliAtmConvention> for AtmConvention {
    fn from(v: CliAtmConvention) -> Self {
        match v {
            CliAtmConvention::AtmForward => AtmConvention::AtmForward,
            CliAtmConvention::DeltaNeutral => AtmConvention::DeltaNeutralStraddle,
        }
    }
}

/// Arguments to `surface`.
#[derive(Debug, Args)]
pub(crate) struct SurfaceArgs {
    /// Currency pair, e.g. EURUSD.
    #[arg(long)]
    pub(crate) pair: String,
    /// Tenor shorthand, e.g. 1Y, 3M, 2W, ON.
    #[arg(long)]
    pub(crate) tenor: String,
    /// Spot FX rate.
    #[arg(long)]
    pub(crate) spot: f64,
    /// Continuously-compounded domestic rate.
    #[arg(long)]
    pub(crate) r_dom: f64,
    /// Continuously-compounded foreign rate.
    #[arg(long)]
    pub(crate) r_for: f64,
    /// Vol-time to expiry in years.
    #[arg(long)]
    pub(crate) t: f64,
    /// ATM volatility (absolute).
    #[arg(long)]
    pub(crate) atm: f64,
    /// 25Δ risk reversal (absolute vol; call wing minus put wing).
    #[arg(long)]
    pub(crate) rr: f64,
    /// 25Δ butterfly (absolute vol).
    #[arg(long)]
    pub(crate) bf: f64,
    /// Lower strike band as a multiple of the forward.
    #[arg(long, default_value_t = 0.85)]
    pub(crate) lo: f64,
    /// Upper strike band as a multiple of the forward.
    #[arg(long, default_value_t = 1.15)]
    pub(crate) hi: f64,
    /// Number of grid strikes (≥ 3).
    #[arg(long, default_value_t = 11)]
    pub(crate) points: usize,
    /// Arbitrage tolerance for the pass/fail verdict.
    #[arg(long, default_value_t = 1e-6)]
    pub(crate) arb_tol: f64,
}

/// Arguments to `exotic`.
#[derive(Debug, Args)]
pub(crate) struct ExoticArgs {
    /// Shared market inputs.
    #[command(flatten)]
    pub(crate) market: MarketArgs,
    /// Strike (used by `digital` and `single-barrier`; ignored by touches).
    #[arg(long, default_value_t = 0.0)]
    pub(crate) strike: f64,
    /// Which exotic to price.
    #[command(subcommand)]
    pub(crate) kind: ExoticKind,
}

/// The exotic variant to price.
#[derive(Debug, Subcommand)]
pub(crate) enum ExoticKind {
    /// A European cash-or-nothing digital.
    Digital {
        /// Digital direction.
        #[arg(long, value_enum)]
        kind: CliDigital,
    },
    /// A one-touch.
    OneTouch {
        /// Barrier level.
        #[arg(long)]
        barrier: f64,
        /// Domestic rebate paid on the touch.
        #[arg(long, default_value_t = 1.0)]
        rebate: f64,
        /// Pay the rebate at expiry instead of at hit.
        #[arg(long, default_value_t = false)]
        at_expiry: bool,
    },
    /// A double-no-touch.
    Dnt {
        /// Lower corridor barrier.
        #[arg(long)]
        lower: f64,
        /// Upper corridor barrier.
        #[arg(long)]
        upper: f64,
        /// Domestic rebate paid if neither barrier is touched.
        #[arg(long, default_value_t = 1.0)]
        rebate: f64,
    },
    /// A single-barrier vanilla.
    Barrier {
        /// Underlying option type.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Barrier topology.
        #[arg(long, value_enum)]
        topology: CliBarrier,
        /// Barrier level.
        #[arg(long)]
        barrier: f64,
        /// Rebate paid on the terminating event (domestic).
        #[arg(long, default_value_t = 0.0)]
        rebate: f64,
    },
}

/// Arguments to `convention`.
#[derive(Debug, Args)]
pub(crate) struct ConventionArgs {
    /// Currency pair, e.g. EURUSD.
    #[arg(long)]
    pub(crate) pair: String,
    /// Tenor shorthand, e.g. 1Y, 3M, ON.
    #[arg(long)]
    pub(crate) tenor: String,
}

/// A user-facing dispatch failure (bad argument or a numerical solve failure).
///
/// Distinct from clap's own parse errors (which clap reports directly); these are
/// the semantic validations the subcommands perform after parsing.
#[derive(Debug)]
pub(crate) enum DispatchError {
    /// A currency pair string did not parse.
    BadPair(String),
    /// A tenor shorthand did not parse.
    BadTenor(crate::tenor::TenorParseError),
    /// A `price` strike-resolution / solve failure.
    Price(price::PriceError),
    /// A `surface` calibration failure.
    Surface(celnet_surface::CalibrationError),
    /// An argument was out of its valid domain.
    Invalid(String),
}

impl core::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DispatchError::BadPair(s) => write!(f, "unknown currency pair `{s}`"),
            DispatchError::BadTenor(e) => write!(f, "{e}"),
            DispatchError::Price(e) => write!(f, "{e}"),
            DispatchError::Surface(e) => write!(f, "surface calibration failed: {e:?}"),
            DispatchError::Invalid(s) => write!(f, "invalid argument: {s}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Convert a pair string into a [`CcyPair`], or a [`DispatchError::BadPair`].
fn parse_pair(s: &str) -> Result<CcyPair, DispatchError> {
    CcyPair::parse(s).ok_or_else(|| DispatchError::BadPair(s.to_owned()))
}

/// Run a parsed [`Cli`], writing the formatted report to `out`.
///
/// # Errors
///
/// [`DispatchError`] for a semantic argument failure (bad pair/tenor, an
/// out-of-domain value, or a numerical solve/calibration failure). Clap has
/// already rejected structurally-invalid input before this is called.
pub(crate) fn dispatch<W: Write>(cli: Cli, out: &mut W) -> Result<(), DispatchError> {
    match cli.command {
        Command::Price(a) => {
            let option = a.option.into();
            let spec = if let Some(k) = a.strike {
                price::StrikeSpec::Outright(k)
            } else if let Some(d) = a.delta {
                price::StrikeSpec::Delta {
                    target: d,
                    convention: a.delta_convention.into(),
                }
            } else if a.atm {
                price::StrikeSpec::Atm {
                    atm: a.atm_convention.into(),
                    convention: a.delta_convention.into(),
                }
            } else {
                return Err(DispatchError::Invalid(
                    "specify exactly one of --strike, --delta, or --atm".to_owned(),
                ));
            };
            let r = price::run(
                option,
                spec,
                a.market.to_market(),
                a.delta_convention.into(),
            )
            .map_err(DispatchError::Price)?;
            write!(out, "{}", price::format_report(option, &r)).ok();
            Ok(())
        }
        Command::Surface(a) => {
            let pair = parse_pair(&a.pair)?;
            let tenor = parse_tenor(&a.tenor).map_err(DispatchError::BadTenor)?;
            if a.points < 3 {
                return Err(DispatchError::Invalid(
                    "--points must be at least 3".to_owned(),
                ));
            }
            if !(a.lo > 0.0 && a.lo < a.hi) {
                return Err(DispatchError::Invalid(
                    "strike band must satisfy 0 < lo < hi".to_owned(),
                ));
            }
            let r = surface::run(surface::SurfaceRequest {
                pair,
                tenor,
                spot: a.spot,
                r_dom: a.r_dom,
                r_for: a.r_for,
                t: a.t,
                atm_vol: a.atm,
                rr_25: a.rr,
                bf_25: a.bf,
                lo: a.lo,
                hi: a.hi,
                points: a.points,
            })
            .map_err(DispatchError::Surface)?;
            write!(out, "{}", surface::format_report(&r, a.arb_tol)).ok();
            Ok(())
        }
        Command::Exotic(a) => {
            let inputs = a.market.to_market().inputs(a.strike);
            let spec = match a.kind {
                ExoticKind::Digital { kind } => exotic::ExoticSpec::Digital(kind.into()),
                ExoticKind::OneTouch {
                    barrier,
                    rebate,
                    at_expiry,
                } => exotic::ExoticSpec::OneTouch {
                    barrier,
                    rebate,
                    at_expiry,
                },
                ExoticKind::Dnt {
                    lower,
                    upper,
                    rebate,
                } => {
                    if !(lower > 0.0 && lower < upper && rebate >= 0.0) {
                        return Err(DispatchError::Invalid(
                            "DNT corridor must satisfy 0 < lower < upper and rebate ≥ 0".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::DoubleNoTouch {
                        lower,
                        upper,
                        rebate,
                    }
                }
                ExoticKind::Barrier {
                    option,
                    topology,
                    barrier,
                    rebate,
                } => exotic::ExoticSpec::SingleBarrier {
                    option: option.into(),
                    topology,
                    strike: a.strike,
                    barrier,
                    rebate,
                },
            };
            let r = exotic::run(spec, &inputs);
            write!(out, "{}", exotic::format_report(spec, &r)).ok();
            Ok(())
        }
        Command::Convention(a) => {
            let pair = parse_pair(&a.pair)?;
            let tenor = parse_tenor(&a.tenor).map_err(DispatchError::BadTenor)?;
            let resolved = convention::run(pair, tenor);
            write!(out, "{}", convention::format_report(pair, tenor, &resolved)).ok();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn run_to_string(argv: &[&str]) -> Result<String, DispatchError> {
        let cli = Cli::try_parse_from(argv).expect("args parse");
        let mut buf = Vec::new();
        dispatch(cli, &mut buf)?;
        Ok(String::from_utf8(buf).expect("utf8"))
    }

    #[test]
    fn price_parses_and_prices() {
        let out = run_to_string(&[
            "celnet", "price", "--option", "call", "--spot", "1.10", "--strike", "1.12", "--vol",
            "0.10", "--t", "1.0", "--r-dom", "0.02", "--r-for", "0.01",
        ])
        .unwrap();
        assert!(out.contains("vanilla call"));
        assert!(out.contains("price"));
        assert!(out.contains("vega"));
    }

    #[test]
    fn price_requires_a_strike_spec() {
        let err = run_to_string(&[
            "celnet", "price", "--option", "call", "--spot", "1.10", "--vol", "0.10", "--t", "1.0",
            "--r-dom", "0.02", "--r-for", "0.01",
        ])
        .unwrap_err();
        assert!(matches!(err, DispatchError::Invalid(_)));
    }

    #[test]
    fn exotic_barrier_parses_and_prices() {
        let out = run_to_string(&[
            "celnet",
            "exotic",
            "--spot",
            "1.10",
            "--vol",
            "0.10",
            "--t",
            "1.0",
            "--r-dom",
            "0.02",
            "--r-for",
            "0.01",
            "--strike",
            "1.10",
            "barrier",
            "--option",
            "call",
            "--topology",
            "down-and-out",
            "--barrier",
            "0.95",
        ])
        .unwrap();
        assert!(out.contains("single-barrier"));
        assert!(out.contains("price"));
    }

    #[test]
    fn surface_parses_and_reports() {
        let out = run_to_string(&[
            "celnet", "surface", "--pair", "EURUSD", "--tenor", "1Y", "--spot", "1.10", "--r-dom",
            "0.02", "--r-for", "0.01", "--t", "1.0", "--atm", "0.105", "--rr", "0.015", "--bf",
            "0.0035",
        ])
        .unwrap();
        assert!(out.contains("forward"));
        assert!(out.contains("arbitrage"));
    }

    #[test]
    fn convention_parses_and_reports() {
        let out =
            run_to_string(&["celnet", "convention", "--pair", "EURUSD", "--tenor", "3M"]).unwrap();
        assert!(out.contains("EURUSD"));
        assert!(out.contains("delta"));
    }

    #[test]
    fn bad_pair_is_an_error() {
        let err = run_to_string(&["celnet", "convention", "--pair", "NOPE", "--tenor", "1Y"])
            .unwrap_err();
        assert!(matches!(err, DispatchError::BadPair(_)));
    }
}
