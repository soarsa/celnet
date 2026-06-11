//! The clap command tree and the dispatch from parsed arguments to the
//! subcommand core functions.
//!
//! `main` parses [`Cli`] and calls [`dispatch`], which validates / converts the
//! arguments and invokes the relevant module's `run` (the same functions the unit
//! tests assert against the underlying crates). Output is written to the supplied
//! sink so the dispatch is testable without capturing global stdout.

use std::io::Write;

use celnet_types::{AtmConvention, CcyPair, Tenor};
use clap::{Args, Parser, Subcommand};

use crate::args::{
    CliAsset, CliBarrier, CliDeltaConvention, CliDigital, CliOptionType, CliSettlementStyle, Market,
};
use crate::basket::{self, CliBasketKind};
use crate::risk::{
    self, AggregateReq, CliDimension, DrillReq, LimitsReq, PositionsReq, RiskCommon, StreamReq,
};
use crate::tenor::parse_tenor;
use crate::{convention, exotic, future_option, linear, perpetual, price, rfq, surface};

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
    /// Price an FX outright forward (deliverable): a closed-form discounted
    /// cashflow at a contract rate, plus its exact linear Greeks.
    Forward(ForwardArgs),
    /// Price an FX swap (near leg spot-settling + far leg at the tenor, opposite
    /// sides): the net PV of the two outright forwards, deliverable underlying.
    Swap(SwapCmdArgs),
    /// Price a non-deliverable forward (NDF): cash-settled in the convertible
    /// currency at a named fixing; non-deliverable underlying only.
    Ndf(NdfArgs),
    /// Price a perpetual (no-expiry) American option: exercisable at any time,
    /// no terminal date — the exact stationary closed form (free boundary by
    /// value matching + smooth pasting) with fully analytic Greeks. Takes no
    /// expiry/tenor: a perpetual has none.
    Perpetual(PerpetualArgs),
    /// Price an option on a listed future (any asset class): the quoted futures
    /// price already embodies the underlying's carry, so the value is the exact
    /// futures-measure closed form under the chosen premium margining
    /// convention (equity-style discounted / futures-style undiscounted).
    FutureOption(FutureOptionArgs),
    /// Resolve and print the convention record for a pair and tenor.
    Convention(ConventionArgs),
    /// Firm-scale hierarchical risk against a running edge (the same `RiskService`
    /// the GUI Book view and Excel `CELNET.*` consume, via the `celnet-client` SDK).
    Risk(RiskArgs),
    /// Subscribe to a two-way RFS stream for an instrument against a running edge,
    /// print the sequenced ticks, then unsubscribe cleanly (the `celnet-client`
    /// multiplexed session — the same stream the GUI blotter consumes).
    Stream(StreamArgs),
    /// Fan one RFQ across a running edge's multi-dealer LP panel via the
    /// `celnet-client` SDK and print the ranked dealer ladder (lp_id, firm
    /// bid/offer, last-look countdown, BEST_BID/BEST_OFFER markers);
    /// `--accept <LP_ID>` then books that pinned row and prints the execution.
    /// In-repo panels are the native maker plus labeled deterministic synthetic
    /// demo/test dealers — live LP connectivity is an environment concern.
    Rfq(RfqArgs),
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
    /// Tenor shorthand, e.g. 1Y, 3M, ON — the instrument's pillar label. The
    /// priced expiry derives from it on the pair's conventions calendar unless
    /// `--expiry-years` pins it explicitly.
    #[arg(long)]
    pub(crate) tenor: String,
    /// Time to expiry in years (authoritative for pricing). Omitted ⇒ derived
    /// from `--tenor` by the conventions resolver, anchored at today; given ⇒
    /// must agree with the tenor label (a contradictory pair is an error — the
    /// label and the priced expiry never drift apart silently).
    #[arg(long)]
    pub(crate) expiry_years: Option<f64>,
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

