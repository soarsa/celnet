//! The per-instrument ground-truth mid an LP forms at a logical instant.
//!
//! Two modes, per the crate design:
//!
//! - [`MidSource::Fixed`] — a **ground-truth ladder**: an exact, time- and
//!   noise-independent mid. Because it is a constant, an LP's bid/offer are exact
//!   functions of its spread and skew alone, so a test can assert the consolidated
//!   `best_bid = max(fresh bids)` / `best_offer = min(fresh offers)` analytically.
//!
//! - [`MidSource::MeanRevertingYield`] — a seeded **mean-reverting yield** mapped
//!   to a clean bond price via [`celnet_bond::clean_price`] (the reference
//!   relation). The yield follows the closed-form conditional mean of an
//!   Ornstein–Uhlenbeck / Vasicek short-rate process,
//!   `E[yₜ | y₀] = θ + (y₀ − θ)·e^{−κt}`, plus a bounded stationary seeded
//!   perturbation `σ·u(t)` with `u(t) ∈ [−1, 1)`. The `e^{−κt}` pull toward the
//!   long-run mean `θ` is what makes the path mean-reverting rather than a free
//!   random walk; the seeded perturbation supplies decorrelated micro-noise. The
//!   yield→price map is not re-derived here — it is the real analytics leaf, so
//!   the simulator's prices ARE the oracle (see the crate docs).
//!
//! Method/paper provenance lives in prose only, never in identifiers
//! (CLAUDE.md §8): "Vasicek" and "Ornstein–Uhlenbeck" name the mean-reversion of
//! the conditional mean we sample; the public type is purpose-named
//! [`MeanRevertingYield`](MidSource::MeanRevertingYield).

use celnet_bond::{Bond, clean_price};
use celnet_types::Rate;

/// The seeded Ornstein–Uhlenbeck / Vasicek conditional-mean sample shared by every
/// mean-reverting model in this module: `θ + (r₀ − θ)·e^{−κt} + σ·u`, with the
/// caller's bounded stationary perturbation `u ∈ [−1, 1)`.
///
/// Factored out so the yield process (mapped to a bond price) and the par-rate
/// process (quoted directly) are provably the SAME dynamics rather than two
/// separately-drifting re-derivations.
fn mean_reverting_sample(
    long_run: f64,
    initial: f64,
    reversion_per_sec: f64,
    perturbation: f64,
    now_nanos: i64,
    noise: f64,
) -> f64 {
    let t_secs = now_nanos as f64 * 1.0e-9;
    let decay = libm::exp(-reversion_per_sec.max(0.0) * t_secs);
    let mean = long_run + (initial - long_run) * decay;
    mean + perturbation * noise
}

/// A seeded mean-reverting **par-rate** model for a swap/OIS curve point.
///
/// Identical dynamics to [`YieldModel`] (see [`mean_reverting_sample`]), but the
/// sampled level IS the quoted market: an OIS is quoted as a par fixed rate, not as
/// a price per 100 face, so there is no yield→price map to apply. The quoted mid is
/// reported in **percent** (e.g. `4.25` for 4.25%) — the units an OIS market quotes
/// and the units the venue's rates two-way carries on the wire — which also keeps the
/// magnitude in the same range as a cash-bond price handle, so the panel's half-spread
/// and skew budgets stay meaningful across both instrument families.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateModel {
    /// Long-run mean par rate `θ` the process reverts toward (decimal, e.g. `0.0425`).
    pub long_run_rate: f64,
    /// Initial par rate `r₀` at logical time zero (decimal). The conditional mean
    /// decays from here toward `long_run_rate`.
    pub initial_rate: f64,
    /// Mean-reversion speed `κ` per second (larger ⇒ faster pull to the mean).
    /// Non-negative; `0` degenerates to a flat mean at `initial_rate`.
    pub reversion_per_sec: f64,
    /// Amplitude `σ` of the bounded stationary seeded perturbation (decimal rate
    /// units, e.g. `2e-4` = ±2 bp of jitter around the conditional mean).
    pub perturbation: f64,
}

impl RateModel {
    /// The sampled par rate at logical `now_nanos` given the tick's seeded `noise`
    /// (`∈ [−1, 1)`), as a **decimal** (e.g. `0.0425`).
    #[must_use]
    pub fn rate_at(&self, now_nanos: i64, noise: f64) -> f64 {
        self.rate_at_view(now_nanos, noise, 0.0)
    }

