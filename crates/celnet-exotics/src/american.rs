//! American and Bermudan early-exercise vanilla pricing for FX options.
//!
//! Physically-settled FX options DO trade American-style: the holder may exercise
//! at any time up to expiry (American) or on a discrete set of permitted dates
//! (Bermudan). This module prices the early-exercise vanilla call/put two
//! independent ways and cross-validates them:
//!
//! 1. **Projected-SOR free-boundary Crank-Nicolson finite difference**
//!    ([`american_fd`] / [`american_fd_greeks`]) — the workhorse. The
//!    early-exercise feature makes the problem a *linear complementarity problem*
//!    (LCP): at every node the value must satisfy
//!
//!    ```text
//!      V ≥ g ,   ℒV ≤ 0 ,   (V − g)·ℒV = 0 ,
//!    ```
//!
//!    where `g = max(φ·(S − K), 0)` is the exercise (intrinsic) payoff and `ℒ` is
//!    the Garman-Kohlhagen operator discretised exactly as in [`crate::pde`]
//!    (log-spot, Crank-Nicolson with a Rannacher fully-implicit start-up). Each
//!    backward time step is solved by **projected successive over-relaxation
//!    (PSOR)**: the Gauss-Seidel/SOR sweep of the implicit tridiagonal system is
//!    followed at every node by the projection `V_j ← max(V_j, g_j)`, which
//!    enforces the LCP and discovers the free (early-exercise) boundary without
//!    tracking it explicitly. For a **Bermudan** the continuation problem is the
//!    *European* PDE between exercise dates, and the projection is applied only at
//!    the time layers that coincide with a permitted exercise date; with the
//!    exercise set equal to every step this reduces to the American solve, and
//!    with a single exercise date at expiry it reduces to the European value.
//!
//! 2. **Longstaff-Schwartz regression Monte-Carlo** ([`american_lsm`]) — an
//!    independent cross-check (and the engine for any future case the FD grid
//!    cannot cover). Forward GBM paths are generated from a low-discrepancy
//!    [`celnet_qmc`] Sobol sequence mapped through a Brownian bridge; the
//!    continuation value at each exercise date is regressed on a polynomial basis
//!    of the spot, and the optimal stopping rule (`exercise iff intrinsic ≥
//!    estimated continuation`) is applied backward. The estimator carries an
//!    honest standard error (it is a Monte-Carlo mean), surfaced on the wire's
//!    `price_std_error`.
//!
//! # Scope (honest boundary)
//!
//! This module prices American / Bermudan **vanilla** (call/put). American
//! *barrier* options price cleanly on the same PSOR grid (the knock-out wall of
//! [`crate::pde`] composes with the exercise projection) and are the natural next
//! increment, but they are **not** built here — they are deliberately out of
//! scope rather than half-built, and are documented as a follow-on so the wire
//! contract does not advertise an unimplemented payoff. The LSM engine is the
//! sanctioned route for path-dependent early exercise when it is added.
//!
//! # No early-exercise premium ⇒ European (the discriminating invariant)
//!
//! Early exercise is never optimal for an American FX **call** when the foreign
//! rate `r_f = 0` (no dividend / carry benefit to holding the asset early), nor
//! for an American **put** when the domestic rate `r_d = 0`. In those regimes the
//! American value equals the European Garman-Kohlhagen value to grid tolerance —
//! a structural oracle the tests pin directly (American ≥ European always; equal
//! when exercise is provably never optimal).
//!
//! # Method provenance (doc comments only)
//!
//! Projected SOR for the American LCP: Cryer (1971); Wilmott-Dewynne-Howison
//! (1993). The penalty/PSOR free-boundary treatment and Brennan-Schwartz
//! ordering: Brennan-Schwartz (1977). Crank-Nicolson with Rannacher start-up:
//! Crank-Nicolson (1947), Rannacher (1984). Regression Monte-Carlo for optimal
//! stopping: Longstaff-Schwartz (2001); Tsitsiklis-Van Roy (2001). All
//! identifiers here are purpose-named and vendor/research-neutral; provenance
//! lives only in documentation.

use celnet_core::math::{exp, ln, sqrt};
use celnet_qmc::{BrownianBridge, SobolSequence, inv_norm_cdf};
use celnet_types::{Greeks, OptionType, VanillaInputs};

/// Whether the option may be exercised continuously (American) or only on a
/// discrete set of permitted dates (Bermudan).
#[derive(Debug, Clone, PartialEq)]
pub enum ExerciseStyle {
    /// Exercise permitted continuously up to and including expiry. The PSOR
    /// projection is applied at **every** backward time step.
    American,
    /// Exercise permitted only on the given set of year-fractions in `(0, T]`
    /// (each measured from valuation). The projection is applied at the time
    /// layers that coincide with a permitted date; the continuation problem is
    /// the European PDE between dates. A single date at `T` is exactly European;
    /// a dense set approaches American.
    Bermudan {
        /// The permitted exercise dates as year-fractions in `(0, T]`, in any
        /// order (sorted internally). Duplicates and out-of-range entries are
        /// ignored. Expiry itself is always an exercise opportunity for a
        /// vanilla, so it is added implicitly if absent.
        dates: Vec<f64>,
    },
}

