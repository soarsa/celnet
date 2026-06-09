//! One-dimensional finite-difference PDE solver for the Garman-Kohlhagen
//! pricing equation, on a **log-spot** grid with **barrier-aligned** nodes and
//! **Crank-Nicolson** time-stepping started by a short **Rannacher** (fully
//! implicit) phase that damps the oscillations a discontinuous terminal or
//! barrier condition would otherwise excite.
//!
//! # The equation
//!
//! In `x = ln S` the value `V(x, τ)` (with `τ = T − t` running backward from
//! expiry) solves the constant-coefficient convection-diffusion PDE
//!
//! ```text
//!   ∂V/∂τ = ½σ² ∂²V/∂x² + (b − ½σ²) ∂V/∂x − r_d V ,   b = r_d − r_f .
//! ```
//!
//! Constant coefficients in `x` make a uniform grid second-order accurate and let
//! the per-step operator be a single tridiagonal solve.
//!
//! # Why Crank-Nicolson + Rannacher
//!
//! Pure Crank-Nicolson is second-order in time but only A-stable, not
//! L-stable: a non-smooth initial layer (the kink of a vanilla payoff, or the
//! jump of a digital/barrier condition) seeds high-frequency modes that
//! Crank-Nicolson damps far too slowly, producing ringing in price and —
//! catastrophically — in Greeks. The **Rannacher** start-up replaces the first
//! few steps with fully-implicit (backward-Euler) sub-steps, which are L-stable
//! and annihilate those modes, after which Crank-Nicolson recovers second-order
//! accuracy on the now-smooth solution.
//!
//! # Barrier alignment
//!
//! For a barrier product the grid is built so a node sits **exactly** on the
//! barrier level (`ln H` is a grid point). The Dirichlet condition `V = 0` (for a
//! knock-out beyond the barrier) is then imposed without interpolation error,
//! which is the dominant error source for finite-difference barriers.
//!
//! Provenance (doc-only): Crank-Nicolson (1947); Rannacher (1984); the
//! barrier-alignment and grid-design guidance in the modern computational-finance
//! literature (Tavella-Randall 2000; Duffy 2006). Identifiers are purpose-named.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::{OptionType, VanillaInputs};

/// Boundary/terminal specification of a 1-D PDE pricing problem on the log-spot
/// axis.
///
/// The terminal payoff is a vanilla call/put; an optional single barrier turns
/// the problem into a knock-out by truncating the domain at the barrier with a
/// zero Dirichlet wall (knock-in values are obtained by the in/out parity
/// `KI = vanilla − KO`, handled by the caller, so the PDE itself only ever solves
/// the well-posed knock-out / vanilla problem).
#[derive(Debug, Clone, Copy)]
pub struct PdeProblem {
    /// Call or put terminal payoff.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Optional knock-out barrier. `Some((level, up))` truncates the domain at
    /// `level`, killing value on the far side (`up = true` ⇒ kill above).
    pub knock_out: Option<(f64, bool)>,
}

/// Grid resolution and time-stepping controls.
#[derive(Debug, Clone, Copy)]
pub struct PdeGrid {
    /// Number of spatial intervals (`nodes = space_steps + 1`).
    pub space_steps: usize,
    /// Number of backward time steps.
    pub time_steps: usize,
    /// Half-width of the log-spot domain in standard deviations of `ln S_T`
    /// around the forward log-level (when there is no barrier wall on that side).
    pub width_in_std: f64,
    /// Number of fully-implicit Rannacher start-up steps (typically 2).
    pub rannacher_steps: usize,
}

impl Default for PdeGrid {
    fn default() -> Self {
        Self {
            space_steps: 800,
            time_steps: 400,
            width_in_std: 8.0,
            rannacher_steps: 2,
        }
    }
}