/// Arguments to `rfq`.
#[derive(Debug, Args)]
pub(crate) struct RfqArgs {
    /// The gRPC endpoint of the edge.
    #[arg(long, default_value = "http://127.0.0.1:50551")]
    pub(crate) endpoint: String,
    /// Currency pair, e.g. EURUSD.
    #[arg(long)]
    pub(crate) pair: String,
    /// Tenor shorthand, e.g. 1Y, 3M, ON — the instrument's pillar label. The
    /// priced expiry derives from it on the pair's conventions calendar unless
    /// `--expiry-years` pins it explicitly.
    #[arg(long)]
    pub(crate) tenor: String,
    /// Time to expiry in years (authoritative for pricing). Omitted ⇒ derived
    /// from `--tenor` by the conventions resolver, anchored at today; given ⇒
    /// must agree with the tenor label (a contradictory pair is an error — the
    /// label and the priced expiry never drift apart silently).
    #[arg(long)]
    pub(crate) expiry_years: Option<f64>,
    /// Call or put.
    #[arg(long, value_enum, default_value = "call")]
    pub(crate) option: CliOptionType,
    /// Explicit strike (mutually exclusive with `--delta`).
    #[arg(long, group = "rfq_strike")]
    pub(crate) strike: Option<f64>,
    /// A signed convention delta resolved to a strike server-side.
    #[arg(long, group = "rfq_strike", allow_hyphen_values = true)]
    pub(crate) delta: Option<f64>,
    /// Base-currency notional.
    #[arg(long, default_value_t = 1_000_000.0)]
    pub(crate) notional: f64,
    /// Book the named panel row (an `lp_id` from the printed ladder) after the
    /// panel prints; omit to print the ranked panel only.
    #[arg(long, value_name = "LP_ID")]
    pub(crate) accept: Option<String>,
    /// The accept direction: buy lifts the chosen row's offer, sell hits its bid
    /// (used only with `--accept`).
    #[arg(long, value_enum, default_value = "buy")]
    pub(crate) side: rfq::CliAcceptSide,
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
    /// The underlying asset class. One unversioned contract names every class: a
    /// vanilla on any class prices by the same asset-class-agnostic
    /// generalized-BSM / Garman-Kohlhagen closed form over the carry-producing
    /// market (ADR-0008), where `--r-for` is the asset's carry yield (FX foreign
    /// rate / equity dividend yield / commodity cost-of-carry / crypto funding).
    #[arg(long, value_enum, default_value = "fx")]
    pub(crate) asset: CliAsset,
    /// The underlying identifier for the chosen `--asset`: a pair (`EURUSD`,
    /// `BTCUSDT`) or a symbol (`AAPL`, `BRENT`). Labels the report; defaults per
    /// class when omitted.
    #[arg(long)]
    pub(crate) underlying: Option<String>,
    /// The contract settlement mechanics: `linear` (quote-margined, the default)
    /// or `inverse-coin` (the digital-asset `1/S_T` convention; valid only with
    /// `--asset crypto`).
    #[arg(long, value_enum, default_value = "linear")]
    pub(crate) settlement_style: CliSettlementStyle,
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
    /// Strike `K` (absolute level). REQUIRED by the struck families — vanilla,
    /// digital, barrier, window-barrier, asian, quanto, tarf, pivot, american,
    /// and the fixed-strike lookback (`--fixed`): a missing strike is a typed
    /// error, never a degenerate K = 0 price. The unstruck families (touches,
    /// var/vol swaps, forward-start, cliquet, accumulator, floating lookback)
    /// take no strike and ignore the flag.
    #[arg(long)]
    pub(crate) strike: Option<f64>,
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
    /// A pivot Target-Redemption Accumulator (Monte-Carlo; reports a standard
    /// error): the TARF mechanic with a distinct pivot kink — the leg is
    /// selected by `--pivot`, valued by the common `exotic --strike`;
    /// `--pivot == --strike` is the exact TARF slice.
    Pivot {
        /// The favourable-side direction (put = client gains when S < strike).
        #[arg(long, value_enum)]
        option: CliOptionType,
        /// The pivot level at which the geared adverse leg engages.
        #[arg(long)]
        pivot: f64,
        /// The cumulative gain target; reaching it redeems (knocks out).
        #[arg(long)]
        target: f64,
        /// Gearing/leverage on the adverse leg (the far side of the pivot).
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
    /// An American / Bermudan early-exercise vanilla, struck at the common
    /// `exotic --strike`. Priced by the projected-SOR free-boundary finite
    /// difference by default (exact); pass `--lsm-paths` to price by the
    /// Longstaff-Schwartz regression Monte-Carlo (reports a standard error).
    /// `--bermudan-steps n` (n > 0) prices a Bermudan with n equally-spaced
    /// exercise dates; 0 (the default) is continuous American.
    American {
        /// Call or put.
        #[arg(long, value_enum)]
        option: CliOptionType,
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

/// Shared market flags for the linear book (`forward` / `swap` / `ndf`): spot, the
/// settlement / far-leg tenor in years, and the two carry rates. A linear product
/// has no volatility input (it is a discounted cashflow, not an option).
#[derive(Debug, Args)]
pub(crate) struct LinearMarketArgs {
    /// Spot FX rate (quote per 1 unit of base).
    #[arg(long)]
    pub(crate) spot: f64,
    /// Time to settlement in years (the far-leg tenor for a swap; authoritative).
    #[arg(long)]
    pub(crate) t: f64,
    /// Continuously-compounded domestic (quote) rate.
    #[arg(long)]
    pub(crate) r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    #[arg(long)]
    pub(crate) r_for: f64,
}

impl LinearMarketArgs {
    fn to_market(&self) -> linear::LinearMarket {
        linear::LinearMarket {
            spot: self.spot,
            t: self.t,
            r_dom: self.r_dom,
            r_for: self.r_for,
        }
    }
}

/// Arguments to `forward`.
#[derive(Debug, Args)]
pub(crate) struct ForwardArgs {
    /// Currency pair, e.g. EURUSD (must be deliverable).
    #[arg(long)]
    pub(crate) pair: String,
    /// The contract (delivery) rate `K`.
    #[arg(long)]
    pub(crate) rate: f64,
    /// The notional (always positive; direction is `--side`).
    #[arg(long, default_value_t = 1.0)]
    pub(crate) notional: f64,
    /// Buy (long the base forward) or sell.
    #[arg(long, value_enum, default_value = "buy")]
    pub(crate) side: linear::CliSide,
    /// Shared linear market inputs.
    #[command(flatten)]
    pub(crate) market: LinearMarketArgs,
}

/// Arguments to `swap`.
#[derive(Debug, Args)]
pub(crate) struct SwapCmdArgs {
    /// Currency pair, e.g. EURUSD (must be deliverable).
    #[arg(long)]
    pub(crate) pair: String,
    /// The near leg's contract rate `K` (drives both legs).
    #[arg(long)]
    pub(crate) rate: f64,
    /// The near leg's notional.
    #[arg(long, default_value_t = 1.0)]
    pub(crate) notional: f64,
    /// The near leg's side (the far leg trades the opposite side).
    #[arg(long, value_enum, default_value = "buy")]
    pub(crate) near_side: linear::CliSide,
    /// Shared linear market inputs (`--t` is the far-leg tenor; the near leg
    /// settles at the spot date).
    #[command(flatten)]
    pub(crate) market: LinearMarketArgs,
}

/// Arguments to `ndf`.
#[derive(Debug, Args)]
pub(crate) struct NdfArgs {
    /// Currency pair, e.g. USDBRL (must be non-deliverable).
    #[arg(long)]
    pub(crate) pair: String,
    /// The contract (forward) rate `K`.
    #[arg(long)]
    pub(crate) rate: f64,
    /// The notional (always positive; direction is `--side`).
    #[arg(long, default_value_t = 1.0)]
    pub(crate) notional: f64,
    /// Buy (long the base forward) or sell.
    #[arg(long, value_enum, default_value = "buy")]
    pub(crate) side: linear::CliSide,
    /// The published settlement-rate fixing the contract references (identity
    /// only — the live fixing value is never sourced in-repo).
    #[arg(long, value_enum)]
    pub(crate) fixing: linear::CliFixing,
    /// Shared linear market inputs.
    #[command(flatten)]
    pub(crate) market: LinearMarketArgs,
}

/// Arguments to `perpetual`. There is deliberately no `--t`/`--tenor`: a
/// perpetual option has no expiry (the wire contract encodes the no-expiry
/// shape as `expiry_years = 0`), and its value is time-homogeneous.
#[derive(Debug, Args)]
pub(crate) struct PerpetualArgs {
    /// Call or put.
    #[arg(long, value_enum)]
    pub(crate) option: CliOptionType,
    /// Strike `K` (absolute level, quote per 1 unit of base/asset).
    #[arg(long)]
    pub(crate) strike: f64,
    /// Spot price (quote per 1 unit of base/asset).
    #[arg(long)]
    pub(crate) spot: f64,
    /// Annualized volatility (absolute, e.g. 0.10 = 10 vol).
    #[arg(long)]
    pub(crate) vol: f64,
    /// Continuously-compounded domestic / discount rate (must be ≥ 0: a
    /// perpetual claim has no finite value under a negative discount rate —
    /// parsed signed so the rejection is the domain message, never a flag-parse
    /// error).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) r_dom: f64,
    /// Continuously-compounded carry yield of the asset (FX foreign rate /
    /// equity dividend yield / commodity cost-of-carry / crypto funding) — a
    /// signed quantity (negative yields are real markets).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) r_for: f64,
    /// The underlying asset class: FX prices over the two-rate carry, every
    /// other class over the generalized cost-of-carry seam with
    /// `b = r_dom − r_for` (ADR-0008) — the same value, carry-tagged rhos.
    #[arg(long, value_enum, default_value = "fx")]
    pub(crate) asset: CliAsset,
    /// The underlying identifier for the chosen `--asset` (labels the report;
    /// defaults per class when omitted).
    #[arg(long)]
    pub(crate) underlying: Option<String>,
}

/// Arguments to `future-option`.
#[derive(Debug, Args)]
pub(crate) struct FutureOptionArgs {
    /// Call or put on the future.
    #[arg(long, value_enum)]
    pub(crate) option: CliOptionType,
    /// The quoted futures price `F` (the complete carry-bearing market input —
    /// the future already embodies the underlying's carry, any asset class).
    #[arg(long)]
    pub(crate) future: f64,
    /// Strike `K` (absolute level, in the future's quote units).
    #[arg(long)]
    pub(crate) strike: f64,
    /// Annualized volatility of the future (absolute, e.g. 0.28 = 28 vol).
    #[arg(long)]
    pub(crate) vol: f64,
    /// The OPTION's time to expiry in years.
    #[arg(long)]
    pub(crate) t: f64,
    /// The FUTURE's own expiry in years (must be ≥ `--t`: the future outlives
    /// the option). Booked term — the quoted future already prices the carry.
    #[arg(long)]
    pub(crate) future_expiry: f64,
    /// Continuously-compounded discount (settlement-currency) rate — a signed
    /// quantity (negative rates are real markets; futures-style margining never
    /// discounts at all).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) r_dom: f64,
    /// The premium margining convention: equity-style (premium-upfront,
    /// discounted) or futures-style (daily-margined, undiscounted).
    #[arg(long, value_enum, default_value = "equity-style")]
    pub(crate) margining: future_option::CliMargining,
    /// The listed future contract's ticker (identity; labels the report).
    #[arg(long)]
    pub(crate) symbol: String,
    /// The listing venue MIC of the contract (identity; labels the report).
    #[arg(long, default_value = "")]
    pub(crate) venue: String,
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
    /// A `risk` / `stream` / `rfq` networked-command failure (connect, status,
    /// timeout).
    Risk(risk::RiskError),
    /// A `forward` / `swap` / `ndf` linear-product failure (bad input or a
    /// product × underlying validity-matrix violation).
    Linear(linear::LinearError),
    /// A struck exotic family was requested without the required `--strike`
    /// (a missing strike must never price a degenerate K = 0 contract).
    MissingStrike {
        /// The struck family that refused to price.
        family: &'static str,
    },
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
            DispatchError::Linear(e) => write!(f, "{e}"),
            DispatchError::MissingStrike { family } => write!(
                f,
                "the {family} exotic requires --strike: it is struck at an absolute \
                 level and has no default"
            ),
            DispatchError::Invalid(s) => write!(f, "invalid argument: {s}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Convert a pair string into a [`CcyPair`], or a [`DispatchError::BadPair`].
fn parse_pair(s: &str) -> Result<CcyPair, DispatchError> {
    CcyPair::parse(s).ok_or_else(|| DispatchError::BadPair(s.to_owned()))
}

/// The per-asset-class default underlying label when `--underlying` is omitted.
fn default_underlying(asset: CliAsset) -> String {
    match asset {
        CliAsset::Fx => "EURUSD",
        CliAsset::Equity => "EQUITY",
        CliAsset::Commodity => "COMMODITY",
        CliAsset::Crypto => "BTCUSDT",
    }
    .to_owned()
}

/// Today's civil date (UTC) — the horizon the live networked subcommands anchor
/// tenor resolution at (a streamed `3M` means three months from now).
fn today_utc() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

/// The absolute slack when reconciling an explicit `--expiry-years` with its
/// `--tenor` label: two calendar days of vol-time, covering the spot-lag /
/// business-day-roll wobble at the very short end (an ON expiry over a weekend).
const TENOR_EXPIRY_ABS_TOL_YEARS: f64 = 2.0 / 365.0;

/// The relative slack for the same reconciliation: 5% — wider than any
/// day-count / settlement-roll gap between a pillar's nominal and
/// calendar-resolved expiry, yet under the ~8% spacing of adjacent monthly
/// pillars, so a wrong-pillar pair always errs.
const TENOR_EXPIRY_REL_TOL: f64 = 0.05;

/// The flat (calendar-free) year fraction a tenor pillar nominally spans — the
/// spelling flat-time callers quote expiries in (e.g. `1.0` for 1Y). `None` for
/// a broken date, whose only year fraction is the calendar-resolved one.
fn nominal_tenor_years(tenor: Tenor) -> Option<f64> {
    match tenor {
        Tenor::Overnight => Some(1.0 / 365.0),
        Tenor::TomNext => Some(2.0 / 365.0),
        Tenor::SpotNext => Some(3.0 / 365.0),
        Tenor::Weeks(n) => Some(f64::from(n) * 7.0 / 365.0),
        Tenor::Months(n) => Some(f64::from(n) / 12.0),
        Tenor::Years(n) => Some(f64::from(n)),
        // The n-th quarterly IMM sits ≈ n quarters out (nominal only; the
        // calendar-resolved route carries the exact IMM Wednesday).
        Tenor::Imm(n) => Some(f64::from(n) * 0.25),
        Tenor::BrokenDate(_) => None,
    }
}

/// The priced expiry for a dated networked instrument (`stream` / `rfq`).
///
/// With only `--tenor` given the expiry derives from the tenor on the pair's
/// conventions calendar ([`celnet_conventions::vol_year_fraction`] — the same
/// resolver the market-data normalization price path uses), anchored at
/// `horizon`. An explicit `--expiry-years` is authoritative when it agrees with
/// the tenor label — within two calendar days plus 5% of either the
/// calendar-resolved or the nominal pillar time (both spellings are real:
/// calendar-exact and flat-time clients). A contradictory pair is a typed
/// error: the label and the priced expiry never drift apart silently.
fn resolve_priced_expiry(
    pair: CcyPair,
    tenor: Tenor,
    explicit: Option<f64>,
    horizon: time::Date,
) -> Result<f64, DispatchError> {
    let derived = celnet_conventions::vol_year_fraction(pair, horizon, tenor).map_err(|e| {
        DispatchError::Invalid(format!(
            "--tenor {} does not resolve to an expiry on the {pair} conventions calendar: {e}",
            crate::tenor::format_tenor(tenor)
        ))
    })?;
    let Some(explicit) = explicit else {
        return Ok(derived);
    };
    if !(explicit.is_finite() && explicit > 0.0) {
        return Err(DispatchError::Invalid(
            "--expiry-years must be finite and positive".to_owned(),
        ));
    }
    let agrees = |anchor: f64| {
        (explicit - anchor).abs() <= TENOR_EXPIRY_ABS_TOL_YEARS + TENOR_EXPIRY_REL_TOL * anchor
    };
    if agrees(derived) || nominal_tenor_years(tenor).is_some_and(agrees) {
        return Ok(explicit);
    }
    Err(DispatchError::Invalid(format!(
        "--expiry-years {explicit} contradicts --tenor {} (≈{derived:.4}y on the {pair} \
         conventions calendar): pass a consistent pair, or omit --expiry-years to derive \
         the expiry from the tenor",
        crate::tenor::format_tenor(tenor)
    )))
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
            // The inverse coin-margined settlement convention is meaningful only for
            // a digital-asset underlying — reject it for any other class loudly,
            // rather than silently pricing the linear payoff under a wrong label.
            if matches!(a.settlement_style, CliSettlementStyle::InverseCoin)
                && !matches!(a.asset, CliAsset::Crypto)
            {
                return Err(DispatchError::Invalid(
                    "--settlement-style inverse-coin is valid only with --asset crypto".to_owned(),
                ));
            }
            let r = price::run(
                option,
                spec,
                a.market.to_market(),
                a.delta_convention.into(),
            )
            .map_err(DispatchError::Price)?;
            let label = a.underlying.unwrap_or_else(|| default_underlying(a.asset));
            write!(
                out,
                "{}",
                price::format_report_for(option, a.asset, &label, a.settlement_style.into(), &r)
            )
            .ok();
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
            // The struck families refuse to price without an explicit strike —
            // a missing `--strike` is a typed error naming the family, never a
            // silently-priced degenerate K = 0 contract. The unstruck families
            // never read the strike (their engines ignore the field), so the
            // shared inputs carry 0.0 for them, exactly as before.
            let strike_flag = a.strike;
            let strike_for = move |family: &'static str| {
                strike_flag.ok_or(DispatchError::MissingStrike { family })
            };
            let inputs = a.market.to_market().inputs(a.strike.unwrap_or(0.0));
            let spec = match a.kind {
                ExoticKind::Vanilla { option } => {
                    // Struck at `inputs.strike`.
                    strike_for("vanilla")?;
                    exotic::ExoticSpec::Vanilla {
                        option: option.into(),
                    }
                }
                ExoticKind::Digital { kind } => {
                    // The cash-or-nothing payout is struck at `inputs.strike`.
                    strike_for("digital")?;
                    exotic::ExoticSpec::Digital(kind.into())
                }
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
                    strike: strike_for("barrier")?,
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
                        strike: strike_for("window-barrier")?,
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
                        strike: strike_for("asian")?,
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
                        strike: strike_for("quanto")?,
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
                        strike: strike_for("tarf")?,
                        target,
                        leverage,
                        fixings,
                        fixing_notional,
                        capped_gain,
                        mc_pairs,
                        mc_seed,
                    }
                }
                ExoticKind::Pivot {
                    option,
                    pivot,
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
                            "pivot --target must be positive".to_owned(),
                        ));
                    }
                    if pivot <= 0.0 {
                        return Err(DispatchError::Invalid(
                            "pivot --pivot must be positive".to_owned(),
                        ));
                    }
                    if fixings < 1 {
                        return Err(DispatchError::Invalid(
                            "pivot --fixings must be ≥ 1".to_owned(),
                        ));
                    }
                    if leverage < 0.0 {
                        return Err(DispatchError::Invalid(
                            "pivot --leverage must be non-negative".to_owned(),
                        ));
                    }
                    if fixing_notional <= 0.0 {
                        return Err(DispatchError::Invalid(
                            "pivot --fixing-notional must be positive".to_owned(),
                        ));
                    }
                    exotic::ExoticSpec::Pivot {
                        option: option.into(),
                        strike: strike_for("pivot")?,
                        pivot,
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
                    // Only the fixed-strike family is struck; the floating
                    // lookback's strike IS the realised extremum, so its
                    // engines never read this field.
                    let strike = if fixed {
                        strike_for("fixed-strike lookback")?
                    } else {
                        strike_flag.unwrap_or(0.0)
                    };
                    exotic::ExoticSpec::Lookback {
                        option: option.into(),
                        fixed,
                        strike,
                        discrete,
                        observations,
                        mc_pairs,
                        mc_seed,
                    }
                }
                ExoticKind::American {
                    option,
                    bermudan_steps,
                    lsm_paths,
                    lsm_seed,
                } => {
                    let strike = strike_for("american")?;
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
        Command::Forward(a) => {
            let pair = parse_pair(&a.pair)?;
            let r = linear::run_forward(pair, a.market.to_market(), a.rate, a.notional, a.side)
                .map_err(DispatchError::Linear)?;
            write!(out, "{}", linear::format_report("fx-forward", &r)).ok();
            Ok(())
        }
        Command::Swap(a) => {
            let pair = parse_pair(&a.pair)?;
            let r = linear::run_swap(pair, a.market.to_market(), a.rate, a.notional, a.near_side)
                .map_err(DispatchError::Linear)?;
            write!(out, "{}", linear::format_report("fx-swap", &r)).ok();
            Ok(())
        }
        Command::Ndf(a) => {
            let pair = parse_pair(&a.pair)?;
            let r = linear::run_ndf(
                pair,
                a.market.to_market(),
                a.rate,
                a.notional,
                a.side,
                a.fixing,
            )
            .map_err(DispatchError::Linear)?;
            write!(out, "{}", linear::format_report("ndf", &r)).ok();
            Ok(())
        }
        Command::Perpetual(a) => {
            if !(a.strike.is_finite() && a.strike > 0.0) {
                return Err(DispatchError::Invalid(
                    "perpetual --strike must be positive".to_owned(),
                ));
            }
            if !(a.spot.is_finite() && a.spot > 0.0) {
                return Err(DispatchError::Invalid(
                    "perpetual --spot must be positive".to_owned(),
                ));
            }
            if !(a.vol.is_finite() && a.vol > 0.0) {
                return Err(DispatchError::Invalid(
                    "perpetual --vol must be positive".to_owned(),
                ));
            }
            // The leaf's documented domain (mirrors the server's refusal): a
            // perpetual claim under a negative discount rate has no finite value
            // — rejected loudly (a NaN rate included), never priced to a NaN.
            if a.r_dom.is_nan() || a.r_dom < 0.0 {
                return Err(DispatchError::Invalid(
                    "a perpetual option has no finite value under a negative discount rate \
                     (--r-dom must be ≥ 0)"
                        .to_owned(),
                ));
            }
            let market = perpetual::PerpetualMarket {
                spot: a.spot,
                vol: a.vol,
                r_dom: a.r_dom,
                r_for: a.r_for,
            };
            // The leaf's carry-domain law (typed): a perpetual CALL with carry
            // strictly exceeding the discount rate (b > r; FX form r_for < 0)
            // has no finite value — refused exactly as the server refuses it.
            let r = perpetual::run(a.option.into(), a.strike, market, a.asset)
                .map_err(|e| DispatchError::Invalid(e.to_string()))?;
            let label = a.underlying.unwrap_or_else(|| default_underlying(a.asset));
            write!(
                out,
                "{}",
                perpetual::format_report(a.option.into(), a.asset, &label, a.strike, &r)
            )
            .ok();
            Ok(())
        }
        Command::FutureOption(a) => {
            if !(a.strike.is_finite() && a.strike > 0.0) {
                return Err(DispatchError::Invalid(
                    "future-option --strike must be positive".to_owned(),
                ));
            }
            if !(a.future.is_finite() && a.future > 0.0) {
                return Err(DispatchError::Invalid(
                    "future-option --future must be positive".to_owned(),
                ));
            }
            if !(a.vol.is_finite() && a.vol > 0.0) {
                return Err(DispatchError::Invalid(
                    "future-option --vol must be positive".to_owned(),
                ));
            }
            if !(a.t.is_finite() && a.t > 0.0) {
                return Err(DispatchError::Invalid(
                    "future-option --t must be finite and > 0".to_owned(),
                ));
            }
            // The contract's term ordering (mirrors the wire validator): the
            // future must outlive the option — never clamped.
            if !(a.future_expiry.is_finite() && a.future_expiry >= a.t) {
                return Err(DispatchError::Invalid(
                    "future-option --future-expiry must be finite and ≥ --t (the future must \
                     outlive the option)"
                        .to_owned(),
                ));
            }
            if a.symbol.is_empty() {
                return Err(DispatchError::Invalid(
                    "future-option --symbol must name the listed contract".to_owned(),
                ));
            }
            let market = future_option::FutureOptionMarket {
                future: a.future,
                vol: a.vol,
                t: a.t,
                r_dom: a.r_dom,
            };
            let g = future_option::run(a.option.into(), a.strike, market, a.margining);
            write!(
                out,
                "{}",
                future_option::format_report(
                    a.option.into(),
                    &a.symbol,
                    &a.venue,
                    a.future_expiry,
                    a.margining,
                    a.strike,
                    &g,
                )
            )
            .ok();
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
            let expiry_years = resolve_priced_expiry(pair, tenor, a.expiry_years, today_utc())?;
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
                expiry_years,
                option: a.option.into(),
                strike,
                notional_base: a.notional,
                ticks: a.ticks,
            };
            risk::run_stream(&req, out).map_err(DispatchError::Risk)?;
            Ok(())
        }
        Command::Rfq(a) => {
            let pair = parse_pair(&a.pair)?;
            let tenor = parse_tenor(&a.tenor).map_err(DispatchError::BadTenor)?;
            let expiry_years = resolve_priced_expiry(pair, tenor, a.expiry_years, today_utc())?;
            let strike = if let Some(k) = a.strike {
                StrikeSpec::Absolute(k)
            } else if let Some(d) = a.delta {
                StrikeSpec::Delta(d)
            } else {
                return Err(DispatchError::Invalid(
                    "specify exactly one of --strike or --delta".to_owned(),
                ));
            };
            let req = rfq::RfqReq {
                endpoint: a.endpoint,
                pair,
                tenor,
                expiry_years,
                option: a.option.into(),
                strike,
                notional_base: a.notional,
                accept: a.accept,
                side: a.side.into(),
            };
            rfq::run(&req, out).map_err(DispatchError::Risk)?;
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

    #[test]
    fn perpetual_parses_and_prices() {
        let out = run_to_string(&[
            "celnet",
            "perpetual",
            "--option",
            "call",
            "--strike",
            "1.10",
            "--spot",
            "1.10",
            "--vol",
            "0.105",
            "--r-dom",
            "0.05",
            "--r-for",
            "0.01",
        ])
        .unwrap();
        assert!(out.contains("perpetual call"));
        assert!(out.contains("price"));
        assert!(out.contains("exercise_boundary"));
        assert!(out.contains("rho_dom"));
    }

    #[test]
    fn perpetual_takes_no_expiry_flag() {
        // A perpetual has no expiry: `--t` must be a structural parse error,
        // never a silently-ignored flag.
        assert!(
            Cli::try_parse_from([
                "celnet",
                "perpetual",
                "--option",
                "call",
                "--strike",
                "1.10",
                "--spot",
                "1.10",
                "--vol",
                "0.105",
                "--r-dom",
                "0.05",
                "--r-for",
                "0.01",
                "--t",
                "1.0",
            ])
            .is_err()
        );
    }

    #[test]
    fn perpetual_rejects_a_negative_discount_rate() {
        let err = run_to_string(&[
            "celnet",
            "perpetual",
            "--option",
            "put",
            "--strike",
            "1.10",
            "--spot",
            "1.10",
            "--vol",
            "0.105",
            "--r-dom=-0.01",
            "--r-for",
            "0.01",
        ])
        .unwrap_err();
        assert!(matches!(err, DispatchError::Invalid(_)));
    }

    /// The leaf's carry-domain law through the full dispatch: an FX perpetual
    /// CALL with `r_for < 0` (`b = r_dom − r_for > r_dom` strictly) has no
    /// finite value — a typed refusal carrying the leaf's message, never a
    /// number. The PUT on the identical market still prices (the `y₂` branch).
    #[test]
    fn perpetual_rejects_a_call_with_carry_exceeding_discount() {
        let args = |option: &'static str| {
            [
                "celnet",
                "perpetual",
                "--option",
                option,
                "--strike",
                "1.10",
                "--spot",
                "1.25",
                "--vol",
                "0.10",
                "--r-dom",
                "0.02",
                "--r-for=-0.005",
            ]
        };
        let err = run_to_string(&args("call")).unwrap_err();
        match err {
            DispatchError::Invalid(msg) => assert_eq!(
                msg,
                "a perpetual call with carry exceeding the discount rate has no finite value"
            ),
            other => panic!("expected the typed Invalid refusal, got {other:?}"),
        }
        let out = run_to_string(&args("put")).unwrap();
        assert!(out.contains("perpetual put") && out.contains("price"));
    }

    #[test]
    fn future_option_parses_and_prices() {
        let out = run_to_string(&[
            "celnet",
            "future-option",
            "--option",
            "call",
            "--future",
            "19.0",
            "--strike",
            "19.0",
            "--vol",
            "0.28",
            "--t",
            "0.75",
            "--future-expiry",
            "0.75",
            "--r-dom",
            "0.10",
            "--margining",
            "equity-style",
            "--symbol",
            "CL",
            "--venue",
            "XNYM",
        ])
        .unwrap();
        assert!(out.contains("future-option call"));
        assert!(out.contains("contract        CL@XNYM"));
        assert!(out.contains("margining       equity-style"));
        assert!(out.contains("price"));
    }

    #[test]
    fn future_option_rejects_a_future_expiring_before_the_option() {
        let err = run_to_string(&[
            "celnet",
            "future-option",
            "--option",
            "put",
            "--future",
            "45.0",
            "--strike",
            "40.0",
            "--vol",
            "0.35",
            "--t",
            "0.75",
            "--future-expiry",
            "0.5",
            "--r-dom",
            "0.04",
            "--symbol",
            "ES",
        ])
        .unwrap_err();
        assert!(matches!(err, DispatchError::Invalid(_)));
    }

    #[test]
    fn rfq_requires_a_strike_spec() {
        // Rejected in dispatch before any network round-trip.
        let err =
            run_to_string(&["celnet", "rfq", "--pair", "EURUSD", "--tenor", "1Y"]).unwrap_err();
        assert!(matches!(err, DispatchError::Invalid(_)));
    }

    #[test]
    fn rfq_strike_and_delta_are_mutually_exclusive() {
        // Clap's own arg-group rejection (structural, pre-dispatch).
        assert!(
            Cli::try_parse_from([
                "celnet", "rfq", "--pair", "EURUSD", "--tenor", "1Y", "--strike", "1.12",
                "--delta", "0.25",
            ])
            .is_err()
        );
    }

    #[test]
    fn stream_and_rfq_require_a_tenor() {
        // The pillar label is the instrument identity: no default, no drift.
        for cmd in ["stream", "rfq"] {
            assert!(
                Cli::try_parse_from(["celnet", cmd, "--pair", "EURUSD", "--strike", "1.12"])
                    .is_err(),
                "{cmd} without --tenor must be a structural parse error"
            );
        }
    }

    // ---- the tenor → priced-expiry resolution (`stream` / `rfq`) ------------

    /// A pinned horizon (a Wednesday) so the resolution tests are deterministic
    /// regardless of when they run.
    fn pinned_wednesday() -> time::Date {
        time::macros::date!(2026 - 06 - 10)
    }

    #[test]
    fn priced_expiry_derives_from_tenor_via_the_conventions_calendar() {
        let pair = CcyPair::parse("EURUSD").unwrap();
        let horizon = pinned_wednesday();
        let derived = resolve_priced_expiry(pair, Tenor::Months(3), None, horizon).unwrap();
        let direct =
            celnet_conventions::vol_year_fraction(pair, horizon, Tenor::Months(3)).unwrap();
        assert_eq!(derived.to_bits(), direct.to_bits());
        // A 3M label prices ≈ a quarter out — never the old defaulted 1Y.
        assert!((0.2..0.35).contains(&derived), "3M derived {derived}");
    }

    #[test]
    fn priced_expiry_accepts_a_consistent_explicit_pair_verbatim() {
        let pair = CcyPair::parse("EURUSD").unwrap();
        // The flat-time spelling of the pillar: authoritative when consistent.
        let t =
            resolve_priced_expiry(pair, Tenor::Years(1), Some(1.0), pinned_wednesday()).unwrap();
        assert_eq!(t.to_bits(), 1.0_f64.to_bits());
        // The short end over a weekend: ON from a Friday resolves to Monday
        // (≈ 3/365) on the calendar, while the nominal pillar time is 1/365 —
        // both spellings are accepted, neither is a contradiction.
        let friday = time::macros::date!(2026 - 06 - 12);
        let nominal = 1.0 / 365.0;
        let t = resolve_priced_expiry(pair, Tenor::Overnight, Some(nominal), friday).unwrap();
        assert_eq!(t.to_bits(), nominal.to_bits());
    }

    #[test]
    fn priced_expiry_rejects_a_contradictory_or_out_of_domain_pair() {
        let pair = CcyPair::parse("EURUSD").unwrap();
        let horizon = pinned_wednesday();
        // The defect shape: a 3M label on a 1Y-priced expiry.
        let err = resolve_priced_expiry(pair, Tenor::Months(3), Some(1.0), horizon).unwrap_err();
        match err {
            DispatchError::Invalid(msg) => {
                assert!(msg.contains("contradicts"), "actionable message: {msg}");
            }
            other => panic!("expected the typed contradiction, got {other:?}"),
        }
        // Adjacent short pillars never conflate: a 2W expiry on a 1W label errs.
        assert!(resolve_priced_expiry(pair, Tenor::Weeks(1), Some(14.0 / 365.0), horizon).is_err());
        // Out-of-domain explicit expiries are typed refusals, never priced.
        for bad in [0.0, -1.0, f64::NAN] {
            assert!(resolve_priced_expiry(pair, Tenor::Years(1), Some(bad), horizon).is_err());
        }
    }

    #[test]
    fn stream_and_rfq_reject_a_contradictory_tenor_expiry_pair() {
        // Through the full dispatch: rejected before any network round-trip.
        for cmd in ["stream", "rfq"] {
            let err = run_to_string(&[
                "celnet",
                cmd,
                "--pair",
                "EURUSD",
                "--tenor",
                "3M",
                "--expiry-years",
                "1.0",
                "--strike",
                "1.12",
            ])
            .unwrap_err();
            match err {
                DispatchError::Invalid(msg) => {
                    assert!(msg.contains("contradicts"), "{cmd}: {msg}");
                }
                other => panic!("{cmd}: expected the typed contradiction, got {other:?}"),
            }
        }
    }

    // ---- the struck-exotic strike grammar ------------------------------------

    #[test]
    fn exotic_struck_families_require_a_strike() {
        let base = [
            "celnet", "exotic", "--spot", "1.10", "--vol", "0.10", "--t", "1.0", "--r-dom", "0.02",
            "--r-for", "0.01",
        ];
        let cases: [(&str, &[&str]); 10] = [
            ("vanilla", &["vanilla", "--option", "call"]),
            ("digital", &["digital", "--kind", "digital-call"]),
            (
                "barrier",
                &[
                    "barrier",
                    "--option",
                    "call",
                    "--topology",
                    "down-and-out",
                    "--barrier",
                    "0.95",
                ],
            ),
            (
                "window-barrier",
                &[
                    "window-barrier",
                    "--option",
                    "call",
                    "--barrier",
                    "1.20",
                    "--window-start",
                    "0.1",
                    "--window-end",
                    "0.5",
                ],
            ),
            ("asian", &["asian", "--option", "call"]),
            (
                "quanto",
                &[
                    "quanto",
                    "--option",
                    "call",
                    "--conversion-vol",
                    "0.09",
                    "--correlation",
                    "0.1",
                ],
            ),
            ("tarf", &["tarf", "--option", "put", "--target", "0.3"]),
            (
                "pivot",
                &[
                    "pivot", "--option", "put", "--pivot", "1.05", "--target", "0.3",
                ],
            ),
            (
                "fixed-strike lookback",
                &["lookback", "--option", "call", "--fixed"],
            ),
            ("american", &["american", "--option", "put"]),
        ];
        for (family, tail) in cases {
            let argv: Vec<&str> = base.iter().chain(tail.iter()).copied().collect();
            let err = run_to_string(&argv).unwrap_err();
            match err {
                DispatchError::MissingStrike { family: got } => assert_eq!(got, family),
                other => panic!("{family}: expected MissingStrike, got {other:?}"),
            }
        }
    }

    #[test]
    fn exotic_unstruck_families_price_without_a_strike() {
        let base = [
            "celnet", "exotic", "--spot", "1.10", "--vol", "0.10", "--t", "1.0", "--r-dom", "0.02",
            "--r-for", "0.01",
        ];
        // The floating lookback's strike is the realised extremum.
        let argv: Vec<&str> = base
            .iter()
            .chain(["lookback", "--option", "call"].iter())
            .copied()
            .collect();
        let out = run_to_string(&argv).unwrap();
        assert!(out.contains("lookback-continuous") && out.contains("price"));
        // A touch is struck at its barrier, not a strike.
        let argv: Vec<&str> = base
            .iter()
            .chain(["one-touch", "--barrier", "1.20"].iter())
            .copied()
            .collect();
        let out = run_to_string(&argv).unwrap();
        assert!(out.contains("one-touch") && out.contains("price"));
    }

    #[test]
    fn exotic_american_uses_the_common_strike_grammar() {
        // The unified grammar: the strike is the common pre-subcommand flag.
        let out = run_to_string(&[
            "celnet", "exotic", "--spot", "1.10", "--vol", "0.10", "--t", "1.0", "--r-dom", "0.02",
            "--r-for", "0.01", "--strike", "1.10", "american", "--option", "put",
        ])
        .unwrap();
        assert!(out.contains("american-fd") && out.contains("price"));
        // The old per-subcommand flag is a structural parse error, not an alias.
        assert!(
            Cli::try_parse_from([
                "celnet", "exotic", "--spot", "1.10", "--vol", "0.10", "--t", "1.0", "--r-dom",
                "0.02", "--r-for", "0.01", "american", "--option", "put", "--strike", "1.10",
            ])
            .is_err()
        );
    }
}