/// Grid resolution and time-stepping controls for the PSOR finite-difference
/// American/Bermudan solve. Mirrors [`crate::pde::PdeGrid`] but adds the PSOR
/// iteration controls.
#[derive(Debug, Clone, Copy)]
pub struct AmericanGrid {
    /// Number of spatial intervals (`nodes = space_steps + 1`).
    pub space_steps: usize,
    /// Number of backward time steps.
    pub time_steps: usize,
    /// Half-width of the log-spot domain in standard deviations of `ln S_T`.
    pub width_in_std: f64,
    /// Number of fully-implicit Rannacher start-up steps (typically 2).
    pub rannacher_steps: usize,
    /// SOR over-relaxation factor `ω ∈ (1, 2)`. `1.0` is plain Gauss-Seidel.
    pub omega: f64,
    /// Convergence tolerance on the max nodewise change between PSOR sweeps.
    pub psor_tol: f64,
    /// Hard cap on PSOR sweeps per time step (guards against a non-converging
    /// relaxation; sized generously so it is never hit on a well-posed grid).
    pub psor_max_iters: usize,
}

impl Default for AmericanGrid {
    fn default() -> Self {
        Self {
            space_steps: 800,
            time_steps: 400,
            width_in_std: 8.0,
            rannacher_steps: 2,
            omega: 1.5,
            psor_tol: 1e-10,
            psor_max_iters: 10_000,
        }
    }
}

/// An American or Bermudan vanilla early-exercise option specification.
#[derive(Debug, Clone, PartialEq)]
pub struct AmericanOption {
    /// Call or put.
    pub option: OptionType,
    /// Strike `K` (quote per unit base).
    pub strike: f64,
    /// Continuous (American) or discrete-date (Bermudan) exercise.
    pub style: ExerciseStyle,
}

/// Intrinsic (immediate-exercise) value of a vanilla at spot `s`.
#[inline]
fn intrinsic(option: OptionType, s: f64, strike: f64) -> f64 {
    (option.sign() * (s - strike)).max(0.0)
}

// ===========================================================================
// PSOR free-boundary finite difference
// ===========================================================================

/// Price an American/Bermudan vanilla by the projected-SOR free-boundary
/// Crank-Nicolson finite difference.
///
/// Returns the present value at `i.spot`. The grid is built so the initial
/// log-spot is a node (exact read, no interpolation). The strike inside `i` is
/// ignored in favour of `spec.strike`.
#[must_use]
pub fn american_fd(i: &VanillaInputs, spec: &AmericanOption, grid: AmericanGrid) -> f64 {
    let layout = FdLayout::new(i, grid);
    let n = layout.nodes;

    // Per-node intrinsic (exercise) payoff g_j, constant in time for a vanilla.
    let g: Vec<f64> = (0..n)
        .map(|j| intrinsic(spec.option, exp(layout.x(j)), spec.strike))
        .collect();

    // Terminal condition V(x, 0) = g (expiry is always an exercise opportunity).
    let mut v = g.clone();

    let dtau = i.t / grid.time_steps as f64;
    let coeff = Coeff::new(i, layout.dx);
    let far = FarField::new(spec.option, spec.strike, i.r_dom, i.r_for);

    // Resolve the set of step indices at which exercise is permitted (the layer
    // *reached* after the step completes carries remaining time τ = t − k·dtau).
    let exercisable = ExerciseSchedule::new(&spec.style, i.t, dtau, grid.time_steps);

    let mut rhs = vec![0.0f64; n];

    for step in 0..grid.time_steps {
        let theta = if step < grid.rannacher_steps {
            1.0
        } else {
            0.5
        };
        let tau = i.t - (step + 1) as f64 * dtau;
        // For an exercisable layer the LCP holds at the boundary too (V ≥ g), so
        // the far-field edge is floored at intrinsic; on a non-exercisable
        // (Bermudan between-date / European) layer the edge is the pure
        // discounted-forward European asymptotic, with no early-exercise floor.
        let project = exercisable.exercisable_at(step);
        far.apply(&mut v, &layout, tau, project);

        // Build the explicit RHS and the implicit tridiagonal, then solve the
        // step. For an exercisable layer the solve is the PSOR LCP (V ≥ g); for a
        // non-exercisable layer (Bermudan between dates) it is the plain European
        // implicit solve.
        step_psor(&mut v, &mut rhs, &coeff, dtau, theta, &g, grid, project);
    }

    layout.read_at_spot(&v)
}

/// The full 13-member Greek strip of an American/Bermudan vanilla by central
/// finite differences of [`american_fd`] (the same FD-Greek scheme the server's
/// exotic router uses for the closed-form exotics). Each bumped axis re-solves
/// the PSOR grid, so the early-exercise boundary moves consistently with the
/// shocked market.
#[must_use]
pub fn american_fd_greeks(i: &VanillaInputs, spec: &AmericanOption, grid: AmericanGrid) -> Greeks {
    let price = |x: &VanillaInputs| american_fd(x, spec, grid);
    fd_greeks(i, &price)
}