/// Solve the 1-D Garman-Kohlhagen PDE for the present value at spot `i.spot`.
///
/// Returns the price interpolated at the spot node (the grid is constructed so
/// the initial log-spot is a node, so this is an exact grid read, not an
/// interpolation, in the no-barrier case; with a barrier-aligned grid the spot
/// is the nearest interior node and a local linear read is used).
///
/// The hot path allocates four working vectors of length `nodes` once and reuses
/// them across all time steps (no per-step allocation).
#[must_use]
pub fn solve(i: &VanillaInputs, problem: PdeProblem, grid: PdeGrid) -> f64 {
    let layout = GridLayout::new(i, problem, grid);
    let n = layout.nodes;

    // Terminal condition V(x, 0) = payoff(e^x).
    let mut v = vec![0.0f64; n];
    for (j, vj) in v.iter_mut().enumerate() {
        let s = exp(layout.x(j));
        *vj = crate::payoff::vanilla_intrinsic(problem.option, s, problem.strike);
    }
    layout.apply_dirichlet(&mut v);

    let dtau = i.t / grid.time_steps as f64;
    let far = FarField::new(i, problem);

    // Pre-factored operators. Backward-Euler (Rannacher) and Crank-Nicolson
    // share the same constant tridiagonal coefficients; only the θ weighting of
    // the implicit/explicit split differs.
    let coeff = Coeff::new(i, layout.dx);

    // Reusable scratch buffers (allocated once).
    let mut rhs = vec![0.0f64; n];
    let mut cprime = vec![0.0f64; n];
    let mut dprime = vec![0.0f64; n];

    for step in 0..grid.time_steps {
        let theta = if step < grid.rannacher_steps {
            // Fully implicit (backward Euler) start-up: L-stable, damps the
            // initial-layer oscillations of the non-smooth payoff.
            1.0
        } else {
            // Crank-Nicolson: second-order, A-stable.
            0.5
        };
        // Remaining time-to-expiry *after* this step completes (τ runs backward
        // from `i.t` at the terminal layer to `0` at valuation). The free far-field
        // edges are refreshed to the analytic discounted-forward asymptotic at this
        // τ before the solve folds them into the RHS.
        let tau = i.t - (step + 1) as f64 * dtau;
        far.apply(&mut v, &layout, tau);
        step_implicit(
            &mut v,
            &mut rhs,
            &mut cprime,
            &mut dprime,
            &coeff,
            &layout,
            dtau,
            theta,
        );
    }

    layout.read_at_spot(&v)
}

/// Geometry of the log-spot grid (uniform in `x = ln S`), built so the spot — and
/// the barrier, if any — land on nodes.
struct GridLayout {
    nodes: usize,
    /// Left edge `x₀` in log-spot.
    x0: f64,
    /// Uniform spacing.
    dx: f64,
    /// Index of the node carrying the initial spot.
    spot_index: usize,
    /// Optional Dirichlet wall: `(index, up)`. Value is zeroed on the far side.
    wall: Option<(usize, bool)>,
}

impl GridLayout {
    fn new(i: &VanillaInputs, problem: PdeProblem, grid: PdeGrid) -> Self {
        let ln_s = ln(i.spot);
        let mu = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * i.t;
        let std = i.vol * sqrt(i.t);
        let half = grid.width_in_std * std;
        let centre = ln_s + mu;

        match problem.knock_out {
            Some((level, up)) => {
                Self::aligned_to_barrier(grid.space_steps, ln_s, ln(level), up, centre, half)
            }
            None => Self::aligned_to_spot(grid.space_steps, ln_s, centre - half, centre + half),
        }
    }

    /// Barrier-free grid: uniform on `[lo, hi]` with `dx` chosen so the spot is a
    /// node (snap the spot to the nearest integer node count from `lo`, then
    /// re-derive `dx`; extend `hi` to a whole number of steps).
    fn aligned_to_spot(space_steps: usize, ln_s: f64, lo: f64, hi: f64) -> Self {
        let dx0 = (hi - lo) / space_steps as f64;
        let k_spot = ((ln_s - lo) / dx0).round().max(1.0);
        let dx = (ln_s - lo) / k_spot;
        let total = ((hi - lo) / dx).ceil().max(k_spot + 1.0) as usize;
        Self {
            nodes: total + 1,
            x0: lo,
            dx,
            spot_index: k_spot as usize,
            wall: None,
        }
    }

