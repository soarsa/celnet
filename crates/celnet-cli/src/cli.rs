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
use crate::basket::{self, CliBasketKind};
use crate::risk::{
    self, AggregateReq, CliDimension, DrillReq, LimitsReq, PositionsReq, RiskCommon, StreamReq,
};
use crate::tenor::parse_tenor;
use crate::{convention, exotic, price, surface};

use celnet_client::{OrgDimension, Scope, StrikeSpec};

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
    /// Price a correlated multi-asset FX option (weighted basket / best-of /
    /// worst-of) over N currency-pair legs (Cholesky-correlated multi-asset GBM
    /// Monte-Carlo; reports a standard error; multi-asset Greeks deferred).
    Basket(BasketArgs),
    /// Resolve and print the convention record for a pair and tenor.
    Convention(ConventionArgs),
    /// Firm-scale hierarchical risk against a running edge (the same `RiskService`
    /// the GUI Book view and Excel `CELNET.*` consume, via the `celnet-client` SDK).
    Risk(RiskArgs),
    /// Subscribe to a two-way RFS stream for an instrument against a running edge,
    /// print the sequenced ticks, then unsubscribe cleanly (the `celnet-client`
    /// multiplexed session — the same stream the GUI blotter consumes).
    Stream(StreamArgs),
}

/// Arguments to `risk` — the edge endpoint, the entitlement scope flags, and one
/// of the four risk sub-subcommands.
#[derive(Debug, Args)]
pub(crate) struct RiskArgs {
    /// The gRPC endpoint of the edge, e.g. `http://127.0.0.1:50551`.
    #[arg(long, default_value = "http://127.0.0.1:50551")]
    pub(crate) endpoint: String,
    /// The reporting currency every node measure is expressed in.
    #[arg(long, default_value = "USD")]
    pub(crate) numeraire: String,
    /// A spot conversion rate `CCY=RATE` (units of numeraire per 1 unit of ccy),
    /// repeatable. The numeraire's own rate is implicitly 1.0; a rate missing for a
    /// currency in the book fails the request loudly server-side.
    #[arg(long = "rate", value_parser = risk::parse_rate)]
    pub(crate) rates: Vec<(String, f64)>,
    /// A read grant `DIM:VALUE` (repeatable); any grant switches to a
    /// deny-by-default scoped principal. No grant/deny ⇒ the grant-all
    /// (show-all-now) default, identical to the GUI/SDK/Excel.
    #[arg(long = "grant", value_parser = risk::parse_scope_flag)]
    pub(crate) grants: Vec<(OrgDimension, u64)>,
    /// An information-barrier deny `DIM:VALUE` (repeatable); deny wins over any
    /// grant (a Chinese wall on a grant-all firm view).
    #[arg(long = "deny", value_parser = risk::parse_scope_flag)]
    pub(crate) denies: Vec<(OrgDimension, u64)>,
    /// The risk sub-subcommand.
    #[command(subcommand)]
    pub(crate) kind: RiskKind,
}