/// Central-finite-difference Greek strip over a re-pricing closure. Shared by the
/// FD pricer; bumps spot/vol/rates/time the same way the analytic exotic router
/// does (1 bp spot, 1 vol-point, 1 bp rate, relative time).
fn fd_greeks(i: &VanillaInputs, price: &dyn Fn(&VanillaInputs) -> f64) -> Greeks {
    const H_S_REL: f64 = 1e-4;
    const H_V: f64 = 1e-4;
    const H_R: f64 = 1e-4;
    const H_T_REL: f64 = 1e-4;

    let with_spot = |s: f64| VanillaInputs { spot: s, ..*i };
    let with_vol = |sig: f64| VanillaInputs { vol: sig, ..*i };

    let base = price(i);
    let h_s = i.spot * H_S_REL;
    let p_up = price(&with_spot(i.spot + h_s));
    let p_dn = price(&with_spot(i.spot - h_s));
    let delta_spot = (p_up - p_dn) / (2.0 * h_s);
    let gamma = (p_up - 2.0 * base + p_dn) / (h_s * h_s);
    let p_up2 = price(&with_spot(i.spot + 2.0 * h_s));
    let p_dn2 = price(&with_spot(i.spot - 2.0 * h_s));
    let speed = (p_up2 - 2.0 * p_up + 2.0 * p_dn - p_dn2) / (2.0 * h_s * h_s * h_s);

    let v_up = price(&with_vol(i.vol + H_V));
    let v_dn = price(&with_vol(i.vol - H_V));
    let vega = (v_up - v_dn) / (2.0 * H_V);
    let volga = (v_up - 2.0 * base + v_dn) / (H_V * H_V);

    let bump = |ds: f64, dv: f64| VanillaInputs {
        spot: i.spot + ds,
        vol: i.vol + dv,
        ..*i
    };
    let vanna = (price(&bump(h_s, H_V)) - price(&bump(h_s, -H_V)) - price(&bump(-h_s, H_V))
        + price(&bump(-h_s, -H_V)))
        / (4.0 * h_s * H_V);
    let gamma_vu = (price(&bump(h_s, H_V)) - 2.0 * v_up + price(&bump(-h_s, H_V))) / (h_s * h_s);
    let gamma_vd = (price(&bump(h_s, -H_V)) - 2.0 * v_dn + price(&bump(-h_s, -H_V))) / (h_s * h_s);
    let zomma = (gamma_vu - gamma_vd) / (2.0 * H_V);

    let rho_dom = {
        let up = price(&VanillaInputs {
            r_dom: i.r_dom + H_R,
            ..*i
        });
        let dn = price(&VanillaInputs {
            r_dom: i.r_dom - H_R,
            ..*i
        });
        (up - dn) / (2.0 * H_R)
    };
    let rho_for = {
        let up = price(&VanillaInputs {
            r_for: i.r_for + H_R,
            ..*i
        });
        let dn = price(&VanillaInputs {
            r_for: i.r_for - H_R,
            ..*i
        });
        (up - dn) / (2.0 * H_R)
    };

    let h_t = i.t * H_T_REL;
    let at_t = |t: f64, ds: f64| VanillaInputs {
        spot: i.spot + ds,
        t,
        ..*i
    };
    let t_up = i.t + h_t;
    let t_dn = (i.t - h_t).max(f64::MIN_POSITIVE);
    let theta = -(price(&at_t(t_up, 0.0)) - price(&at_t(t_dn, 0.0))) / (2.0 * h_t);
    let charm = {
        let d_up = (price(&at_t(t_up, h_s)) - price(&at_t(t_up, -h_s))) / (2.0 * h_s);
        let d_dn = (price(&at_t(t_dn, h_s)) - price(&at_t(t_dn, -h_s))) / (2.0 * h_s);
        (d_up - d_dn) / (2.0 * h_t)
    };
    let color = {
        let g_up = (price(&at_t(t_up, h_s)) - 2.0 * price(&at_t(t_up, 0.0))
            + price(&at_t(t_up, -h_s)))
            / (h_s * h_s);
        let g_dn = (price(&at_t(t_dn, h_s)) - 2.0 * price(&at_t(t_dn, 0.0))
            + price(&at_t(t_dn, -h_s)))
            / (h_s * h_s);
        (g_up - g_dn) / (2.0 * h_t)
    };

    let delta_forward = delta_spot * exp(i.r_for * i.t);

    Greeks {
        price: base,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rho_dom,
        rho_for,
        vanna,
        volga,
        charm,
        speed,
        zomma,
        color,
    }
}

/// The set of backward-step indices at which early exercise is permitted.
struct ExerciseSchedule {
    /// `true` ⇒ exercise permitted at every step (American).
    every: bool,
    /// For Bermudan, the step indices whose post-step layer carries a permitted
    /// exercise date (sorted, deduped).
    steps: Vec<usize>,
}

impl ExerciseSchedule {
    fn new(style: &ExerciseStyle, t: f64, dtau: f64, time_steps: usize) -> Self {
        match style {
            ExerciseStyle::American => Self {
                every: true,
                steps: Vec::new(),
            },
            ExerciseStyle::Bermudan { dates } => {
                // Backward layout (mirroring `crate::pde`): the carried vector `v`
                // starts at EXPIRY and marches to valuation; after backward step
                // `k` the remaining-time-to-expiry is `τ = t − (k+1)·dtau`. A
                // Bermudan date `d` (calendar years from valuation) sits at
                // `τ = t − d`, i.e. backward step `k = d/dtau − 1`. The terminal
                // (expiry) exercise is ALWAYS available for a vanilla and is
                // already seeded into `v` before the loop, so a date AT expiry
                // (`d == t`) needs no interior projection and is dropped here; only
                // strictly-interior dates `0 < d < t` map to a projected step.
                let mut steps: Vec<usize> = dates
                    .iter()
                    .filter(|d| **d > 0.0 && **d < t - 1e-12)
                    .map(|d| {
                        let k = (d / dtau).round().max(1.0) as usize;
                        k.min(time_steps).saturating_sub(1)
                    })
                    .collect();
                steps.sort_unstable();
                steps.dedup();
                Self {
                    every: false,
                    steps,
                }
            }
        }
    }

