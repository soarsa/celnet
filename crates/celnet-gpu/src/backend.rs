//! The [`PricingBackend`] seam and its plain-data path/payoff descriptors.
//!
//! A backend simulates geometric-Brownian-motion terminal states for a batch of
//! Monte-Carlo paths and reduces a payoff over them. The trait is *scalar-generic*
//! (`F: Scalar`) so the f64 CPU oracle and the f32 GPU path share one contract;
//! the concrete element type is the only thing that differs. Both produce a
//! [`Reduction`] in f64 (the f32 backend widens its block sums on read-back), so
//! callers compare results in one numeric space.
//!
//! This mirrors the `PricingBackend` sketch in `docs/ARCHITECTURE.md` §4
//! (`simulate_paths` / `reduce_payoff`), specialized here to the single-asset GBM
//! vanilla Monte-Carlo that the GPU path implements end-to-end.

use crate::Scalar;

/// Specification of a single-asset geometric-Brownian-motion Monte-Carlo batch.
///
/// The terminal log-spot of path `p` is
/// `ln S_T = ln S_0 + (μ − ½σ²)·T + σ·√T·Z_p`, with `Z_p` the path's standard
/// normal drawn from the Philox stream. For a Garman-Kohlhagen FX option the
/// risk-neutral drift is `μ = r_dom − r_for`; the present value discounts the
/// expected payoff by `e^{−r_dom·T}`. One exact step reproduces the terminal
/// distribution of a vanilla, which is all this backend prices today.
///
/// `steps` is a **reserved** field for forthcoming path-dependent extensions
/// (barrier/touch/Asian kernels). It is clamped to `≥ 1` by [`PathSpec::gbm`]
/// but the current single-step terminal engine (both [`crate::CpuBackend`] and
/// the WGSL kernel) draws only the `step = 0` normal and ignores any larger
/// value: a caller setting `steps > 1` gets the exact one-step terminal, not a
/// multi-step path. The field is wired now so the multi-step engine can land
/// without a `PathSpec` interface change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathSpec {
    /// Initial spot `S_0`.
    pub spot: f64,
    /// Risk-neutral drift `μ = r_dom − r_for`.
    pub drift: f64,
    /// Volatility `σ` (annualized, absolute).
    pub vol: f64,
    /// Time to expiry `T` in years.
    pub t: f64,
    /// Domestic continuously-compounded rate `r_dom`, used for discounting.
    pub r_dom: f64,
    /// Number of Monte-Carlo paths in the batch.
    pub paths: u32,
    /// Reserved step count (≥ 1) for forthcoming path-dependent kernels. One
    /// step is exact for a GBM terminal, which is all the current engine prices;
    /// values `> 1` are accepted but ignored (see the type-level docs).
    pub steps: u32,
    /// Global Philox run seed (fixes the entire reproducible bitstream).
    pub seed: u64,
}

impl PathSpec {
    /// Build a spec from a Garman-Kohlhagen vanilla parameter set.
    ///
    /// The eight market parameters are irreducible (spot, vol, expiry, both
    /// rates, batch size, step count, seed); grouping them into sub-structs would
    /// only obscure a flat, plain-data constructor.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn gbm(
        spot: f64,
        vol: f64,
        t: f64,
        r_dom: f64,
        r_for: f64,
        paths: u32,
        steps: u32,
        seed: u64,
    ) -> Self {
        Self {
            spot,
            drift: r_dom - r_for,
            vol,
            t,
            r_dom,
            paths,
            steps: steps.max(1),
            seed,
        }
    }
}

/// A European vanilla payoff kernel: `max(±(S_T − K), 0)`.
///
/// Purpose-named (no model/person attribution). The sign carries call/put.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PayoffKernel {
    /// Strike `K`.
    pub strike: f64,
    /// `+1.0` for a call, `−1.0` for a put.
    pub sign: f64,
}

impl PayoffKernel {
    /// European call with the given strike.
    #[must_use]
    pub const fn call(strike: f64) -> Self {
        Self { strike, sign: 1.0 }
    }

    /// European put with the given strike.
    #[must_use]
    pub const fn put(strike: f64) -> Self {
        Self { strike, sign: -1.0 }
    }