    /// The sampled par rate displaced by one quoting member's own `view_rate`
    /// (decimal) — the dealer's idiosyncratic view of where the curve point is, on
    /// top of the market-wide sampled level. Same separation of common market level
    /// and private view as [`YieldModel::clean_price_at_view`], which is what makes a
    /// multi-dealer panel consolidate rather than degenerate.
    #[must_use]
    pub fn rate_at_view(&self, now_nanos: i64, noise: f64, view_rate: f64) -> f64 {
        mean_reverting_sample(
            self.long_run_rate,
            self.initial_rate,
            self.reversion_per_sec,
            self.perturbation,
            now_nanos,
            noise,
        ) + view_rate
    }

    /// The quoted mid in **percent** — the units an OIS two-way is quoted in.
    #[must_use]
    pub fn quoted_mid_at_view(&self, now_nanos: i64, noise: f64, view_rate: f64) -> f64 {
        self.rate_at_view(now_nanos, noise, view_rate) * 100.0
    }
}

/// A seeded mean-reverting yield model whose sampled yield is priced to a clean
/// bond price by [`celnet_bond::clean_price`].
///
/// The sampled yield at logical time `t` seconds is
/// `θ + (y₀ − θ)·e^{−κt} + σ·u`, where `u ∈ [−1, 1)` is the caller-supplied
/// seeded perturbation for the tick. See the module docs for the provenance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YieldModel {
    /// The bond whose clean price the sampled yield is mapped to. Validated at
    /// its own construction; a malformed schedule degrades a quote to non-finite
    /// (and the consolidator then excludes it) rather than panicking.
    pub bond: Bond,
    /// Long-run mean yield `θ` the process reverts toward (decimal, e.g. `0.045`).
    pub long_run_yield: f64,
    /// Initial yield `y₀` at logical time zero (decimal). The conditional mean
    /// decays from here toward `long_run_yield`.
    pub initial_yield: f64,
    /// Mean-reversion speed `κ` per second (larger ⇒ faster pull to the mean).
    /// Non-negative; `0` degenerates to a flat mean at `initial_yield`.
    pub reversion_per_sec: f64,
    /// Amplitude `σ` of the bounded stationary seeded yield perturbation (decimal
    /// yield units, e.g. `2e-4` = ±2 bp of jitter around the conditional mean).
    pub perturbation: f64,
}

impl YieldModel {
    /// The sampled yield at logical `now_nanos` given the tick's seeded `noise`
    /// (`∈ [−1, 1)`): the OU/Vasicek conditional mean plus the bounded stationary
    /// perturbation.
    #[must_use]
    pub fn yield_at(&self, now_nanos: i64, noise: f64) -> f64 {
        mean_reverting_sample(
            self.long_run_yield,
            self.initial_yield,
            self.reversion_per_sec,
            self.perturbation,
            now_nanos,
            noise,
        )
    }

    /// The clean price implied by the sampled yield, via the reference relation.
    /// Returns [`f64::NAN`] only if the bond's cashflow schedule is malformed
    /// (never for a well-formed [`Bond`]); a non-finite mid is then excluded by
    /// the consolidator's non-finite fold rather than silently mispricing.
    #[must_use]
    pub fn clean_price_at(&self, now_nanos: i64, noise: f64) -> f64 {
        self.clean_price_at_view(now_nanos, noise, 0.0)
    }

    /// The clean price implied by the sampled yield **displaced by one quoting
    /// member's own `view_yield`** (decimal yield) — the dealer's idiosyncratic
    /// view of where the security is, on top of the market-wide sampled level.
    ///
    /// Separating the two is what makes a multi-dealer panel consolidate: the
    /// market level is common information every dealer marks off, and the dealer's
    /// own view is a small displacement around it. Priced through the same real
    /// analytics leaf, so a displaced view is still an oracle price of a real yield
    /// (never a price handle nudged after the fact).
    #[must_use]
    pub fn clean_price_at_view(&self, now_nanos: i64, noise: f64, view_yield: f64) -> f64 {
        let y = self.yield_at(now_nanos, noise) + view_yield;
        clean_price(&self.bond, Rate(y)).unwrap_or(f64::NAN)
    }
}

