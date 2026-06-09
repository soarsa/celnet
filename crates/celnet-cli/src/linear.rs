//! The `forward` / `swap` / `ndf` subcommand cores — the linear (non-option) FX
//! book on the command line.
//!
//! These price the dedicated `celnet-linear` book (an outright forward, an FX swap,
//! and a non-deliverable forward) locally, exactly as the server's `pricer` routes
//! them — the same closed-form discounted-cashflow engine, the same near = spot /
//! far = expiry swap-leg separation, and the same product × underlying validity
//! matrix (a forward/swap requires a *deliverable* underlying; an NDF requires a
//! *non-deliverable* one). The CLI is a local-compute client, so it reaches an
//! identical price to the SDK/Excel-via-server path without a running edge — gated
//! against the frozen golden-vector corpus by `tests/conformance.rs`.
//!
//! ## Honest boundary (VERIFICATION-CONTRACT §g)
//!
//! Only the NDF fixing *identity* and the settlement convention are encoded; the
//! live fixing VALUE is an estate-gated feed, never sourced in-repo — it does not
//! enter the deterministic discounted-cashflow PV.

use celnet_linear::{
    ForwardGreeks, LinearInputs, LinearTerms, Side as LinearSide, forward, ndf::Ndf, swap,
};
use celnet_types::{Carry, CcyPair, FixingSource, Tenor, Underlying};

/// The near leg of a swap settles on the spot date (`t = 0`); the far leg settles
/// at the instrument's forward tenor. This mirrors the server's
/// `pricer::SWAP_NEAR_SETTLE_YEARS`, so CLI == server.
const SWAP_NEAR_SETTLE_YEARS: f64 = 0.0;

/// Buy or sell on the command line for a linear (directional) product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliSide {
    /// Buy: long the base/asset forward.
    Buy,
    /// Sell: short the base/asset forward.
    Sell,
}

impl From<CliSide> for LinearSide {
    fn from(v: CliSide) -> Self {
        match v {
            CliSide::Buy => LinearSide::Buy,
            CliSide::Sell => LinearSide::Sell,
        }
    }
}

/// The published NDF settlement-rate fixing on the command line (EMTA/ISDA
/// per-currency templates). Identity only — never a market-data value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliFixing {
    /// Korea — KFTC18 (USD/KRW).
    KrwKftc18,
    /// Taiwan — Taipei Forex (USD/TWD).
    TwdTaipei,
    /// India — RBI reference rate (USD/INR).
    InrRbiRef,
    /// Brazil — BCB PTAX (USD/BRL).
    BrlPtax,
    /// Chile — Dólar Observado (USD/CLP).
    ClpDolarObs,
    /// Colombia — TRM (USD/COP).
    CopTrm,
}

impl From<CliFixing> for FixingSource {
    fn from(v: CliFixing) -> Self {
        match v {
            CliFixing::KrwKftc18 => FixingSource::KrwKftc18,
            CliFixing::TwdTaipei => FixingSource::TwdTaipei,
            CliFixing::InrRbiRef => FixingSource::InrRbiRef,
            CliFixing::BrlPtax => FixingSource::BrlPtax,
            CliFixing::ClpDolarObs => FixingSource::ClpDolarObs,
            CliFixing::CopTrm => FixingSource::CopTrm,
        }
    }
}

/// A linear-product pricing failure surfaced to the user.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LinearError {
    /// The notional or a settlement time was out of its valid domain.
    BadInput(&'static str),
    /// A forward/swap was requested on a non-deliverable pair (use `ndf`), or an
    /// NDF on a deliverable pair (use `forward`) — the validity matrix.
    Deliverability(&'static str),
}

impl core::fmt::Display for LinearError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LinearError::BadInput(s) | LinearError::Deliverability(s) => f.write_str(s),
        }
    }
}

/// The shared market state for a linear product: spot + the two carry rates and the
/// settlement time `t` (the forward/NDF settlement, or the far-leg tenor of a swap).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LinearMarket {
    /// Spot (quote per 1 unit of base).
    pub(crate) spot: f64,
    /// Settlement / far-leg tenor in years.
    pub(crate) t: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub(crate) r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub(crate) r_for: f64,
}

impl LinearMarket {
    fn carry(self) -> Carry {
        Carry::FxRates {
            r_dom: self.r_dom,
            r_for: self.r_for,
        }
    }
}

/// The priced result of a linear product: the PV plus the exact closed-form linear
/// Greek strip (gamma/vega/etc. are identically zero — a linear DCF has no convexity
/// or vol sensitivity). No standard error: these are exact, not Monte-Carlo.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LinearResult {
    /// Present value (quote-currency premium).
    pub(crate) price: f64,
    /// Spot delta `∂PV/∂S`.
    pub(crate) delta: f64,
    /// Theta `∂PV/∂t`.
    pub(crate) theta: f64,
    /// Domestic-rate sensitivity `∂PV/∂r_dom`.
    pub(crate) rho_dom: f64,
    /// Foreign-rate sensitivity `∂PV/∂r_for`.
    pub(crate) rho_for: f64,
}