    #[inline]
    fn exercisable_at(&self, step: usize) -> bool {
        self.every || self.steps.binary_search(&step).is_ok()
    }
}

/// Geometry of the log-spot grid (uniform in `x = ln S`), built so the spot lands
/// on a node. The American problem has no interior knock-out wall; only the two
/// free far-field edges are Dirichlet.
struct FdLayout {
    nodes: usize,
    x0: f64,
    dx: f64,
    spot_index: usize,
}

impl FdLayout {
    fn new(i: &VanillaInputs, grid: AmericanGrid) -> Self {
        let ln_s = ln(i.spot);
        let mu = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * i.t;
        let std = i.vol * sqrt(i.t);
        let half = grid.width_in_std * std;
        let centre = ln_s + mu;
        let (lo, hi) = (centre - half, centre + half);
        let dx0 = (hi - lo) / grid.space_steps as f64;
        let k_spot = ((ln_s - lo) / dx0).round().max(1.0);
        let dx = (ln_s - lo) / k_spot;
        let total = ((hi - lo) / dx).ceil().max(k_spot + 1.0) as usize;
        Self {
            nodes: total + 1,
            x0: lo,
            dx,
            spot_index: k_spot as usize,
        }
    }

    #[inline]
    fn x(&self, j: usize) -> f64 {
        self.x0 + j as f64 * self.dx
    }

    #[inline]
    fn read_at_spot(&self, v: &[f64]) -> f64 {
        v[self.spot_index]
    }
}

/// Far-field Dirichlet edge values for the early-exercise problem.
///
/// In the deep tails the American value coincides with the European
/// discounted-forward intrinsic: deep-ITM exercise is certain, and the
/// early-exercise premium vanishes far from the boundary, so the edge is the same
/// `max(φ·(S·e^{−r_f τ} − K·e^{−r_d τ}), 0)` asymptotic [`crate::pde`] uses — but
/// floored at the immediate-exercise intrinsic `g` (an American value can never
/// be below its exercise payoff, which dominates the discounted forward at the
/// very deep-ITM edge when carry is adverse). This keeps the edge consistent with
/// the LCP `V ≥ g` everywhere.
struct FarField {
    option: OptionType,
    strike: f64,
    r_dom: f64,
    r_for: f64,
}

impl FarField {
    fn new(option: OptionType, strike: f64, r_dom: f64, r_for: f64) -> Self {
        Self {
            option,
            strike,
            r_dom,
            r_for,
        }
    }

    #[inline]
    fn value(&self, x: f64, tau: f64, floor_intrinsic: bool) -> f64 {
        let s = exp(x);
        let fwd = s * exp(-self.r_for * tau) - self.strike * exp(-self.r_dom * tau);
        let european = (self.option.sign() * fwd).max(0.0);
        if floor_intrinsic {
            european.max(intrinsic(self.option, s, self.strike))
        } else {
            european
        }
    }

    fn apply(&self, v: &mut [f64], layout: &FdLayout, tau: f64, floor_intrinsic: bool) {
        let n = v.len();
        v[0] = self.value(layout.x(0), tau, floor_intrinsic);
        v[n - 1] = self.value(layout.x(n - 1), tau, floor_intrinsic);
    }
}