    /// Barrier grid: choose `dx` so the spot↔barrier log distance is an **exact**
    /// integer number of intervals `m`, hence *both* the spot and the barrier
    /// land on nodes. The live region is `[barrier, …]` (down) or `[…, barrier]`
    /// (up); the domain is extended on the *live* (far) side by whole steps so the
    /// far-field Dirichlet edge sits well outside the support of the payoff.
    fn aligned_to_barrier(
        space_steps: usize,
        ln_s: f64,
        ln_h: f64,
        up: bool,
        centre: f64,
        half: f64,
    ) -> Self {
        let gap = (ln_h - ln_s).abs();
        // Provisional spacing from the target resolution over the natural width.
        let span = (centre + half).max(ln_h) - (centre - half).min(ln_h);
        let dx0 = span / space_steps as f64;
        // Number of intervals between spot and barrier (≥ 1), giving exact `dx`.
        let m = (gap / dx0).round().max(1.0);
        let dx = gap / m;

        if up {
            // Live region is below the barrier: [lo, ln_h]. Extend `lo` downward
            // by whole steps to cover the natural width.
            let want_lo = (centre - half).min(ln_s - half);
            let steps_below = ((ln_h - want_lo) / dx).ceil().max(m + 1.0) as usize;
            let lo = ln_h - steps_below as f64 * dx;
            let total = steps_below;
            Self {
                nodes: total + 1,
                x0: lo,
                dx,
                spot_index: (steps_below as f64 - m) as usize,
                wall: Some((total, true)),
            }
        } else {
            // Live region is above the barrier: [ln_h, hi]. The barrier is node 0.
            let want_hi = (centre + half).max(ln_s + half);
            let total = ((want_hi - ln_h) / dx).ceil().max(m + 1.0) as usize;
            Self {
                nodes: total + 1,
                x0: ln_h,
                dx,
                spot_index: m as usize,
                wall: Some((0, false)),
            }
        }
    }

    #[inline]
    fn x(&self, j: usize) -> f64 {
        self.x0 + j as f64 * self.dx
    }

    /// Zero the value on the dead side of a knock-out wall. The two *free* outer
    /// edges (those that are not a knock-out wall) are handled separately by
    /// [`FarField`], which refreshes them each step to the analytic
    /// discounted-forward asymptotic; this method only enforces the zero wall.
    fn apply_dirichlet(&self, v: &mut [f64]) {
        if let Some((idx, up)) = self.wall {
            if up {
                for vj in v.iter_mut().skip(idx) {
                    *vj = 0.0;
                }
            } else {
                for vj in v.iter_mut().take(idx + 1) {
                    *vj = 0.0;
                }
            }
        }
    }

    /// Linear read of the solution at the initial spot node.
    fn read_at_spot(&self, v: &[f64]) -> f64 {
        v[self.spot_index]
    }
}

/// Analytic far-field Dirichlet values for the two *free* outer edges of the
/// log-spot domain (the ones that are not a knock-out wall).
///
/// As `S → ∞` (and `S → 0`) the vanilla/knock-out value approaches the
/// **discounted forward intrinsic**: for a call the deep-ITM upper edge tends to
/// `S·e^{−r_f τ} − K·e^{−r_d τ}` (the exercise is certain, so the value is the
/// present value of receiving the asset and paying the strike at expiry) and the
/// deep-OTM lower edge tends to `0`; for a put the roles swap. Freezing the edge
/// at the *undiscounted terminal intrinsic* `max(φ(S−K), 0)` instead — as a naive
/// solver does — injects an `O(discount)` boundary error that is only masked by a
/// very wide domain and silently degrades accuracy at higher rates, longer `T`, or
/// a narrower domain (audit finding `pde.rs:250-262`). Refreshing these edges each
/// step with the correct asymptotic removes that error.
///
/// A knock-out wall keeps its zero Dirichlet value (re-imposed separately); only
/// the genuinely-free outer edges are touched here.
struct FarField {
    option: OptionType,
    strike: f64,
    r_dom: f64,
    r_for: f64,
}

impl FarField {
    fn new(i: &VanillaInputs, problem: PdeProblem) -> Self {
        Self {
            option: problem.option,
            strike: problem.strike,
            r_dom: i.r_dom,
            r_for: i.r_for,
        }
    }

    /// Discounted-forward intrinsic at log-spot `x` with `tau` years remaining:
    /// `max(φ·(S·e^{−r_f τ} − K·e^{−r_d τ}), 0)`. In the deep tails exactly one of
    /// the two payoff branches dominates, so this is the certain-exercise present
    /// value the edge converges to.
    #[inline]
    fn value(&self, x: f64, tau: f64) -> f64 {
        let s = exp(x);
        let fwd = s * exp(-self.r_for * tau) - self.strike * exp(-self.r_dom * tau);
        (self.option.sign() * fwd).max(0.0)
    }