/// The `risk` sub-subcommands — one per `RiskService` operation.
#[derive(Debug, Subcommand)]
pub(crate) enum RiskKind {
    /// Roll the entitled book up over an org dimension into a node tree.
    Aggregate {
        /// The dimension to group along.
        #[arg(long, value_enum, default_value = "firm")]
        dimension: CliDimension,
        /// Narrow to one `DIM:VALUE` subtree before the group-by.
        #[arg(long = "scope", value_parser = risk::parse_scope_flag)]
        scope: Option<(OrgDimension, u64)>,
        /// Evaluate VaR/ES by bumping spot over these relative shocks (repeatable,
        /// e.g. `--var-shock -0.01 --var-shock 0.01`). Without any, the non-additive
        /// block is absent (never a spurious zero).
        #[arg(long = "var-shock", allow_hyphen_values = true)]
        var_shocks: Vec<f64>,
        /// The VaR/ES confidence level (e.g. 0.99).
        #[arg(long, default_value_t = 0.99)]
        var_alpha: f64,
        /// Charge FRTB-SbM spot curvature at this risk weight (0 ⇒ not evaluated).
        #[arg(long, default_value_t = 0.0)]
        curvature: f64,
    },
    /// Drill one node into its child sub-nodes and/or constituent positions.
    Drill {
        /// The node to drill, `DIM:VALUE` (use `firm:0` for the apex).
        #[arg(long, value_parser = risk::parse_scope_flag)]
        node: (OrgDimension, u64),
        /// The finer dimension to break children out at.
        #[arg(long, value_enum, default_value = "book")]
        child_dimension: CliDimension,
        /// Return the child sub-nodes.
        #[arg(long, default_value_t = false)]
        children: bool,
        /// Return the constituent positions.
        #[arg(long, default_value_t = false)]
        positions: bool,
    },
    /// List the entitled open book.
    Positions {
        /// Narrow the listing to one `DIM:VALUE` subtree.
        #[arg(long = "scope", value_parser = risk::parse_scope_flag)]
        scope: Option<(OrgDimension, u64)>,
    },
    /// Read the limit-tree RAG / utilization at a scope.
    Limits {
        /// The scope to evaluate, `DIM:VALUE`.
        #[arg(long, value_parser = risk::parse_scope_flag)]
        scope: (OrgDimension, u64),
        /// VaR/ES spot shocks for the non-additive limits (repeatable).
        #[arg(long = "var-shock", allow_hyphen_values = true)]
        var_shocks: Vec<f64>,
        /// The VaR/ES confidence level.
        #[arg(long, default_value_t = 0.99)]
        var_alpha: f64,
    },
}

/// Arguments to `stream`.
#[derive(Debug, Args)]
pub(crate) struct StreamArgs {
    /// The gRPC endpoint of the edge.
    #[arg(long, default_value = "http://127.0.0.1:50551")]
    pub(crate) endpoint: String,
    /// Currency pair, e.g. EURUSD.
    #[arg(long)]
    pub(crate) pair: String,
    /// Tenor shorthand, e.g. 1Y, 3M, ON.
    #[arg(long, default_value = "1Y")]
    pub(crate) tenor: String,
    /// Time to expiry in years (authoritative for pricing).
    #[arg(long, default_value_t = 1.0)]
    pub(crate) expiry_years: f64,
    /// Call or put.
    #[arg(long, value_enum, default_value = "call")]
    pub(crate) option: CliOptionType,
    /// Explicit strike (mutually exclusive with `--delta`).
    #[arg(long, group = "stream_strike")]
    pub(crate) strike: Option<f64>,
    /// A signed convention delta resolved to a strike server-side.
    #[arg(long, group = "stream_strike", allow_hyphen_values = true)]
    pub(crate) delta: Option<f64>,
    /// Base-currency notional.
    #[arg(long, default_value_t = 1_000_000.0)]
    pub(crate) notional: f64,
    /// The number of post-snapshot ticks to print before unsubscribing.
    #[arg(long, default_value_t = 3)]
    pub(crate) ticks: u32,
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

/// The booking / pricing model on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub(crate) enum CliPricingModel {
    /// The default analytic / closed-form engine.
    #[default]
    Analytic,
    /// The local-stochastic-volatility booking model (LSV: particle-calibrated
    /// leverage over a stochastic-variance backbone, ADI-PDE / Monte-Carlo).
    /// Supported only for `vanilla`, `barrier` (continuous knock-out/in) and
    /// `window-barrier`; any other product is rejected.
    Lsv,
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
    /// The booking / pricing model. `analytic` (default) uses the product's
    /// closed form; `lsv` routes the supported products (vanilla, barrier,
    /// window-barrier) through the local-stochastic-volatility engine.
    #[arg(long, value_enum, default_value = "analytic")]
    pub(crate) model: CliPricingModel,
    /// Which exotic to price.
    #[command(subcommand)]
    pub(crate) kind: ExoticKind,
}