/// Constant tridiagonal stencil of the spatial operator (identical discretisation
/// to [`crate::pde`]).
struct Coeff {
    lower: f64,
    diag: f64,
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

/// One backward θ-step solved by **projected SOR** when `project` is set
/// (exercise permitted), or by a plain implicit Gauss-Seidel/SOR solve of the
/// same tridiagonal system when it is not (Bermudan continuation between dates).
///
/// The implicit system is `(I − θΔτ L) Vⁿ⁺¹ = (I + (1−θ)Δτ L) Vⁿ`. PSOR sweeps the
/// interior with over-relaxation factor `ω`; on each updated node it projects
/// `V_j ← max(V_j, g_j)` when `project`, enforcing the LCP `V ≥ g`. The iteration
/// terminates when the max nodewise change drops below `grid.psor_tol`.
#[allow(clippy::too_many_arguments)]
fn step_psor(
    v: &mut [f64],
    rhs: &mut [f64],
    coeff: &Coeff,
    dtau: f64,
    theta: f64,
    g: &[f64],
    grid: AmericanGrid,
    project: bool,
) {
    let n = v.len();
    let (a, b) = (1usize, n - 2);

    // Explicit RHS on the interior.
    let ex = (1.0 - theta) * dtau;
    for j in a..=b {
        rhs[j] = v[j] + ex * (coeff.lower * v[j - 1] + coeff.diag * v[j] + coeff.upper * v[j + 1]);
    }

    // Implicit coefficients (constant across rows).
    let im = theta * dtau;
    let sub = -im * coeff.lower;
    let dia = 1.0 - im * coeff.diag;
    let sup = -im * coeff.upper;

    // Fold the fixed Dirichlet neighbours into the interior-end RHS.
    rhs[a] -= sub * v[a - 1];
    rhs[b] -= sup * v[b + 1];

    // PSOR iteration. The initial guess is the carried value `v` (warm start).
    let inv_dia = 1.0 / dia;
    for _ in 0..grid.psor_max_iters {
        let mut max_change = 0.0f64;
        for j in a..=b {
            // Gauss-Seidel residual: use the already-updated v[j-1] and the
            // current v[j+1]. The interior-end couplings to the Dirichlet edges
            // were folded into rhs, so the neighbour terms are the interior ones.
            let lhs_neighbours = if j == a {
                sup * v[j + 1]
            } else if j == b {
                sub * v[j - 1]
            } else {
                sub * v[j - 1] + sup * v[j + 1]
            };
            let gs = (rhs[j] - lhs_neighbours) * inv_dia;
            let mut new_val = v[j] + grid.omega * (gs - v[j]);
            if project {
                new_val = new_val.max(g[j]);
            }
            let change = (new_val - v[j]).abs();
            if change > max_change {
                max_change = change;
            }
            v[j] = new_val;
        }
        if max_change < grid.psor_tol {
            break;
        }
    }
}

// ===========================================================================
// Longstaff-Schwartz regression Monte-Carlo
// ===========================================================================

/// Monte-Carlo controls for the Longstaff-Schwartz estimator.
#[derive(Debug, Clone, Copy)]
pub struct LsmConfig {
    /// Number of simulated paths.
    pub paths: usize,
    /// Number of equally-spaced exercise opportunities over `(0, T]` used to
    /// discretise an American exercise (more ⇒ closer to the continuous boundary;
    /// for a Bermudan the explicit `dates` override this).
    pub exercise_dates: usize,
    /// Scramble seed for the Sobol sequence (different seeds give independent
    /// estimates; the same seed is bit-reproducible).
    pub seed: u64,
}

impl Default for LsmConfig {
    fn default() -> Self {
        Self {
            paths: 100_000,
            exercise_dates: 50,
            seed: 0x4c534d_u64, // "LSM"
        }
    }
}

/// The Longstaff-Schwartz estimate: the present value and its Monte-Carlo
/// standard error (the standard deviation of the per-path discounted cashflow,
/// divided by `√paths`). The std-error is surfaced honestly on the wire's
/// `price_std_error`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LsmEstimate {
    /// Present value (domestic premium).
    pub price: f64,
    /// Standard error of the mean.
    pub std_error: f64,
}