    /// Refresh the two free outer Dirichlet edges (node 0 and node `n−1`) to the
    /// discounted-forward asymptotic at `tau`. Edges coinciding with a knock-out
    /// wall are left for [`GridLayout::apply_dirichlet`] to zero.
    fn apply(&self, v: &mut [f64], layout: &GridLayout, tau: f64) {
        let n = v.len();
        let wall_low = matches!(layout.wall, Some((idx, false)) if idx == 0);
        let wall_high = matches!(layout.wall, Some((idx, true)) if idx == n - 1);
        if !wall_low {
            v[0] = self.value(layout.x(0), tau);
        }
        if !wall_high {
            v[n - 1] = self.value(layout.x(n - 1), tau);
        }
    }
}

/// Constant tridiagonal stencil coefficients of the spatial operator
/// `L V = ½σ² V_xx + (b−½σ²) V_x − r_d V` discretised with centred differences.
struct Coeff {
    /// Sub-diagonal weight (coefficient of `V_{j-1}`).
    lower: f64,
    /// Diagonal weight (coefficient of `V_j`), excluding the time term.
    diag: f64,
    /// Super-diagonal weight (coefficient of `V_{j+1}`).
    upper: f64,
}

impl Coeff {
    fn new(i: &VanillaInputs, dx: f64) -> Self {
        let var = i.vol * i.vol;
        let drift = i.r_dom - i.r_for - 0.5 * var;
        let diff = 0.5 * var / (dx * dx);
        let conv = drift / (2.0 * dx);
        Self {
            lower: diff - conv,
            diag: -2.0 * diff - i.r_dom,
            upper: diff + conv,
        }
    }
}

/// One backward time step of the θ-method: `(I − θΔτ L) Vⁿ⁺¹ = (I + (1−θ)Δτ L) Vⁿ`.
///
/// `theta = 1` is backward Euler (Rannacher start-up); `theta = ½` is
/// Crank-Nicolson. Interior nodes use the constant stencil; boundary nodes hold
/// Dirichlet values (knock-out wall and far-field edges), so the system reduces
/// to the interior tridiagonal solve.
#[allow(clippy::too_many_arguments)]
fn step_implicit(
    v: &mut [f64],
    rhs: &mut [f64],
    cprime: &mut [f64],
    dprime: &mut [f64],
    coeff: &Coeff,
    layout: &GridLayout,
    dtau: f64,
    theta: f64,
) {
    let n = v.len();

    // Interior range [a, b] excluding fixed Dirichlet nodes.
    let (a, b) = interior_range(layout, n);

    // Explicit RHS: (I + (1−θ)Δτ L) Vⁿ on the interior.
    let ex = (1.0 - theta) * dtau;
    for j in a..=b {
        rhs[j] = v[j] + ex * (coeff.lower * v[j - 1] + coeff.diag * v[j] + coeff.upper * v[j + 1]);
    }

    // Implicit tridiagonal: (I − θΔτ L). Coefficients (constant across rows).
    let im = theta * dtau;
    let sub = -im * coeff.lower;
    let dia = 1.0 - im * coeff.diag;
    let sup = -im * coeff.upper;

    // Fold the (fixed) Dirichlet neighbours into the RHS at the interior ends.
    // v[a-1] and v[b+1] are boundary values held constant this step.
    rhs[a] -= sub * v[a - 1];
    rhs[b] -= sup * v[b + 1];

    // Thomas algorithm on rows a..=b.
    thomas(sub, dia, sup, rhs, cprime, dprime, v, a, b);

    // Re-impose the knock-out wall (idempotent; keeps the dead side exactly 0).
    layout.apply_dirichlet(v);
}

/// The interior solve range, excluding the two outer Dirichlet edges and, for a
/// knock-out, excluding the dead side beyond the wall.
fn interior_range(layout: &GridLayout, n: usize) -> (usize, usize) {
    let mut a = 1;
    let mut b = n - 2;
    if let Some((idx, up)) = layout.wall {
        if up {
            // Live region is [0, idx]; interior is [1, idx-1].
            b = idx.saturating_sub(1);
        } else {
            // Live region is [idx, n-1]; interior is [idx+1, n-2].
            a = idx + 1;
        }
    }
    (a, b)
}

