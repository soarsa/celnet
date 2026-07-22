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
        let t_secs = now_nanos as f64 * 1.0e-9;
        let decay = libm::exp(-self.reversion_per_sec.max(0.0) * t_secs);
        let mean = self.long_run_yield + (self.initial_yield - self.long_run_yield) * decay;
        mean + self.perturbation * noise
    }

    /// The clean price implied by the sampled yield, via the reference relation.
    /// Returns [`f64::NAN`] only if the bond's cashflow schedule is malformed
    /// (never for a well-formed [`Bond`]); a non-finite mid is then excluded by
    /// the consolidator's non-finite fold rather than silently mispricing.
    #[must_use]
    pub fn clean_price_at(&self, now_nanos: i64, noise: f64) -> f64 {
        let y = self.yield_at(now_nanos, noise);
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
}

impl MidSource {
    /// The mid at logical `now_nanos` given the tick's seeded `noise`. For
    /// [`MidSource::Fixed`] the mid is constant and `noise`/`now_nanos` are
    /// ignored; for the stochastic mode it is the clean price of the sampled
    /// yield.
    #[must_use]
    pub fn mid_at(&self, now_nanos: i64, noise: f64) -> f64 {
        match self {
            MidSource::Fixed(m) => *m,
            MidSource::MeanRevertingYield(model) => model.clean_price_at(now_nanos, noise),
        }
    }

    /// Whether this mid source carries stochastic (tick-seeded) dynamics. Ladder
    /// mids are constant, so their quotes need no per-tick noise draw.
    #[must_use]
    pub fn is_stochastic(&self) -> bool {
        matches!(self, MidSource::MeanRevertingYield(_))
    }
}