/// Price an American/Bermudan vanilla by Longstaff-Schwartz regression
/// Monte-Carlo over Sobol+bridge GBM paths.
///
/// The exercise grid is the explicit Bermudan `dates` (sorted, in `(0, T]`, with
/// expiry appended) or, for an American option, `cfg.exercise_dates`
/// equally-spaced opportunities. Paths are simulated on a **uniform** fine grid
/// (the union resolution, so every exercise date snaps onto a grid node) via the
/// [`BrownianBridge`], which loads the dominant variance onto the best-distributed
/// low Sobol dimensions. At each exercise date the continuation value is regressed
/// on a degree-3 polynomial basis of the spot over the in-the-money paths
/// (Longstaff-Schwartz: only ITM paths inform the boundary), and the path is
/// stopped where intrinsic ≥ estimated continuation.
#[must_use]
pub fn american_lsm(i: &VanillaInputs, spec: &AmericanOption, cfg: LsmConfig) -> LsmEstimate {
    let dates = exercise_dates(spec, i.t, cfg.exercise_dates);
    let m = dates.len();
    let n_paths = cfg.paths.max(1);

    // Simulate on a uniform fine grid over [0, T] whose resolution is at least
    // the exercise-date count, and onto which every exercise date snaps to a node
    // (handles non-uniform Bermudan dates with one bridge construction).
    let steps = simulation_steps(&dates, i.t);
    let dt = i.t / steps as f64;
    // For each exercise date, the index of its node on the fine grid (1..=steps,
    // since fine-grid node `g` is at time `(g+1)·dt`, `g = 0..steps`, t_{steps-1}=T).
    let node_of_date: Vec<usize> = dates
        .iter()
        .map(|t| ((t / dt).round().max(1.0) as usize).min(steps) - 1)
        .collect();

    let drift = i.r_dom - i.r_for - 0.5 * i.vol * i.vol;
    let bridge = BrownianBridge::new(steps, i.t);
    let sobol = SobolSequence::new(steps);

    // Discount factors to each exercise date, taken at the SNAPPED node time so
    // discounting and the simulated spot use exactly the same instant (no drift
    // between the requested date and its grid node).
    let df: Vec<f64> = node_of_date
        .iter()
        .map(|&node| exp(-i.r_dom * (node + 1) as f64 * dt))
        .collect();

    // Simulate the spot matrix: paths × exercise-dates (only the exercise-date
    // columns are retained from each fine path).
    let mut spots = vec![0.0f64; n_paths * m];
    let mut z = vec![0.0f64; steps];
    let mut w = vec![0.0f64; steps];
    let mut stream = sobol.stream(cfg.seed);
    let mut u = vec![0.0f64; steps];
    for p in 0..n_paths {
        stream.next_point(&mut u);
        for k in 0..steps {
            z[k] = inv_norm_cdf(u[k]);
        }
        bridge.build(&z, &mut w); // W(t_g) correlated Brownian path on the fine grid
        for (k, &node) in node_of_date.iter().enumerate() {
            let t_node = (node + 1) as f64 * dt;
            // S(t) = S0 · exp(drift·t + σ·W(t)).
            spots[p * m + k] = i.spot * exp(drift * t_node + i.vol * w[node]);
        }
    }

    // Backward induction. `cashflow[p]` is the realised discounted-to-date payoff
    // the path collects, with `stop[p]` the date index it is collected at.
    let mut cashflow = vec![0.0f64; n_paths];
    let mut stop = vec![m - 1usize; n_paths];
    // Initialise at expiry (last date): exercise iff ITM (always optimal at T).
    for p in 0..n_paths {
        let s = spots[p * m + (m - 1)];
        cashflow[p] = intrinsic(spec.option, s, spec.strike);
        stop[p] = m - 1;
    }

    // Step backward through the interior exercise dates.
    for k in (0..m - 1).rev() {
        // In-the-money paths at date k inform the continuation regression.
        let mut itm: Vec<usize> = Vec::new();
        for p in 0..n_paths {
            let s = spots[p * m + k];
            if intrinsic(spec.option, s, spec.strike) > 0.0 {
                itm.push(p);
            }
        }
        if itm.is_empty() {
            continue;
        }
        // Regress the discounted (from the path's current stop date back to date
        // k) continuation cashflow on a degree-3 polynomial of **moneyness**
        // `x = S/K`. Normalising by the strike keeps the design matrix
        // well-conditioned (raw spot³ ≈ 10⁶ would swamp the constant column and
        // bias the least-squares solve, which in turn corrupts the optimal
        // stopping rule and low-biases the estimate).
        let inv_k = 1.0 / spec.strike;
        let basis = |s: f64| {
            let x = s * inv_k;
            [1.0, x, x * x, x * x * x]
        };
        let mut xs = Vec::with_capacity(itm.len());
        let mut ys = Vec::with_capacity(itm.len());
        for &p in &itm {
            let s = spots[p * m + k];
            // Continuation value at date k = the path's realised cashflow
            // discounted from its stop date back to date k.
            let cont = cashflow[p] * (df[stop[p]] / df[k]);
            xs.push(basis(s));
            ys.push(cont);
        }
        let beta = ridge_regress(&xs, &ys);
        for &p in &itm {
            let s = spots[p * m + k];
            let ex = intrinsic(spec.option, s, spec.strike);
            let est_cont = dot(&beta, &basis(s));
            if ex >= est_cont {
                // Exercise now: collect intrinsic at date k.
                cashflow[p] = ex;
                stop[p] = k;
            }
        }
    }

    // Present value: discount each path's cashflow from its stop date to today,
    // average, and compute the standard error of the mean.
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    for p in 0..n_paths {
        let pv = cashflow[p] * df[stop[p]];
        sum += pv;
        sum_sq += pv * pv;
    }
    let nf = n_paths as f64;
    let mean = sum / nf;
    let var = (sum_sq / nf - mean * mean).max(0.0);
    let std_error = sqrt(var / nf);
    LsmEstimate {
        price: mean,
        std_error,
    }
}

/// The exercise-date grid: the explicit Bermudan dates (sorted, `(0, T]`, expiry
/// appended) or `n` equally-spaced American opportunities `k/n · T` for
/// `k = 1..=n`.
fn exercise_dates(spec: &AmericanOption, t: f64, n: usize) -> Vec<f64> {
    match &spec.style {
        ExerciseStyle::American => {
            let n = n.max(1);
            (1..=n).map(|k| t * k as f64 / n as f64).collect()
        }
        ExerciseStyle::Bermudan { dates } => {
            let mut d: Vec<f64> = dates
                .iter()
                .copied()
                .filter(|x| *x > 0.0 && *x <= t + 1e-12)
                .map(|x| x.min(t))
                .collect();
            d.push(t); // expiry is always exercisable
            d.sort_by(|a, b| a.partial_cmp(b).unwrap());
            d.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
            d
        }
    }
}

/// The fine simulation-grid step count: dense enough that (a) each exercise date
/// snaps to a distinct node and (b) the bridge has enough resolution for an
/// accurate GBM path. Taken as the larger of the exercise-date count and a fixed
/// floor, then rounded up so every date lands on a node within `dt/2`.
fn simulation_steps(dates: &[f64], t: f64) -> usize {
    const FLOOR: usize = 64;
    let mut steps = dates.len().max(FLOOR);
    // Ensure the coarsest exercise-date spacing is resolved: if any consecutive
    // dates would collapse onto the same node, increase the resolution.
    let dt = t / steps as f64;
    let min_gap = dates
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(f64::INFINITY, f64::min);
    if min_gap.is_finite() && min_gap > 0.0 && min_gap < 2.0 * dt {
        steps = ((t / (min_gap * 0.5)).ceil() as usize).max(steps);
    }
    steps.max(1)
}