/// Thomas (tridiagonal) solver for a constant-coefficient system on rows
/// `a..=b`, writing the solution into `out[a..=b]`. `cprime`/`dprime` are scratch
/// of length ≥ `n`.
#[allow(clippy::too_many_arguments)]
fn thomas(
    sub: f64,
    dia: f64,
    sup: f64,
    rhs: &[f64],
    cprime: &mut [f64],
    dprime: &mut [f64],
    out: &mut [f64],
    a: usize,
    b: usize,
) {
    cprime[a] = sup / dia;
    dprime[a] = rhs[a] / dia;
    for j in (a + 1)..=b {
        let m = dia - sub * cprime[j - 1];
        cprime[j] = sup / m;
        dprime[j] = (rhs[j] - sub * dprime[j - 1]) / m;
    }
    out[b] = dprime[b];
    for j in (a..b).rev() {
        out[j] = dprime[j] - cprime[j] * out[j + 1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BarrierKind, BarrierStyle, SingleBarrier, single_barrier_price};
    use celnet_vanilla::price as vanilla_price;

    fn base() -> VanillaInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)
    }

    /// Repricing a vanilla call/put: the PDE solution matches the
    /// Garman-Kohlhagen closed form to grid tolerance — the headline validation
    /// that the discretisation, Rannacher start-up and time integration are
    /// correct.
    #[test]
    fn pde_reprices_vanilla() {
        let i = base();
        let grid = PdeGrid {
            space_steps: 1000,
            time_steps: 600,
            ..PdeGrid::default()
        };
        for option in [OptionType::Call, OptionType::Put] {
            for k in [80.0, 100.0, 125.0] {
                let problem = PdeProblem {
                    option,
                    strike: k,
                    knock_out: None,
                };
                let pde = solve(&i, problem, grid);
                let exact = vanilla_price(option, &VanillaInputs { strike: k, ..i });
                assert!(
                    (pde - exact).abs() < 5e-3,
                    "{option:?} K={k}: PDE {pde} vs analytic {exact}"
                );
            }
        }
    }

    /// Repricing a single (up-and-out) barrier call: the barrier-aligned PDE
    /// matches the Reiner-Rubinstein closed form. The Rannacher start-up is what
    /// makes the discontinuous knock-out condition price cleanly.
    #[test]
    fn pde_reprices_single_barrier() {
        let i = base();
        let grid = PdeGrid {
            space_steps: 1200,
            time_steps: 800,
            ..PdeGrid::default()
        };
        let (k, h) = (100.0, 130.0);
        let problem = PdeProblem {
            option: OptionType::Call,
            strike: k,
            knock_out: Some((h, true)),
        };
        let pde = solve(&i, problem, grid);
        let analytic = single_barrier_price(
            &(&i).into(),
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: k,
                barrier: h,
                rebate: 0.0,
            },
        );
        assert!(
            (pde - analytic).abs() < 5e-3,
            "up-and-out call: PDE {pde} vs analytic {analytic} (|diff|={})",
            (pde - analytic).abs()
        );
    }

    /// A down-and-out put likewise matches the closed form, exercising the
    /// lower-wall branch of the grid layout and interior range.
    #[test]
    fn pde_reprices_down_and_out_put() {
        let i = base();
        let grid = PdeGrid {
            space_steps: 1200,
            time_steps: 800,
            ..PdeGrid::default()
        };
        let (k, h) = (100.0, 80.0);
        let problem = PdeProblem {
            option: OptionType::Put,
            strike: k,
            knock_out: Some((h, false)),
        };
        let pde = solve(&i, problem, grid);
        let analytic = single_barrier_price(
            &(&i).into(),
            SingleBarrier {
                kind: BarrierKind {
                    up: false,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Put,
                },
                strike: k,
                barrier: h,
                rebate: 0.0,
            },
        );
        assert!(
            (pde - analytic).abs() < 5e-3,
            "down-and-out put: PDE {pde} vs analytic {analytic} (|diff|={})",
            (pde - analytic).abs()
        );
    }

    /// Far-field boundary value (audit `pde.rs:250-262`): the [`FarField`] edge is
    /// the **discounted-forward intrinsic** `max(φ·(S·e^{−r_f τ} − K·e^{−r_d τ}),
    /// 0)`, NOT the frozen undiscounted terminal intrinsic `max(φ·(S − K), 0)` the
    /// old code held constant for the whole solve. This pins the corrected boundary
    /// directly: it must (a) discount both legs at `τ > 0`, differing from the
    /// undiscounted intrinsic by the rate-driven amount, (b) be zero on the
    /// deep-OTM edge, and (c) collapse to the terminal intrinsic at `τ = 0`.
    #[test]
    fn far_field_is_discounted_forward_not_frozen_intrinsic() {
        let i = VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.20, 0.02);
        let tau = 1.0;
        let call = FarField::new(
            &i,
            PdeProblem {
                option: OptionType::Call,
                strike: 70.0,
                knock_out: None,
            },
        );
        let (s, x) = (200.0, ln(200.0)); // deep-ITM upper edge
        let want = s * exp(-i.r_for * tau) - 70.0 * exp(-i.r_dom * tau);
        let frozen = s - 70.0; // the old (wrong) frozen terminal intrinsic
        let got = call.value(x, tau);
        assert!(
            (got - want).abs() < 1e-12,
            "call far-field {got} must equal discounted forward {want}"
        );
        assert!(
            (got - frozen).abs() > 1.0,
            "discounted-forward {got} must differ from frozen intrinsic {frozen}"
        );
        assert!(
            (call.value(x, 0.0) - frozen).abs() < 1e-9,
            "τ=0 ⇒ intrinsic"
        );
        assert!(
            call.value(ln(40.0), tau).abs() < 1e-12,
            "deep-OTM edge is 0"
        );

        let put = FarField::new(
            &i,
            PdeProblem {
                option: OptionType::Put,
                strike: 140.0,
                knock_out: None,
            },
        );
        let sp = 40.0;
        let want_p = 140.0 * exp(-i.r_dom * tau) - sp * exp(-i.r_for * tau);
        assert!((put.value(ln(sp), tau) - want_p).abs() < 1e-12);
        assert!(put.value(ln(400.0), tau).abs() < 1e-12);
    }

    /// End-to-end: with the corrected boundary the no-barrier PDE matches the
    /// Garman-Kohlhagen closed form across call/put and deep ITM/OTM strikes at a
    /// **high** domestic rate (the regime where a wrong far-field boundary would
    /// leak an `O(discount)` error into the price).
    #[test]
    fn pde_reprices_vanilla_high_rate() {
        let i = VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.20, 0.02);
        let grid = PdeGrid {
            space_steps: 1500,
            time_steps: 800,
            ..PdeGrid::default()
        };
        for option in [OptionType::Call, OptionType::Put] {
            for k in [70.0, 100.0, 140.0] {
                let pde = solve(
                    &i,
                    PdeProblem {
                        option,
                        strike: k,
                        knock_out: None,
                    },
                    grid,
                );
                let exact = vanilla_price(option, &VanillaInputs { strike: k, ..i });
                assert!(
                    (pde - exact).abs() < 5e-3,
                    "{option:?} K={k}: PDE {pde} vs analytic {exact} (|diff|={})",
                    (pde - exact).abs()
                );
            }
        }
    }

    /// The barrier node lands exactly on the grid (alignment property): the wall
    /// index reconstructs the barrier level to a small fraction of `dx`.
    #[test]
    fn barrier_node_is_aligned() {
        let i = base();
        let problem = PdeProblem {
            option: OptionType::Call,
            strike: 100.0,
            knock_out: Some((130.0, true)),
        };
        let layout = GridLayout::new(&i, problem, PdeGrid::default());
        let (idx, _up) = layout.wall.unwrap();
        let ln_h_node = layout.x(idx);
        assert!(
            (exp(ln_h_node) - 130.0).abs() < 1e-9,
            "barrier node {} not aligned to 130",
            exp(ln_h_node)
        );
        // And the spot is a node too.
        assert!((exp(layout.x(layout.spot_index)) - 100.0).abs() < 1e-9);
    }
}