/// The exotic variant to price.
#[derive(Debug, Subcommand)]
pub(crate) enum ExoticKind {
    /// A vanilla European option priced under the selected `--model` (use with
    /// `--model lsv` to price a vanilla on the LSV engine).
    Vanilla {
        /// Call or put.
        #[arg(long, value_enum)]
        option: CliOptionType,
    },
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
    /// A window knock-out barrier (active only inside a calendar window). Priced
    /// only under `--model lsv`; the ADI-PDE engine by default, or Monte-Carlo
    /// (with a std-error) when `--mc-pairs > 0`.
    WindowBarrier {
        /// Underlying option type.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Barrier level `H`.
        #[arg(long)]
        barrier: f64,
        /// `true` for up-and-out (barrier above spot), `false` for down-and-out.
        #[arg(long, default_value_t = true)]
        up: bool,
        /// Window start in years from inception.
        #[arg(long)]
        window_start: f64,
        /// Window end in years from inception.
        #[arg(long)]
        window_end: f64,
        /// Antithetic Monte-Carlo path pairs (`0` ⇒ exact ADI PDE; `> 0` ⇒
        /// Monte-Carlo with a std-error).
        #[arg(long, default_value_t = 0)]
        mc_pairs: usize,
        /// Monte-Carlo time steps (ignored when `--mc-pairs 0`; `0` ⇒ default).
        #[arg(long, default_value_t = 0)]
        mc_steps: usize,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        #[arg(long, default_value_t = 0)]
        mc_seed: u64,
    },
    /// A variance swap: print the fair variance strike `K_var` and `√K_var`.
    VarSwap,
    /// A volatility swap: print the fair vol strike `K_vol` (Carr-Lee adjusted).
    VolSwap,
    /// A fixed-strike arithmetic-average-rate Asian option.
    Asian {
        /// Underlying option type on the realised average.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Use continuous averaging instead of discrete fixings.
        #[arg(long, default_value_t = false)]
        continuous: bool,
        /// Number of equally-spaced future fixings (discrete style only).
        #[arg(long, default_value_t = 12)]
        observations: u32,
        /// Use the Turnbull-Wakeman estimator instead of the Curran default.
        #[arg(long, default_value_t = false)]
        turnbull_wakeman: bool,
        /// Realised running average of already-fixed observations (seasoned).
        #[arg(long, default_value_t = 0.0)]
        elapsed_avg: f64,
        /// Fraction `∈ [0, 1)` of the average weight already fixed (seasoned).
        #[arg(long, default_value_t = 0.0)]
        elapsed_weight: f64,
    },
    /// A forward-start vanilla (strike resets at a future date to m·S(reset)).
    ForwardStart {
        /// Call or put.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Strike-reset multiple `m` (`1.0` is the ATM-forward reset).
        #[arg(long, default_value_t = 1.0)]
        moneyness: f64,
        /// Reset (strike-fixing) date `t₁` in years (`0 ≤ reset ≤ expiry`).
        #[arg(long)]
        reset: f64,
    },
    /// A cliquet / ratchet (plain closed-form, or clamped Monte-Carlo with a
    /// reported standard error).
    Cliquet {
        /// Call or put per-period payoff direction.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Per-period strike-reset multiple `m`.
        #[arg(long, default_value_t = 1.0)]
        moneyness: f64,
        /// Number of evenly-spaced ratchet periods over `[0, expiry]`.
        #[arg(long, default_value_t = 4)]
        periods: u32,
        /// Per-period local floor on each clamped period return (omit ⇒ none).
        #[arg(long)]
        local_floor: Option<f64>,
        /// Per-period local cap on each clamped period return (omit ⇒ none).
        #[arg(long)]
        local_cap: Option<f64>,
        /// Global floor on the accumulated payoff (omit ⇒ none).
        #[arg(long)]
        global_floor: Option<f64>,
        /// Global cap on the accumulated payoff (omit ⇒ none).
        #[arg(long)]
        global_cap: Option<f64>,
        /// Antithetic Monte-Carlo path pairs for the clamped variant.
        #[arg(long, default_value_t = 200_000)]
        mc_pairs: usize,
        /// Counter-RNG seed for the clamped Monte-Carlo estimator.
        #[arg(long, default_value_t = 0)]
        mc_seed: u64,
    },
    /// A quanto option (vanilla or digital), settlement-currency converted.
    Quanto {
        /// Call or put.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Price the cash-or-nothing digital instead of the vanilla.
        #[arg(long, default_value_t = false)]
        digital: bool,
        /// Annualised volatility `σ_Z` of the settlement-conversion rate.
        #[arg(long)]
        conversion_vol: f64,
        /// Correlation `ρ ∈ [−1, 1]` between the underlying and the conversion rate.
        #[arg(long)]
        correlation: f64,
    },
    /// A Target-Redemption Forward (Monte-Carlo; reports a standard error).
    Tarf {
        /// The favourable-side direction (put = client gains when S < strike).
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// The cumulative gain target; reaching it redeems (knocks out).
        #[arg(long)]
        target: f64,
        /// Gearing/leverage on the adverse (loss) leg.
        #[arg(long, default_value_t = 1.0)]
        leverage: f64,
        /// Number of equally-spaced fixings over `[0, expiry]`.
        #[arg(long, default_value_t = 12)]
        fixings: u32,
        /// Per-fixing notional.
        #[arg(long, default_value_t = 1.0)]
        fixing_notional: f64,
        /// Settle the breaching fixing at the capped (remaining-target) gain
        /// instead of the full intrinsic (the default carries the gap exposure).
        #[arg(long, default_value_t = false)]
        capped_gain: bool,
        /// Antithetic Monte-Carlo path pairs.
        #[arg(long, default_value_t = 200_000)]
        mc_pairs: usize,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        #[arg(long, default_value_t = 0)]
        mc_seed: u64,
    },
    /// An accumulator (Monte-Carlo; reports a standard error).
    Accumulator {
        /// Pivot strike at which the client accumulates each fixing.
        #[arg(long)]
        pivot: f64,
        /// Up-and-out knock-out barrier (must sit above the pivot).
        #[arg(long)]
        barrier: f64,
        /// Gearing/leverage on the below-pivot (loss) leg.
        #[arg(long, default_value_t = 1.0)]
        leverage: f64,
        /// Number of equally-spaced fixings over `[0, expiry]`.
        #[arg(long, default_value_t = 12)]
        fixings: u32,
        /// Per-fixing notional.
        #[arg(long, default_value_t = 1.0)]
        fixing_notional: f64,
        /// Monitor the knock-out barrier continuously between fixings instead of
        /// only at the discrete fixing dates.
        #[arg(long, default_value_t = false)]
        continuous: bool,
        /// Antithetic Monte-Carlo path pairs.
        #[arg(long, default_value_t = 200_000)]
        mc_pairs: usize,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        #[arg(long, default_value_t = 0)]
        mc_seed: u64,
    },
    /// A lookback option (continuous closed-form, or discrete Monte-Carlo with a
    /// reported standard error).
    Lookback {
        /// Call or put.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Use the fixed-strike family (against `--strike`) instead of floating.
        #[arg(long, default_value_t = false)]
        fixed: bool,
        /// Monitor discretely over `--observations` fixings (Monte-Carlo) instead
        /// of continuously (exact closed form).
        #[arg(long, default_value_t = false)]
        discrete: bool,
        /// Number of equally-spaced monitoring observations for the discrete
        /// variant.
        #[arg(long, default_value_t = 64)]
        observations: u32,
        /// Antithetic Monte-Carlo path pairs for the discrete variant.
        #[arg(long, default_value_t = 200_000)]
        mc_pairs: usize,
        /// Counter-RNG seed for the discrete Monte-Carlo estimator.
        #[arg(long, default_value_t = 0)]
        mc_seed: u64,
    },
    /// An American / Bermudan early-exercise vanilla. Priced by the projected-SOR
    /// free-boundary finite difference by default (exact); pass `--lsm-paths` to
    /// price by the Longstaff-Schwartz regression Monte-Carlo (reports a standard
    /// error). `--bermudan-steps n` (n > 0) prices a Bermudan with n equally-spaced
    /// exercise dates; 0 (the default) is continuous American.
    American {
        /// Call or put.
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// Strike `K` (absolute level).
        #[arg(long)]
        strike: f64,
        /// `0` (default) ⇒ continuous American; `n > 0` ⇒ Bermudan with `n`
        /// equally-spaced exercise dates over the option life.
        #[arg(long, default_value_t = 0)]
        bermudan_steps: u32,
        /// Longstaff-Schwartz Monte-Carlo paths. `0` (default) selects the exact
        /// finite-difference engine; `> 0` selects the LSM engine (reports a
        /// standard error).
        #[arg(long, default_value_t = 0)]
        lsm_paths: usize,
        /// Sobol scramble seed for the LSM engine.
        #[arg(long, default_value_t = 0)]
        lsm_seed: u64,
    },
}