/// Dot product of two equal-length 4-vectors.
#[inline]
fn dot(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

/// Ordinary least squares (with a tiny ridge for numerical stability) of `ys` on
/// the 4-column design `xs`, by forming and solving the 4×4 normal equations
/// `(XᵀX + εI) β = Xᵀy` with Gaussian elimination. The ridge `ε` is negligible
/// relative to the design scale and only guards against a singular `XᵀX` when the
/// ITM spot range collapses; it does not bias the regression materially.
fn ridge_regress(xs: &[[f64; 4]], ys: &[f64]) -> [f64; 4] {
    let mut ata = [[0.0f64; 4]; 4];
    let mut aty = [0.0f64; 4];
    for (x, &y) in xs.iter().zip(ys.iter()) {
        for (r, (aty_r, ata_r)) in aty.iter_mut().zip(ata.iter_mut()).enumerate() {
            *aty_r += x[r] * y;
            for (c, ata_rc) in ata_r.iter_mut().enumerate() {
                *ata_rc += x[r] * x[c];
            }
        }
    }
    // Ridge scaled to the trace so it is dimensionally consistent.
    let trace = ata[0][0] + ata[1][1] + ata[2][2] + ata[3][3];
    let eps = 1e-12 * (trace + 1.0);
    for (r, row) in ata.iter_mut().enumerate() {
        row[r] += eps;
    }
    solve4(&mut ata, &mut aty)
}

/// Solve a 4×4 linear system by Gaussian elimination with partial pivoting.
/// Consumes the augmented matrix in place; returns the solution vector.
fn solve4(a: &mut [[f64; 4]; 4], b: &mut [f64; 4]) -> [f64; 4] {
    for col in 0..4 {
        // Partial pivot.
        let mut pivot = col;
        let mut best = a[col][col].abs();
        for (r, row) in a.iter().enumerate().skip(col + 1) {
            if row[col].abs() > best {
                best = row[col].abs();
                pivot = r;
            }
        }
        if pivot != col {
            a.swap(col, pivot);
            b.swap(col, pivot);
        }
        let diag = a[col][col];
        if diag.abs() < f64::MIN_POSITIVE {
            // Degenerate column (constant ITM set); leave the coefficient zero.
            continue;
        }
        // Split the matrix so the pivot row and the rows below it can be borrowed
        // disjointly (no aliasing) while eliminating the column.
        let (head, tail) = a.split_at_mut(col + 1);
        let pivot_row = &head[col];
        let b_pivot = b[col];
        for (r_off, row) in tail.iter_mut().enumerate() {
            let factor = row[col] / diag;
            if factor != 0.0 {
                for (rc, pc) in row.iter_mut().zip(pivot_row.iter()).skip(col) {
                    *rc -= factor * *pc;
                }
                b[col + 1 + r_off] -= factor * b_pivot;
            }
        }
    }
    let mut x = [0.0f64; 4];
    for col in (0..4).rev() {
        let s = b[col]
            - a[col]
                .iter()
                .enumerate()
                .skip(col + 1)
                .map(|(c, &v)| v * x[c])
                .sum::<f64>();
        x[col] = if a[col][col].abs() < f64::MIN_POSITIVE {
            0.0
        } else {
            s / a[col][col]
        };
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_vanilla::price as vanilla_price;

    fn base() -> VanillaInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)
    }

    fn amr(option: OptionType, strike: f64) -> AmericanOption {
        AmericanOption {
            option,
            strike,
            style: ExerciseStyle::American,
        }
    }

    /// American value is never below the European value (early exercise is an
    /// added right): the early-exercise premium is non-negative.
    #[test]
    fn american_dominates_european() {
        let i = base();
        let grid = AmericanGrid::default();
        for option in [OptionType::Call, OptionType::Put] {
            for k in [80.0, 100.0, 120.0] {
                let am = american_fd(&i, &amr(option, k), grid);
                let eu = vanilla_price(option, &VanillaInputs { strike: k, ..i });
                // Dominance holds up to the FD discretisation error (~1e-3): the
                // American value is never materially below the European closed
                // form, and is strictly above it wherever early exercise binds.
                assert!(
                    am >= eu - 5e-3,
                    "{option:?} K={k}: American {am} < European {eu}"
                );
            }
        }
    }

    /// No-early-exercise regime: an American CALL with `r_for = 0` is never worth
    /// exercising early, so it equals the European Garman-Kohlhagen value to grid
    /// tolerance. The discriminating structural oracle.
    #[test]
    fn american_call_no_foreign_rate_equals_european() {
        let i = VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.0);
        let grid = AmericanGrid {
            space_steps: 1200,
            time_steps: 600,
            ..AmericanGrid::default()
        };
        for k in [80.0, 100.0, 120.0] {
            let am = american_fd(&i, &amr(OptionType::Call, k), grid);
            let eu = vanilla_price(OptionType::Call, &VanillaInputs { strike: k, ..i });
            assert!(
                (am - eu).abs() < 5e-3,
                "American call (r_for=0) K={k}: {am} vs European {eu}"
            );
        }
    }

    /// Symmetric no-early-exercise regime: an American PUT with `r_dom = 0`
    /// equals the European value to grid tolerance.
    #[test]
    fn american_put_no_domestic_rate_equals_european() {
        let i = VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.0, 0.05);
        let grid = AmericanGrid {
            space_steps: 1200,
            time_steps: 600,
            ..AmericanGrid::default()
        };
        for k in [80.0, 100.0, 120.0] {
            let am = american_fd(&i, &amr(OptionType::Put, k), grid);
            let eu = vanilla_price(OptionType::Put, &VanillaInputs { strike: k, ..i });
            assert!(
                (am - eu).abs() < 5e-3,
                "American put (r_dom=0) K={k}: {am} vs European {eu}"
            );
        }
    }

    /// In a regime where early exercise IS optimal (an ITM put with a high
    /// domestic rate), the American put carries a strictly positive premium over
    /// European.
    #[test]
    fn american_put_has_positive_premium_when_optimal() {
        let i = VanillaInputs::new(100.0, 100.0, 0.30, 1.0, 0.10, 0.0);
        let grid = AmericanGrid::default();
        let am = american_fd(&i, &amr(OptionType::Put, 110.0), grid);
        let eu = vanilla_price(OptionType::Put, &VanillaInputs { strike: 110.0, ..i });
        assert!(
            am > eu + 1e-2,
            "American put {am} should exceed European {eu}"
        );
    }

    /// Hand-pinned published American value. Longstaff & Schwartz (2001),
    /// "Valuing American Options by Simulation", Table 1, first row: an American
    /// PUT with `S₀ = K = 40`, `r = 0.06`, `σ = 0.20`, `T = 1` (no dividend, so
    /// the FX foreign rate `r_f = 0`). Their **finite-difference** reference price
    /// is `2.314` (their independent LSM column reports `2.313`, s.e. `0.009`).
    /// Our PSOR free-boundary FD must reproduce that published value to FD
    /// tolerance — an independent oracle the gate pins by hand (no internal
    /// formula re-derived as its own check).
    #[test]
    fn matches_published_longstaff_schwartz_table1() {
        let i = VanillaInputs::new(40.0, 40.0, 0.20, 1.0, 0.06, 0.0);
        let grid = AmericanGrid {
            space_steps: 2000,
            time_steps: 2000,
            ..AmericanGrid::default()
        };
        let spec = amr(OptionType::Put, 40.0);
        let fd = american_fd(&i, &spec, grid);
        const PUBLISHED_FD: f64 = 2.314;
        assert!(
            (fd - PUBLISHED_FD).abs() < 1e-2,
            "American put (LS 2001 Table 1) FD {fd} vs published {PUBLISHED_FD}"
        );
    }

    /// PSOR FD ≈ Longstaff-Schwartz LSM within the MC standard error.
    #[test]
    fn fd_matches_lsm_within_stderr() {
        let i = VanillaInputs::new(100.0, 100.0, 0.25, 1.0, 0.08, 0.0);
        let spec = amr(OptionType::Put, 100.0);
        let fd = american_fd(&i, &spec, AmericanGrid::default());
        let lsm = american_lsm(
            &i,
            &spec,
            LsmConfig {
                paths: 200_000,
                exercise_dates: 50,
                seed: 0xABCD,
            },
        );
        let tol = 4.0 * lsm.std_error + 1e-2;
        assert!(
            (fd - lsm.price).abs() < tol,
            "FD {fd} vs LSM {} (se {}, tol {tol})",
            lsm.price,
            lsm.std_error
        );
        assert!(lsm.std_error > 0.0, "LSM must carry a positive std-error");
    }

    /// Bermudan with a single exercise date at expiry equals the European value;
    /// with a dense date set it approaches the American value.
    #[test]
    fn bermudan_endpoints() {
        let i = VanillaInputs::new(100.0, 100.0, 0.30, 1.0, 0.10, 0.0);
        let grid = AmericanGrid::default();

        // Single date at expiry ⇒ European.
        let one = AmericanOption {
            option: OptionType::Put,
            strike: 110.0,
            style: ExerciseStyle::Bermudan { dates: vec![1.0] },
        };
        let berm_one = american_fd(&i, &one, grid);
        let eu = vanilla_price(OptionType::Put, &VanillaInputs { strike: 110.0, ..i });
        assert!(
            (berm_one - eu).abs() < 5e-3,
            "Bermudan(1 date @ T) {berm_one} vs European {eu}"
        );

        // Dense date set ⇒ approaches American.
        let dense_dates: Vec<f64> = (1..=50).map(|k| k as f64 / 50.0).collect();
        let dense = AmericanOption {
            option: OptionType::Put,
            strike: 110.0,
            style: ExerciseStyle::Bermudan { dates: dense_dates },
        };
        let berm_dense = american_fd(&i, &dense, grid);
        let american = american_fd(&i, &amr(OptionType::Put, 110.0), grid);
        // 50 exercise dates approach (but do not reach) the continuous-exercise
        // American value; the residual is the coarser exercise frequency.
        assert!(
            berm_dense <= american + 1e-3 && (american - berm_dense) < 5e-2,
            "Bermudan(dense) {berm_dense} vs American {american}"
        );
        // Monotone: more dates ⇒ at least as valuable.
        assert!(berm_dense >= berm_one - 1e-3);
    }

    /// The FD Greek strip is finite and has the right signs for an American call
    /// (positive delta, positive gamma, positive vega).
    #[test]
    fn fd_greeks_are_sane() {
        let i = base();
        let g = american_fd_greeks(&i, &amr(OptionType::Call, 100.0), AmericanGrid::default());
        assert!(g.price > 0.0 && g.price.is_finite());
        assert!(g.delta_spot > 0.0, "call delta {}", g.delta_spot);
        assert!(g.gamma > 0.0, "gamma {}", g.gamma);
        assert!(g.vega > 0.0, "vega {}", g.vega);
    }
}