impl LinearResult {
    fn from_greeks(price: f64, g: ForwardGreeks) -> Self {
        Self {
            price,
            delta: g.delta,
            theta: g.theta,
            rho_dom: g.rho_dom,
            rho_for: g.rho_for,
        }
    }
}

/// `true` if `pair` settles non-deliverable per the convention registry (the
/// settlement style is a pair-level property, invariant under tenor/orientation —
/// mirrors `celnet-server::pricer::underlying_is_non_deliverable`).
fn is_non_deliverable(pair: CcyPair) -> bool {
    celnet_conventions::resolve(pair, Tenor::Years(1))
        .record
        .is_non_deliverable()
}

fn outright_inputs(
    pair: CcyPair,
    market: LinearMarket,
    rate: f64,
    notional: f64,
    side: CliSide,
    settle_t: f64,
) -> Result<LinearInputs, LinearError> {
    LinearInputs::outright(
        market.spot,
        Underlying::Fx(pair),
        market.carry(),
        LinearTerms::new(rate, notional, side.into()),
        settle_t,
    )
    .map_err(|_| LinearError::BadInput("notional must be positive"))
}

/// Price an FX outright forward (deliverable underlying only).
pub(crate) fn run_forward(
    pair: CcyPair,
    market: LinearMarket,
    rate: f64,
    notional: f64,
    side: CliSide,
) -> Result<LinearResult, LinearError> {
    if is_non_deliverable(pair) {
        return Err(LinearError::Deliverability(
            "forward requires a deliverable pair; this pair is non-deliverable (use `ndf`)",
        ));
    }
    let inputs = outright_inputs(pair, market, rate, notional, side, market.t)?;
    Ok(LinearResult::from_greeks(
        forward::pv(&inputs),
        celnet_linear::greeks(&inputs),
    ))
}

/// Price an FX swap (near leg spot-settling + far leg at the forward tenor, opposite
/// sides), deliverable underlying only. The PV is the sum of the two leg PVs; the
/// reported Greek strip is the **net** (near + far) risk.
pub(crate) fn run_swap(
    pair: CcyPair,
    market: LinearMarket,
    rate: f64,
    notional: f64,
    near_side: CliSide,
) -> Result<LinearResult, LinearError> {
    if is_non_deliverable(pair) {
        return Err(LinearError::Deliverability(
            "swap requires a deliverable pair; this pair is non-deliverable",
        ));
    }
    let near_inputs = outright_inputs(
        pair,
        market,
        rate,
        notional,
        near_side,
        SWAP_NEAR_SETTLE_YEARS,
    )?
    .with_far(market.t)
    .map_err(|_| LinearError::BadInput("far settlement time must be non-negative"))?;
    let pv = swap::pv(&near_inputs).map_err(|_| LinearError::BadInput("swap far leg missing"))?;
    // Net swap Greeks: the near leg (spot) plus the far leg (opposite side, forward
    // tenor), each reconstructed as a standalone outright — the exact analytic Greek
    // of the two-leg sum.
    let far_side = near_inputs.side.opposite();
    let near_only = LinearInputs {
        far_settle_t: None,
        ..near_inputs.clone()
    };
    let far_only = LinearInputs {
        side: far_side,
        far_settle_t: None,
        near_settle_t: market.t,
        ..near_inputs
    };
    let ng = celnet_linear::greeks(&near_only);
    let fg = celnet_linear::greeks(&far_only);
    Ok(LinearResult {
        price: pv,
        delta: ng.delta + fg.delta,
        theta: ng.theta + fg.theta,
        rho_dom: ng.rho_dom + fg.rho_dom,
        rho_for: ng.rho_for + fg.rho_for,
    })
}

/// Price a non-deliverable forward (non-deliverable underlying only). The fixing is
/// booking/convention identity only — it does not enter the deterministic PV.
pub(crate) fn run_ndf(
    pair: CcyPair,
    market: LinearMarket,
    rate: f64,
    notional: f64,
    side: CliSide,
    fixing: CliFixing,
) -> Result<LinearResult, LinearError> {
    if !is_non_deliverable(pair) {
        return Err(LinearError::Deliverability(
            "ndf requires a non-deliverable pair; this pair is deliverable (use `forward`)",
        ));
    }
    let inputs = outright_inputs(pair, market, rate, notional, side, market.t)?;
    // The NDF PV equals the equal-terms deliverable-forward PV; the Greek strip is
    // the same outright forward strip from the identical inputs. Take the strip
    // before moving `inputs` into the NDF (`LinearInputs` is no longer `Copy`).
    let greeks = celnet_linear::greeks(&inputs);
    let ndf = Ndf::new(inputs, fixing.into());
    Ok(LinearResult::from_greeks(ndf.pv(), greeks))
}

/// Format a linear-product report (the `price` line keyed exactly like `exotic`/
/// `price` so the CLI conformance harness parses it identically).
#[must_use]
pub(crate) fn format_report(label: &str, r: &LinearResult) -> String {
    format!(
        "{label}\n  price     {price}\n  delta     {delta}\n  theta     {theta}\n  \
         rho_dom   {rho_dom}\n  rho_for   {rho_for}\n",
        price = r.price,
        delta = r.delta,
        theta = r.theta,
        rho_dom = r.rho_dom,
        rho_for = r.rho_for,
    )
}