/// How an LP forms its ground-truth mid for one instrument at a logical instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidSource {
    /// A fixed, exact mid (ladder mode) — analytic ground truth for BBO tests.
    Fixed(f64),
    /// A seeded mean-reverting yield priced to a clean bond price (stochastic
    /// mode) — soak/demo realism.
    MeanRevertingYield(YieldModel),
    /// A seeded mean-reverting **par rate** quoted directly in percent (stochastic
    /// mode) — the swap/OIS curve points. No yield→price map: the sampled level is
    /// itself the quoted market.
    MeanRevertingRate(RateModel),
}

impl MidSource {
    /// The mid at logical `now_nanos` given the tick's seeded `noise`. For
    /// [`MidSource::Fixed`] the mid is constant and `noise`/`now_nanos` are
    /// ignored; for the stochastic mode it is the clean price of the sampled
    /// yield.
    #[must_use]
    pub fn mid_at(&self, now_nanos: i64, noise: f64) -> f64 {
        self.mid_at_view(now_nanos, noise, 0.0)
    }

    /// The mid at logical `now_nanos` given the tick's seeded `noise` and the
    /// quoting member's own `view_yield` displacement (decimal yield) — see
    /// [`YieldModel::clean_price_at_view`].
    ///
    /// A [`MidSource::Fixed`] ladder mid ignores `view_yield`: it is an exact,
    /// caller-specified constant whose whole purpose is to be the analytic ground
    /// truth of a BBO test, so it carries no dealer-view dispersion by design.
    #[must_use]
    pub fn mid_at_view(&self, now_nanos: i64, noise: f64, view_yield: f64) -> f64 {
        match self {
            MidSource::Fixed(m) => *m,
            MidSource::MeanRevertingYield(model) => {
                model.clean_price_at_view(now_nanos, noise, view_yield)
            }
            MidSource::MeanRevertingRate(model) => {
                model.quoted_mid_at_view(now_nanos, noise, view_yield)
            }
        }
    }

    /// The bond-yield model behind this mid, when the line is priced through the bond
    /// analytics leaf. `None` for a ladder mid and for a directly-quoted par rate — a
    /// swap curve point has no bond and no DV01-per-100, so a caller reasoning in
    /// price/DV01 terms must select on this rather than assume every off-grid line is
    /// a cash bond.
    #[must_use]
    pub fn yield_model(&self) -> Option<&YieldModel> {
        match self {
            MidSource::MeanRevertingYield(model) => Some(model),
            MidSource::Fixed(_) | MidSource::MeanRevertingRate(_) => None,
        }
    }

    /// The par-rate model behind this mid, when the line is a directly-quoted swap /
    /// OIS curve point. `None` otherwise.
    #[must_use]
    pub fn rate_model(&self) -> Option<&RateModel> {
        match self {
            MidSource::MeanRevertingRate(model) => Some(model),
            MidSource::Fixed(_) | MidSource::MeanRevertingYield(_) => None,
        }
    }

    /// This mid source with its **starting** level displaced by `d` (in the arm's own
    /// natural units — decimal yield for a bond line, decimal rate for a swap line).
    ///
    /// This is the per-member dispersion of the reference level applied when a fleet is
    /// built: each member starts from its own mark around the common reference. A
    /// [`MidSource::Fixed`] ladder mid is returned unchanged — it is an exact
    /// caller-specified constant whose whole purpose is to be the analytic ground truth
    /// of a BBO test, so it carries no dispersion by design.
    #[must_use]
    pub fn with_initial_displaced(self, d: f64) -> Self {
        match self {
            MidSource::Fixed(m) => MidSource::Fixed(m),
            MidSource::MeanRevertingYield(mut model) => {
                model.initial_yield += d;
                MidSource::MeanRevertingYield(model)
            }
            MidSource::MeanRevertingRate(mut model) => {
                model.initial_rate += d;
                MidSource::MeanRevertingRate(model)
            }
        }
    }

    /// Whether this mid source carries stochastic (tick-seeded) dynamics. Ladder
    /// mids are constant, so their quotes need no per-tick noise draw.
    #[must_use]
    pub fn is_stochastic(&self) -> bool {
        matches!(
            self,
            MidSource::MeanRevertingYield(_) | MidSource::MeanRevertingRate(_)
        )
    }
}