/// Arguments to `basket`.
#[derive(Debug, Args)]
pub(crate) struct BasketArgs {
    /// A leg `PAIR:WEIGHT:SPOT:VOL:R_FOR` (repeatable, at least one). Per-leg
    /// market data travels in the leg; the shared domestic rate is `--r-dom`.
    #[arg(long = "leg", value_parser = basket::parse_leg, required = true)]
    pub(crate) legs: Vec<basket::ParsedLeg>,
    /// The row-major N×N correlation matrix entries (repeatable; exactly N²
    /// values, e.g. for 2 legs: `--correlation 1 --correlation 0.4
    /// --correlation 0.4 --correlation 1`). Must be symmetric, unit-diagonal,
    /// positive-definite.
    #[arg(long = "correlation", allow_hyphen_values = true, required = true)]
    pub(crate) correlations: Vec<f64>,
    /// Call or put on the aggregated underlying.
    #[arg(long, value_enum, default_value = "call")]
    pub(crate) option: CliOptionType,
    /// The strike `K` on the aggregated underlying.
    #[arg(long)]
    pub(crate) strike: f64,
    /// The aggregation kind.
    #[arg(long, value_enum, default_value = "basket")]
    pub(crate) kind: CliBasketKind,
    /// The shared domestic (numeraire / settlement) rate `r_dom`.
    #[arg(long, default_value_t = 0.0)]
    pub(crate) r_dom: f64,
    /// The expiry in vol-time years.
    #[arg(long)]
    pub(crate) t: f64,
    /// Scrambled-Sobol points per replication (paths per scramble).
    #[arg(long, default_value_t = 16_384)]
    pub(crate) mc_paths: usize,
    /// Independent randomized scrambles (`≥ 2` for a finite standard error).
    #[arg(long, default_value_t = 24)]
    pub(crate) mc_replications: usize,
    /// Time steps per path (`1` suffices for these European payoffs).
    #[arg(long, default_value_t = 1)]
    pub(crate) mc_steps: usize,
    /// The base scramble seed (identical seeds reproduce results bit-for-bit).
    #[arg(long, default_value_t = 0)]
    pub(crate) mc_seed: u64,
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
    /// A `risk` / `stream` networked-command failure (connect, status, timeout).
    Risk(risk::RiskError),
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
            DispatchError::Risk(e) => write!(f, "{e}"),
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
                ExoticKind::Vanilla { option } => exotic::ExoticSpec::Vanilla {
                    option: option.into(),
                },
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
                ExoticKind::WindowBarrier {
                    option,
                    barrier,
                    up,
                    window_start,
                    window_end,
                    mc_pairs,
                    mc_steps,
                    mc_seed,
                } => {
                    if barrier <= 0.0 {
                        return Err(DispatchError::Invalid(
                            "window-barrier --barrier must be positive".to_owned(),
                        ));
                    }
                    if window_start < 0.0 || window_start >= window_end {
                        return Err(DispatchError::Invalid(
                            "window-barrier needs 0 <= --window-start < --window-end".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::WindowBarrier {
                        option: option.into(),
                        strike: a.strike,
                        barrier,
                        up,
                        start: window_start,
                        end: window_end,
                        mc_pairs,
                        mc_steps,
                        mc_seed,
                    }
                }
                ExoticKind::VarSwap => exotic::ExoticSpec::VarianceSwap,
                ExoticKind::VolSwap => exotic::ExoticSpec::VolatilitySwap,
                ExoticKind::Asian {
                    option,
                    continuous,
                    observations,
                    turnbull_wakeman,
                    elapsed_avg,
                    elapsed_weight,
                } => {
                    if !continuous && observations < 1 {
                        return Err(DispatchError::Invalid(
                            "discrete Asian needs --observations ≥ 1".to_owned(),
                        ));
                    }
                    if !(0.0..1.0).contains(&elapsed_weight) {
                        return Err(DispatchError::Invalid(
                            "Asian --elapsed-weight must lie in [0, 1)".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::Asian {
                        option: option.into(),
                        continuous,
                        observations,
                        turnbull_wakeman,
                        strike: a.strike,
                        elapsed_avg,
                        elapsed_weight,
                    }
                }
                ExoticKind::ForwardStart {
                    option,
                    moneyness,
                    reset,
                } => {
                    if !(reset >= 0.0 && inputs.t >= reset) {
                        return Err(DispatchError::Invalid(
                            "forward-start --reset must satisfy 0 ≤ reset ≤ expiry".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::ForwardStart {
                        option: option.into(),
                        moneyness,
                        reset,
                    }
                }
                ExoticKind::Cliquet {
                    option,
                    moneyness,
                    periods,
                    local_floor,
                    local_cap,
                    global_floor,
                    global_cap,
                    mc_pairs,
                    mc_seed,
                } => {
                    if periods < 1 {
                        return Err(DispatchError::Invalid(
                            "cliquet --periods must be ≥ 1".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::Cliquet {
                        option: option.into(),
                        moneyness,
                        periods,
                        local_floor,
                        local_cap,
                        global_floor,
                        global_cap,
                        mc_pairs,
                        mc_seed,
                    }
                }
                ExoticKind::Quanto {
                    option,
                    digital,
                    conversion_vol,
                    correlation,
                } => {
                    if !(-1.0..=1.0).contains(&correlation) {
                        return Err(DispatchError::Invalid(
                            "quanto --correlation must lie in [-1, 1]".to_owned(),
                        ));
                    }
                    if conversion_vol < 0.0 {
                        return Err(DispatchError::Invalid(
                            "quanto --conversion-vol must be non-negative".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::Quanto {
                        option: option.into(),
                        digital,
                        strike: a.strike,
                        conversion_vol,
                        correlation,
                    }
                }
                ExoticKind::Tarf {
                    option,
                    target,
                    leverage,
                    fixings,
                    fixing_notional,
                    capped_gain,
                    mc_pairs,
                    mc_seed,
                } => {
                    if target <= 0.0 {
                        return Err(DispatchError::Invalid(
                            "tarf --target must be positive".to_owned(),
                        ));
                    }
                    if fixings < 1 {
                        return Err(DispatchError::Invalid(
                            "tarf --fixings must be ≥ 1".to_owned(),
                        ));
                    }
                    if leverage < 0.0 {
                        return Err(DispatchError::Invalid(
                            "tarf --leverage must be non-negative".to_owned(),
                        ));
                    }
                    if fixing_notional <= 0.0 {
                        return Err(DispatchError::Invalid(
                            "tarf --fixing-notional must be positive".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::Tarf {
                        option: option.into(),
                        strike: a.strike,
                        target,
                        leverage,
                        fixings,
                        fixing_notional,
                        capped_gain,
                        mc_pairs,
                        mc_seed,
                    }
                }
                ExoticKind::Accumulator {
                    pivot,
                    barrier,
                    leverage,
                    fixings,
                    fixing_notional,
                    continuous,
                    mc_pairs,
                    mc_seed,
                } => {
                    if barrier <= pivot {
                        return Err(DispatchError::Invalid(
                            "accumulator --barrier must sit above --pivot".to_owned(),
                        ));
                    }
                    if fixings < 1 {
                        return Err(DispatchError::Invalid(
                            "accumulator --fixings must be ≥ 1".to_owned(),
                        ));
                    }
                    if leverage < 0.0 {
                        return Err(DispatchError::Invalid(
                            "accumulator --leverage must be non-negative".to_owned(),
                        ));
                    }
                    if fixing_notional <= 0.0 {
                        return Err(DispatchError::Invalid(
                            "accumulator --fixing-notional must be positive".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::Accumulator {
                        pivot,
                        barrier,
                        leverage,
                        fixings,
                        fixing_notional,
                        continuous,
                        mc_pairs,
                        mc_seed,
                    }
                }
                ExoticKind::Lookback {
                    option,
                    fixed,
                    discrete,
                    observations,
                    mc_pairs,
                    mc_seed,
                } => {
                    if discrete && observations < 1 {
                        return Err(DispatchError::Invalid(
                            "lookback --observations must be ≥ 1 for the discrete variant"
                                .to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::Lookback {
                        option: option.into(),
                        fixed,
                        strike: a.strike,
                        discrete,
                        observations,
                        mc_pairs,
                        mc_seed,
                    }
                }
                ExoticKind::American {
                    option,
                    strike,
                    bermudan_steps,
                    lsm_paths,
                    lsm_seed,
                } => {
                    if !(strike.is_finite() && strike > 0.0) {
                        return Err(DispatchError::Invalid(
                            "american --strike must be positive".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::American {
                        option: option.into(),
                        strike,
                        bermudan_steps,
                        lsm_paths,
                        lsm_seed,
                    }
                }
            };
            let r = match a.model {
                CliPricingModel::Analytic => {
                    if matches!(spec, exotic::ExoticSpec::WindowBarrier { .. }) {
                        return Err(DispatchError::Invalid(
                            "the window-barrier product is priced only under --model lsv \
                             (it has no closed form)"
                                .to_owned(),
                        ));
                    }
                    exotic::run(spec, &inputs)
                }
                CliPricingModel::Lsv => {
                    exotic::lsv_run(spec, &inputs).map_err(DispatchError::Invalid)?
                }
            };
            write!(out, "{}", exotic::format_report(spec, &r)).ok();
            Ok(())
        }
        Command::Basket(a) => {
            let n = a.legs.len();
            if a.correlations.len() != n * n {
                return Err(DispatchError::Invalid(format!(
                    "expected exactly {n}² = {} --correlation values for {n} legs, got {}",
                    n * n,
                    a.correlations.len()
                )));
            }
            if !(a.strike.is_finite() && a.strike > 0.0) {
                return Err(DispatchError::Invalid(
                    "basket --strike must be positive".to_owned(),
                ));
            }
            if !(a.t.is_finite() && a.t > 0.0) {
                return Err(DispatchError::Invalid(
                    "basket --t (expiry) must be positive".to_owned(),
                ));
            }
            let replications = a.mc_replications.max(2);
            let correlation: Vec<Vec<f64>> = (0..n)
                .map(|i| a.correlations[i * n..i * n + n].to_vec())
                .collect();
            let req = basket::BasketRequest {
                legs: a.legs,
                correlation,
                option: a.option.into(),
                strike: a.strike,
                kind: a.kind.into(),
                r_dom: a.r_dom,
                t: a.t,
                cfg: celnet_exotics::BasketMcConfig {
                    budget: a.mc_paths.max(1),
                    replications,
                    steps: a.mc_steps.max(1),
                    seed: a.mc_seed,
                },
            };
            let r = basket::run(&req)
                .map_err(|e| DispatchError::Invalid(format!("basket pricing failed: {e}")))?;
            write!(out, "{}", basket::format_report(&req, &r)).ok();
            Ok(())
        }
        Command::Convention(a) => {
            let pair = parse_pair(&a.pair)?;
            let tenor = parse_tenor(&a.tenor).map_err(DispatchError::BadTenor)?;
            let resolved = convention::run(pair, tenor);
            write!(out, "{}", convention::format_report(pair, tenor, &resolved)).ok();
            Ok(())
        }
        Command::Risk(a) => dispatch_risk(a, out),
        Command::Stream(a) => {
            let pair = parse_pair(&a.pair)?;
            let tenor = parse_tenor(&a.tenor).map_err(DispatchError::BadTenor)?;
            let strike = if let Some(k) = a.strike {
                StrikeSpec::Absolute(k)
            } else if let Some(d) = a.delta {
                StrikeSpec::Delta(d)
            } else {
                return Err(DispatchError::Invalid(
                    "specify exactly one of --strike or --delta".to_owned(),
                ));
            };
            let req = StreamReq {
                endpoint: a.endpoint,
                pair,
                tenor,
                expiry_years: a.expiry_years,
                option: a.option.into(),
                strike,
                notional_base: a.notional,
                ticks: a.ticks,
            };
            risk::run_stream(&req, out).map_err(DispatchError::Risk)?;
            Ok(())
        }
    }
}

/// Build the `RiskCommon` shared by the four risk sub-subcommands.
fn risk_common(a: &RiskArgs) -> RiskCommon {
    RiskCommon {
        endpoint: a.endpoint.clone(),
        numeraire_ccy: a.numeraire.clone(),
        rates: a.rates.clone(),
    }
}

/// Dispatch the `risk` subcommand to the matching `RiskService` SDK call.
fn dispatch_risk<W: Write>(a: RiskArgs, out: &mut W) -> Result<(), DispatchError> {
    let common = risk_common(&a);
    match a.kind {
        RiskKind::Aggregate {
            dimension,
            scope,
            var_shocks,
            var_alpha,
            curvature,
        } => {
            let req = AggregateReq {
                common,
                dimension: dimension.into(),
                scope: scope.map(|(d, v)| Scope::at(d, v)),
                grants: a.grants,
                denies: a.denies,
                var_shocks,
                var_alpha,
                curvature_risk_weight: curvature,
            };
            risk::run_aggregate(&req, out).map_err(DispatchError::Risk)
        }
        RiskKind::Drill {
            node,
            child_dimension,
            children,
            positions,
        } => {
            let req = DrillReq {
                common,
                node: Scope::at(node.0, node.1),
                child_dimension: child_dimension.into(),
                grants: a.grants,
                denies: a.denies,
                include_children: children,
                include_positions: positions,
            };
            risk::run_drill(&req, out).map_err(DispatchError::Risk)
        }
        RiskKind::Positions { scope } => {
            let req = PositionsReq {
                endpoint: a.endpoint,
                scope: scope.map(|(d, v)| Scope::at(d, v)),
                grants: a.grants,
                denies: a.denies,
            };
            risk::run_positions(&req, out).map_err(DispatchError::Risk)
        }
        RiskKind::Limits {
            scope,
            var_shocks,
            var_alpha,
        } => {
            let req = LimitsReq {
                common,
                scope: Scope::at(scope.0, scope.1),
                grants: a.grants,
                denies: a.denies,
                var_shocks,
                var_alpha,
            };
            risk::run_limits(&req, out).map_err(DispatchError::Risk)
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
    fn basket_parses_and_prices() {
        let out = run_to_string(&[
            "celnet",
            "basket",
            "--leg",
            "EURUSD:0.5:1.10:0.11:0.015",
            "--leg",
            "GBPUSD:0.5:1.27:0.13:0.02",
            "--correlation",
            "1",
            "--correlation",
            "0.4",
            "--correlation",
            "0.4",
            "--correlation",
            "1",
            "--option",
            "call",
            "--strike",
            "1.18",
            "--kind",
            "worst-of",
            "--r-dom",
            "0.02",
            "--t",
            "1.0",
            "--mc-paths",
            "2048",
            "--mc-replications",
            "8",
        ])
        .unwrap();
        assert!(out.contains("worst-of call"));
        assert!(out.contains("price"));
        assert!(out.contains("std_error"));
    }

    #[test]
    fn basket_rejects_wrong_correlation_count() {
        let err = run_to_string(&[
            "celnet",
            "basket",
            "--leg",
            "EURUSD:0.5:1.10:0.11:0.015",
            "--leg",
            "GBPUSD:0.5:1.27:0.13:0.02",
            "--correlation",
            "1",
            "--correlation",
            "0.4",
            "--strike",
            "1.18",
            "--t",
            "1.0",
        ])
        .unwrap_err();
        assert!(matches!(err, DispatchError::Invalid(_)));
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