    /// Undiscounted payoff at terminal spot `s` (evaluated in `F`'s precision by
    /// the caller; this f64 form is the CPU oracle).
    #[inline]
    #[must_use]
    pub fn evaluate(&self, s: f64) -> f64 {
        (self.sign * (s - self.strike)).max(0.0)
    }
}

/// The result of reducing a payoff over a Monte-Carlo batch.
///
/// `sum`/`sum_sq` are the (undiscounted) payoff moments accumulated by an
/// integer/tree reduction — never a single running float atomic — so the
/// accumulation order is fixed and the result is reproducible. `mean`, the
/// discounted `price`, and the `std_error` are derived in f64.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reduction {
    /// Number of paths reduced.
    pub paths: u32,
    /// Σ payoff (undiscounted).
    pub sum: f64,
    /// Σ payoff² (undiscounted), for the Monte-Carlo standard error.
    pub sum_sq: f64,
    /// Discount factor `e^{−r_dom·T}` applied to the mean to get the price.
    pub discount: f64,
}

impl Reduction {
    /// Mean undiscounted payoff `Σ/N`.
    #[must_use]
    pub fn mean(&self) -> f64 {
        if self.paths == 0 {
            return 0.0;
        }
        self.sum / f64::from(self.paths)
    }

    /// Discounted Monte-Carlo price estimate `e^{−r_dom·T} · mean`.
    #[must_use]
    pub fn price(&self) -> f64 {
        self.discount * self.mean()
    }

    /// Monte-Carlo standard error of the *price* estimate,
    /// `e^{−r_dom·T} · √(Var/N)` with the sample variance of the payoff.
    #[must_use]
    pub fn std_error(&self) -> f64 {
        let n = f64::from(self.paths);
        if self.paths < 2 {
            return 0.0;
        }
        let mean = self.mean();
        // Unbiased sample variance, clamped at 0 against tiny negative roundoff.
        let var = ((self.sum_sq - n * mean * mean) / (n - 1.0)).max(0.0);
        self.discount * celnet_core::math::sqrt(var / n)
    }
}

/// A Monte-Carlo pricing backend over a chosen scalar precision.
///
/// Implementors: [`crate::CpuBackend`] (f64 reference oracle) and
/// [`crate::GpuBackend`] (f32 wgpu compute, falling back to the CPU backend when
/// no adapter is present). The two methods are deliberately separable so a path
/// buffer can be produced once and reduced by several payoffs; the GPU
/// implementation fuses them in one dispatch for throughput and exposes the same
/// API. `F` is the simulation precision; the returned [`Reduction`] is always f64.
pub trait PricingBackend {
    /// The scalar precision the backend simulates in (`f64` CPU, `f32` GPU).
    type Scalar: Scalar;

    /// Human-readable backend identity (e.g. `"cpu-f64"`, `"gpu-f32:Metal"`).
    fn label(&self) -> String;

    /// Simulate the batch and return per-path terminal spots in `Self::Scalar`.
    ///
    /// Determinism: the `i`-th element corresponds to Philox path index `i`, so
    /// the buffer is identical across runs and (modulo the f32/f64 element type)
    /// across backends.
    fn simulate_paths(&self, spec: &PathSpec) -> Vec<Self::Scalar>;

    /// Reduce `payoff` over already-simulated `terminals`, discounting by
    /// `discount = e^{−r_dom·T}` to produce a [`Reduction`].
    fn reduce_payoff(
        &self,
        terminals: &[Self::Scalar],
        payoff: &PayoffKernel,
        paths: u32,
        discount: f64,
    ) -> Reduction;

    /// Fused convenience: simulate then reduce, returning the [`Reduction`].
    ///
    /// The GPU backend overrides this to keep the path buffer on-device (one
    /// dispatch, no host round-trip of the full path array); the default chains
    /// the two primitives for the CPU oracle.
    fn price_vanilla(&self, spec: &PathSpec, payoff: &PayoffKernel) -> Reduction {
        let terminals = self.simulate_paths(spec);
        let discount = celnet_core::math::exp(-spec.r_dom * spec.t);
        self.reduce_payoff(&terminals, payoff, spec.paths, discount)
    }
}
