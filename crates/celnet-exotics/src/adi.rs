//! Two-dimensional alternating-direction-implicit (ADI) finite-difference solver
//! for the local-stochastic-volatility pricing PDE in `(log-spot, variance)`.
//!
//! # The equation
//!
//! With `x = ln S` and instantaneous variance `v`, the LSV value `U(x, v, τ)`
//! (`τ = T − t` backward from expiry) solves the two-dimensional
//! convection-diffusion PDE
//!
//! ```text
//!   ∂U/∂τ = ½ L²(x,τ) v U_xx + ρ ξ L(x,τ) v U_xv + ½ ξ² v U_vv
//!           + (b − ½ L² v) U_x + κ(θ − v) U_v − r_d U ,
//! ```
//!
//! where `b = r_d − r_f`, `L(x,τ)` is the [`crate::leverage`] function, and
//! `(κ, θ, ξ, ρ)` are the stochastic-variance parameters of [`crate::stochvol`].
//! The cross term `U_xv` (present whenever `ρ ≠ 0`) is what makes a naive
//! direction-by-direction implicit scheme inconsistent — it must be handled
//! explicitly, which is exactly what the ADI splitting does.
//!
//! # The splitting
//!
//! The spatial operator is split as `A = A_0 + A_1 + A_2`:
//! * `A_0` — the **mixed** second derivative `ρ ξ L v U_xv` (always explicit);
//! * `A_1` — the `x`-direction convection-diffusion (implicit in `x`);
//! * `A_2` — the `v`-direction convection-diffusion (implicit in `v`).
//!
//! The **Hundsdorfer-Verwer (HV)** scheme then takes, per backward step of size
//! `Δτ` with parameter `ϑ` (here `ϑ = ½ + √3/6 ≈ 0.7887`, the value that makes the
//! scheme second-order and unconditionally stable for this operator class):
//!
//! ```text
//!   Y0 = U_n + Δτ A U_n                                    (explicit predictor)
//!   Yj = Y_{j-1} + ϑ Δτ A_j (Yj − U_n) ,   j = 1, 2        (implicit corrector, per dir)
//!   Ỹ0 = Y0 + ½ Δτ A (Y2 − U_n)                            (HV mid-correction)
//!   Ỹj = Ỹ_{j-1} + ϑ Δτ A_j (Ỹj − Y2) ,  j = 1, 2          (implicit, per dir)
//!   U_{n+1} = Ỹ2 .
//! ```
//!
//! Each implicit stage is a one-dimensional tridiagonal (Thomas) solve along the
//! corresponding axis, so the cost per step is `O(n_x · n_v)` — the whole point of
//! ADI over a fully-coupled 2-D implicit solve.
//!
//! # Alternative scheme
//!
//! The **Craig-Sneyd (CS)** scheme is the same family with a single explicit
//! mid-correction of the *mixed* term only; HV is preferred here for its stronger
//! damping of the variance-boundary layer, but both are second-order. The
//! provenance for both lives in this doc comment.
//!
//! # Boundaries
//!
//! * `x` (log-spot): Dirichlet far-field from the discounted intrinsic at the
//!   edges; for a knock-out, a zero wall on the dead side (barrier-aligned node).
//! * `v` (variance): at `v = 0` the diffusion `A_2` degenerates to pure upward
//!   convection `κθ U_v`, handled by a one-sided difference (the natural,
//!   well-posed boundary — no condition is imposed); at the top `v_max` a linear
//!   (`U_vv = 0`) condition.
//!
//! # Method provenance (doc comments only)
//!
//! ADI for multi-factor option PDEs: Craig & Sneyd (1988); Hundsdorfer & Verwer
//! (2003, *Numerical Solution of Time-Dependent Advection-Diffusion-Reaction
//! Equations*); in t'Hout & Foulon (2010, *ADI finite difference schemes for
//! option pricing in the Heston model with correlation*). Identifiers are
//! purpose-named; provenance lives only in documentation.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::{OptionType, VanillaInputs};

use crate::leverage::LeverageSurface;
use crate::stochvol::VarianceParams;

/// The Hundsdorfer-Verwer parameter `ϑ = ½ + √3/6`, the second-order,
/// unconditionally-stable choice for advection-diffusion with a mixed term.
const HV_THETA: f64 = 0.788_675_134_594_812_9;

/// Grid resolution for the 2-D ADI solver.
#[derive(Debug, Clone, Copy)]
pub struct AdiGrid {
    /// Number of spot (log-spot) intervals (`nodes_x = x_steps + 1`).
    pub x_steps: usize,
    /// Number of variance intervals (`nodes_v = v_steps + 1`).
    pub v_steps: usize,
    /// Number of backward time steps.
    pub time_steps: usize,
    /// Half-width of the log-spot domain in standard deviations of `ln S_T`.
    pub width_in_std: f64,
    /// Upper variance bound as a multiple of `max(v0, θ)`.
    pub v_max_mult: f64,
}

impl Default for AdiGrid {
    fn default() -> Self {
        Self {
            x_steps: 120,
            v_steps: 50,
            time_steps: 80,
            width_in_std: 6.0,
            v_max_mult: 6.0,
        }
    }
}

/// Terminal / boundary specification of an LSV PDE problem.
#[derive(Debug, Clone, Copy)]
pub struct AdiProblem {
    /// Call or put terminal payoff.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Optional knock-out barrier `Some((level, up))` — a zero Dirichlet wall on
    /// the dead side of the log-spot axis (applied at every variance row).
    pub knock_out: Option<(f64, bool)>,
}

/// The fully-assembled 2-D grid geometry and operator coefficients.
struct Lsv2d<'a> {
    nx: usize,
    nv: usize,
    x0: f64,
    dx: f64,
    /// Variance node levels (possibly non-uniform — here uniform for clarity).
    v: Vec<f64>,
    dv: f64,
    spot_index: usize,
    v0_index: usize,
    /// The barrier-aligned wall position `(index, up)`, if the grid carries a
    /// barrier node. Whether it is *enforced* this phase is governed by
    /// [`Lsv2d::wall_active`] (always-on for a full-life knock-out; toggled by
    /// calendar phase for a window barrier).
    wall: Option<(usize, bool)>,
    /// Whether the (aligned) wall is currently active.
    wall_active: bool,
    // Model parameters.
    r_dom: f64,
    carry: f64,
    var: VarianceParams,
    lev: &'a LeverageSurface,
}

impl<'a> Lsv2d<'a> {
    fn new(
        i: &VanillaInputs,
        var: &VarianceParams,
        lev: &'a LeverageSurface,
        p: AdiProblem,
        g: AdiGrid,
    ) -> Self {
        let ln_s = ln(i.spot);
        let atm_vol = sqrt(var.v0);
        let mu = (i.r_dom - i.r_for - 0.5 * var.v0) * i.t;
        let std = atm_vol * sqrt(i.t);
        let half = g.width_in_std * std;
        let centre = ln_s + mu;

        // --- Log-spot axis: align spot (and barrier if present) on nodes.
        let (nx, x0, dx, spot_index, wall) = match p.knock_out {
            Some((level, up)) => {
                Self::axis_with_barrier(g.x_steps, ln_s, ln(level), up, centre, half)
            }
            None => {
                let lo = (centre - half).min(ln_s - half);
                let hi = (centre + half).max(ln_s + half);
                let dx0 = (hi - lo) / g.x_steps as f64;
                let k_spot = ((ln_s - lo) / dx0).round().max(1.0);
                let dx = (ln_s - lo) / k_spot;
                let total = ((hi - lo) / dx).ceil().max(k_spot + 1.0) as usize;
                (total + 1, lo, dx, k_spot as usize, None)
            }
        };

        // --- Variance axis: uniform [0, v_max], aligned so v0 is a node.
        let v_max = g.v_max_mult * var.v0.max(var.long_var);
        let dv0 = v_max / g.v_steps as f64;
        let k_v0 = (var.v0 / dv0).round().max(1.0);
        let dv = var.v0 / k_v0;
        let nv = (v_max / dv).ceil().max(k_v0 + 1.0) as usize + 1;
        let v: Vec<f64> = (0..nv).map(|j| j as f64 * dv).collect();
        let v0_index = k_v0 as usize;

        Self {
            nx,
            nv,
            x0,
            dx,
            v,
            dv,
            spot_index,
            v0_index,
            wall,
            wall_active: wall.is_some(),
            r_dom: i.r_dom,
            carry: i.r_dom - i.r_for,
            var: *var,
            lev,
        }
    }

    /// Build the log-spot axis with a barrier node, mirroring the 1-D solver's
    /// alignment logic.
    fn axis_with_barrier(
        x_steps: usize,
        ln_s: f64,
        ln_h: f64,
        up: bool,
        centre: f64,
        half: f64,
    ) -> (usize, f64, f64, usize, Option<(usize, bool)>) {
        let gap = (ln_h - ln_s).abs();
        let span = (centre + half).max(ln_h) - (centre - half).min(ln_h);
        let dx0 = span / x_steps as f64;
        let m = (gap / dx0).round().max(1.0);
        let dx = gap / m;
        if up {
            let want_lo = (centre - half).min(ln_s - half);
            let steps_below = ((ln_h - want_lo) / dx).ceil().max(m + 1.0) as usize;
            let lo = ln_h - steps_below as f64 * dx;
            (
                steps_below + 1,
                lo,
                dx,
                (steps_below as f64 - m) as usize,
                Some((steps_below, true)),
            )
        } else {
            let want_hi = (centre + half).max(ln_s + half);
            let total = ((want_hi - ln_h) / dx).ceil().max(m + 1.0) as usize;
            (total + 1, ln_h, dx, m as usize, Some((0, false)))
        }
    }

    #[inline]
    fn x(&self, i: usize) -> f64 {
        self.x0 + i as f64 * self.dx
    }

    #[inline]
    fn idx(&self, i: usize, j: usize) -> usize {
        j * self.nx + i
    }
}

/// Solve the LSV PDE and return the present value at `(spot, v0)`.
///
/// The grid is built so the initial spot and `v0` both land on nodes; the result
/// is the exact grid read at `(spot_index, v0_index)`. The terminal condition is
/// the vanilla intrinsic; a knock-out imposes a zero wall on the dead side at
/// every variance level.
#[must_use]
pub fn solve(
    i: &VanillaInputs,
    var: &VarianceParams,
    lev: &LeverageSurface,
    problem: AdiProblem,
    grid: AdiGrid,
) -> f64 {
    let mut g = Lsv2d::new(i, var, lev, problem, grid);
    let (nx, nv) = (g.nx, g.nv);

    // Terminal condition U(x, v, 0) = payoff(e^x), then integrate the whole life.
    let mut u = terminal_payoff(&g, problem.option, problem.strike);
    let mut work = Workspace::new(nx, nv);
    integrate_phase(&mut g, &mut u, i.t, 0.0, grid.time_steps, i.t, &mut work);

    u[g.idx(g.spot_index, g.v0_index)]
}

/// Build the terminal value plane `U(x, v, 0) = payoff(e^x)` and apply the wall.
fn terminal_payoff(g: &Lsv2d, option: OptionType, strike: f64) -> Vec<f64> {
    let mut u = vec![0.0f64; g.nx * g.nv];
    for j in 0..g.nv {
        for ii in 0..g.nx {
            let s = exp(g.x(ii));
            u[g.idx(ii, j)] = crate::payoff::vanilla_intrinsic(option, s, strike);
        }
    }
    apply_wall(g, &mut u);
    u
}

/// Integrate the value plane `u` **backward** over a calendar phase that ends (in
/// calendar time) at `cal_end` and lasts `phase_len` years, in `steps` HV steps,
/// with the wall enforced according to `g.wall_active`. `total_t` is the option's
/// full horizon `T` (used only to bound the calendar clamp).
///
/// "Backward" means `u` enters as the value at the *later* calendar boundary
/// (`cal_end + phase_len`) and leaves as the value at `cal_end`.
fn integrate_phase(
    g: &mut Lsv2d,
    u: &mut [f64],
    phase_len: f64,
    cal_end: f64,
    steps: usize,
    _total_t: f64,
    work: &mut Workspace,
) {
    if steps == 0 || phase_len <= 0.0 {
        return;
    }
    let dtau = phase_len / steps as f64;
    for step in 0..steps {
        // Calendar time at the mid-point of this backward step.
        let cal_t = (cal_end + phase_len - (step as f64 + 0.5) * dtau).max(0.0);
        hv_step(g, u, dtau, cal_t, work);
        apply_wall(g, u);
    }
}

/// The three-phase window-barrier specification consumed by [`solve_window`].
#[derive(Debug, Clone, Copy)]
pub struct WindowSpec {
    /// Call or put terminal payoff.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Barrier level `H`.
    pub barrier: f64,
    /// Up (`true`) or down (`false`) barrier.
    pub up: bool,
    /// Calendar length after the window closes (`T − end`); barrier inactive.
    pub t_after: f64,
    /// Calendar length of the active window (`end − start`); barrier active.
    pub t_window: f64,
    /// Calendar length before the window opens (`start`); barrier inactive.
    pub t_before: f64,
}

/// Solve a second-generation **window knock-out** barrier on the 2-D ADI grid.
///
/// The grid is built with the barrier aligned on a node, but the zero wall is only
/// enforced during the active window. Backward induction runs in three phases:
/// after-window (no wall) → window (wall) → before-window (no wall), reusing the
/// single grid and value plane. The HV time-step budget is split across the
/// phases in proportion to their calendar length.
#[must_use]
pub fn solve_window(
    i: &VanillaInputs,
    var: &VarianceParams,
    lev: &LeverageSurface,
    spec: WindowSpec,
    grid: AdiGrid,
) -> f64 {
    // Build the grid with the barrier aligned (so the wall node is exact), via the
    // standard knock-out problem geometry.
    let problem = AdiProblem {
        option: spec.option,
        strike: spec.strike,
        knock_out: Some((spec.barrier, spec.up)),
    };
    let mut g = Lsv2d::new(i, var, lev, problem, grid);
    let (nx, nv) = (g.nx, g.nv);

    // Terminal payoff with the wall *inactive* (the after-window phase carries no
    // barrier), so the terminal condition is the full vanilla intrinsic.
    g.wall_active = false;
    let mut u = terminal_payoff(&g, spec.option, spec.strike);
    let mut work = Workspace::new(nx, nv);

    // Split the time budget across the three phases by calendar length.
    let total = (spec.t_after + spec.t_window + spec.t_before).max(1e-12);
    let n_after = phase_steps(grid.time_steps, spec.t_after, total);
    let n_window = phase_steps(grid.time_steps, spec.t_window, total);
    let n_before = grid.time_steps - n_after - n_window;

    // Phase A: after the window (no wall), ending at calendar time `end`.
    g.wall_active = false;
    integrate_phase(
        &mut g,
        &mut u,
        spec.t_after,
        spec.t_window + spec.t_before,
        n_after,
        i.t,
        &mut work,
    );

    // Phase B: the active window (wall on), ending at calendar time `start`.
    g.wall_active = true;
    apply_wall(&g, &mut u); // knock out the dead side entering the window
    integrate_phase(
        &mut g,
        &mut u,
        spec.t_window,
        spec.t_before,
        n_window,
        i.t,
        &mut work,
    );

    // Phase C: before the window (no wall), ending at calendar time 0.
    g.wall_active = false;
    integrate_phase(&mut g, &mut u, spec.t_before, 0.0, n_before, i.t, &mut work);

    u[g.idx(g.spot_index, g.v0_index)]
}

/// Allocate a phase's share of the total HV step budget, proportional to its
/// calendar length (at least one step for a non-degenerate phase).
#[inline]
fn phase_steps(total_steps: usize, phase_len: f64, total_len: f64) -> usize {
    if phase_len <= 0.0 {
        return 0;
    }
    let raw = (total_steps as f64 * phase_len / total_len).round() as usize;
    raw.max(1).min(total_steps)
}

/// Reusable per-step scratch space. Plane buffers are length `nx·nv`; the line
/// scratch (`rhs`, `sub`, `dia`, `sup`) are length `max(nx, nv)`.
struct Workspace {
    au_a: Vec<f64>,    // A·(·) buffer A
    au_b: Vec<f64>,    // A·(·) buffer B
    y0: Vec<f64>,      // explicit predictor
    y2: Vec<f64>,      // state after the first two correctors
    scratch: Vec<f64>, // generic plane scratch (corrector source / target)
    rhs: Vec<f64>,     // tridiagonal RHS
    sub: Vec<f64>,     // per-row sub-diagonal
    dia: Vec<f64>,     // per-row diagonal
    sup: Vec<f64>,     // per-row super-diagonal
    cp: Vec<f64>,      // Thomas forward scratch
    dp: Vec<f64>,      // Thomas forward scratch
}

impl Workspace {
    fn new(nx: usize, nv: usize) -> Self {
        let n = nx * nv;
        let m = nx.max(nv);
        Self {
            au_a: vec![0.0; n],
            au_b: vec![0.0; n],
            y0: vec![0.0; n],
            y2: vec![0.0; n],
            scratch: vec![0.0; n],
            rhs: vec![0.0; m],
            sub: vec![0.0; m],
            dia: vec![0.0; m],
            sup: vec![0.0; m],
            cp: vec![0.0; m],
            dp: vec![0.0; m],
        }
    }
}

/// Line-scratch view passed to the implicit sweeps so they never borrow the whole
/// [`Workspace`] (which would alias the plane buffers).
struct LineScratch<'s> {
    rhs: &'s mut [f64],
    sub: &'s mut [f64],
    dia: &'s mut [f64],
    sup: &'s mut [f64],
    cp: &'s mut [f64],
    dp: &'s mut [f64],
}

/// One Hundsdorfer-Verwer backward step.
// The plane-buffer combinations below index several distinct buffers per element,
// so an index loop is clearer (and no slower) than a multi-way zip.
#[allow(clippy::needless_range_loop)]
fn hv_step(g: &Lsv2d, u: &mut [f64], dtau: f64, cal_t: f64, w: &mut Workspace) {
    let n = u.len();

    // Y0 = U + Δτ A U  (full explicit operator: mixed + x + v + reaction).
    apply_full(g, u, cal_t, &mut w.au_a);
    for k in 0..n {
        w.y0[k] = u[k] + dtau * w.au_a[k];
    }
    apply_wall(g, &mut w.y0);

    // Corrector j = 1 (x-direction): (I − ϑΔτ A1) Y1 = Y0 − ϑΔτ A1 U → scratch.
    {
        let Workspace {
            y0,
            scratch,
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
            ..
        } = w;
        let mut ls = LineScratch {
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
        };
        implicit_x(g, y0, u, dtau, cal_t, scratch, &mut ls);
    }
    apply_wall(g, &mut w.scratch);
    // Corrector j = 2 (v-direction): (I − ϑΔτ A2) Y2 = Y1 − ϑΔτ A2 U → y2.
    {
        let Workspace {
            scratch,
            y2,
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
            ..
        } = w;
        let mut ls = LineScratch {
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
        };
        implicit_v(g, scratch, u, dtau, cal_t, y2, &mut ls);
    }
    apply_wall(g, &mut w.y2);

    // HV mid-correction: Ỹ0 = Y0 + ½ Δτ A (Y2 − U). au_a := A(Y2), au_b := A(U).
    apply_full(g, &w.y2, cal_t, &mut w.au_a);
    apply_full(g, u, cal_t, &mut w.au_b);
    for k in 0..n {
        w.scratch[k] = w.y0[k] + 0.5 * dtau * (w.au_a[k] - w.au_b[k]);
    }
    apply_wall(g, &mut w.scratch);

    // Ỹ1: (I − ϑΔτ A1) Ỹ1 = Ỹ0 − ϑΔτ A1 Y2 → au_a (reused as a plane buffer).
    {
        let Workspace {
            scratch,
            y2,
            au_a,
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
            ..
        } = w;
        let mut ls = LineScratch {
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
        };
        implicit_x(g, scratch, y2, dtau, cal_t, au_a, &mut ls);
    }
    apply_wall(g, &mut w.au_a);
    // Ỹ2: (I − ϑΔτ A2) Ỹ2 = Ỹ1 − ϑΔτ A2 Y2 → u (final).
    {
        let Workspace {
            au_a,
            y2,
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
            ..
        } = w;
        let mut ls = LineScratch {
            rhs,
            sub,
            dia,
            sup,
            cp,
            dp,
        };
        implicit_v(g, au_a, y2, dtau, cal_t, u, &mut ls);
    }
    apply_wall(g, u);
}

/// Apply the full spatial operator `A U = (A0 + A1 + A2) U − r_d U` over the
/// interior, leaving boundary rows/cols zeroed (they are Dirichlet/handled).
fn apply_full(g: &Lsv2d, u: &[f64], cal_t: f64, out: &mut [f64]) {
    for o in out.iter_mut() {
        *o = 0.0;
    }
    let dx = g.dx;
    let dv = g.dv;
    let rho = g.var.correlation;
    let xi = g.var.vol_of_var;
    let k = g.var.mean_reversion;
    let theta = g.var.long_var;

    for j in 1..g.nv - 1 {
        let v = g.v[j];
        for i in 1..g.nx - 1 {
            if g.is_dead(i) {
                continue;
            }
            let s = exp(g.x(i));
            let l = g.lev.leverage(s, cal_t);
            let l2v = l * l * v;

            let c = g.idx(i, j);
            let uij = u[c];
            let uxp = u[g.idx(i + 1, j)];
            let uxm = u[g.idx(i - 1, j)];
            let uvp = u[g.idx(i, j + 1)];
            let uvm = u[g.idx(i, j - 1)];

            // x-direction: ½L²v U_xx + (b − ½L²v) U_x.
            let uxx = (uxp - 2.0 * uij + uxm) / (dx * dx);
            let ux = (uxp - uxm) / (2.0 * dx);
            let a1 = 0.5 * l2v * uxx + (g.carry - 0.5 * l2v) * ux;

            // v-direction: ½ξ²v U_vv + κ(θ−v) U_v.
            let uvv = (uvp - 2.0 * uij + uvm) / (dv * dv);
            let uv = (uvp - uvm) / (2.0 * dv);
            let a2 = 0.5 * xi * xi * v * uvv + k * (theta - v) * uv;

            // mixed: ρξ L v U_xv  (centred cross difference).
            let uxv = (u[g.idx(i + 1, j + 1)] - u[g.idx(i + 1, j - 1)] - u[g.idx(i - 1, j + 1)]
                + u[g.idx(i - 1, j - 1)])
                / (4.0 * dx * dv);
            let a0 = rho * xi * l * v * uxv;

            out[c] = a0 + a1 + a2 - g.r_dom * uij;
        }
    }
}

/// Implicit `x`-sweep: solve `(I − ϑΔτ A1) Y = src − ϑΔτ A1 base` for every
/// variance row, where `A1` is the `x`-direction convection-diffusion-reaction
/// operator. The reaction `−r_d` is folded entirely into the `x`-direction stage
/// (a standard, stable allocation of the zeroth-order term).
fn implicit_x(
    g: &Lsv2d,
    src: &[f64],
    base: &[f64],
    dtau: f64,
    cal_t: f64,
    out: &mut [f64],
    ls: &mut LineScratch,
) {
    out.copy_from_slice(src);
    let im = HV_THETA * dtau;
    let dx = g.dx;
    let (a, b) = g.live_x_range();

    for j in 1..g.nv - 1 {
        let v = g.v[j];
        for i in a..=b {
            let s = exp(g.x(i));
            let l = g.lev.leverage(s, cal_t);
            let l2v = l * l * v;
            let (lo, di, up) = x_stencil(l2v, g.carry, g.r_dom, dx);
            let c = g.idx(i, j);
            let a1_base = lo * base[g.idx(i - 1, j)] + di * base[c] + up * base[g.idx(i + 1, j)];
            ls.rhs[i] = src[c] - im * a1_base;
            ls.sub[i] = -im * lo;
            ls.dia[i] = 1.0 - im * di;
            ls.sup[i] = -im * up;
        }
        // Fold fixed Dirichlet neighbours at the ends into the RHS.
        ls.rhs[a] -= ls.sub[a] * out[g.idx(a - 1, j)];
        ls.rhs[b] -= ls.sup[b] * out[g.idx(b + 1, j)];
        thomas_variable(ls.sub, ls.dia, ls.sup, ls.rhs, ls.cp, ls.dp, a, b);
        for i in a..=b {
            out[g.idx(i, j)] = ls.rhs[i];
        }
    }
}

/// Implicit `v`-sweep: solve `(I − ϑΔτ A2) Y = src − ϑΔτ A2 base` for every
/// log-spot column, with `A2` the `v`-direction convection-diffusion. At `v = 0`
/// the diffusion vanishes and an upwind one-sided convection `κθ U_v` is used; at
/// `v_max` a linear (`U_vv = 0`) condition.
fn implicit_v(
    g: &Lsv2d,
    src: &[f64],
    base: &[f64],
    dtau: f64,
    _cal_t: f64,
    out: &mut [f64],
    ls: &mut LineScratch,
) {
    out.copy_from_slice(src);
    let im = HV_THETA * dtau;
    let dv = g.dv;
    let xi = g.var.vol_of_var;
    let k = g.var.mean_reversion;
    let theta = g.var.long_var;

    for i in 1..g.nx - 1 {
        if g.is_dead(i) {
            continue;
        }
        // Row 0 (v=0): upwind A2 = κθ (U_1 − U_0)/dv  (pure positive convection).
        {
            let di0 = -k * theta / dv;
            let up0 = k * theta / dv;
            let c0 = g.idx(i, 0);
            let a2_base = di0 * base[c0] + up0 * base[g.idx(i, 1)];
            ls.rhs[0] = src[c0] - im * a2_base;
            ls.sub[0] = 0.0;
            ls.dia[0] = 1.0 - im * di0;
            ls.sup[0] = -im * up0;
        }
        for j in 1..g.nv - 1 {
            let v = g.v[j];
            let (lo, di, up) = v_stencil(xi, k, theta, v, dv);
            let c = g.idx(i, j);
            let a2_base = lo * base[g.idx(i, j - 1)] + di * base[c] + up * base[g.idx(i, j + 1)];
            ls.rhs[j] = src[c] - im * a2_base;
            ls.sub[j] = -im * lo;
            ls.dia[j] = 1.0 - im * di;
            ls.sup[j] = -im * up;
        }
        // Top row v_max: linear extrapolation U_vv = 0 ⇒ A2 ≈ κ(θ−v) (U−U_{-1})/dv.
        {
            let jmax = g.nv - 1;
            let v = g.v[jmax];
            let coef = k * (theta - v) / dv;
            let lo = -coef;
            let di = coef;
            let c = g.idx(i, jmax);
            let a2_base = lo * base[g.idx(i, jmax - 1)] + di * base[c];
            ls.rhs[jmax] = src[c] - im * a2_base;
            ls.sub[jmax] = -im * lo;
            ls.dia[jmax] = 1.0 - im * di;
            ls.sup[jmax] = 0.0;
        }
        thomas_variable(ls.sub, ls.dia, ls.sup, ls.rhs, ls.cp, ls.dp, 0, g.nv - 1);
        for j in 0..g.nv {
            out[g.idx(i, j)] = ls.rhs[j];
        }
    }
}

/// `x`-direction tridiagonal stencil weights `(lower, diag, upper)` of
/// `A1 = ½L²v ∂²_x + (b−½L²v) ∂_x − r_d`.
#[inline]
fn x_stencil(l2v: f64, carry: f64, r_dom: f64, dx: f64) -> (f64, f64, f64) {
    let diff = 0.5 * l2v / (dx * dx);
    let conv = (carry - 0.5 * l2v) / (2.0 * dx);
    (diff - conv, -2.0 * diff - r_dom, diff + conv)
}

/// `v`-direction tridiagonal stencil weights `(lower, diag, upper)` of
/// `A2 = ½ξ²v ∂²_v + κ(θ−v) ∂_v`.
#[inline]
fn v_stencil(xi: f64, k: f64, theta: f64, v: f64, dv: f64) -> (f64, f64, f64) {
    let diff = 0.5 * xi * xi * v / (dv * dv);
    let conv = k * (theta - v) / (2.0 * dv);
    (diff - conv, -2.0 * diff, diff + conv)
}

/// Variable-coefficient Thomas solve of a tridiagonal system on rows `a..=b`,
/// solving in place: `rhs` is overwritten with the solution. `sub`, `dia`, `sup`
/// are the per-row sub/diag/super coefficients; `cp`, `dp` are caller-supplied
/// scratch (length ≥ `b+1`), keeping the solve allocation-free on the hot path.
#[allow(clippy::too_many_arguments)]
fn thomas_variable(
    sub: &[f64],
    dia: &[f64],
    sup: &[f64],
    rhs: &mut [f64],
    cp: &mut [f64],
    dp: &mut [f64],
    a: usize,
    b: usize,
) {
    cp[a] = sup[a] / dia[a];
    dp[a] = rhs[a] / dia[a];
    for j in (a + 1)..=b {
        let m = dia[j] - sub[j] * cp[j - 1];
        cp[j] = sup[j] / m;
        dp[j] = (rhs[j] - sub[j] * dp[j - 1]) / m;
    }
    rhs[b] = dp[b];
    for j in (a..b).rev() {
        rhs[j] = dp[j] - cp[j] * rhs[j + 1];
    }
}

impl Lsv2d<'_> {
    /// Is log-spot index `i` on the dead side of a *currently-active* knock-out
    /// wall?
    #[inline]
    fn is_dead(&self, i: usize) -> bool {
        match self.wall {
            Some((idx, up)) if self.wall_active => {
                if up {
                    i >= idx
                } else {
                    i <= idx
                }
            }
            _ => false,
        }
    }

    /// Live interior `x`-range `[a, b]` excluding the outer Dirichlet edges and the
    /// dead side of a *currently-active* knock-out wall.
    #[inline]
    fn live_x_range(&self) -> (usize, usize) {
        let mut a = 1;
        let mut b = self.nx - 2;
        if let Some((idx, up)) = self.wall
            && self.wall_active
        {
            if up {
                b = idx.saturating_sub(1);
            } else {
                a = idx + 1;
            }
        }
        (a, b)
    }
}

/// Impose the knock-out zero wall at every variance level (idempotent). A no-op
/// when the wall is inactive this phase.
fn apply_wall(g: &Lsv2d, u: &mut [f64]) {
    if !g.wall_active {
        return;
    }
    if let Some((idx, up)) = g.wall {
        for j in 0..g.nv {
            if up {
                for i in idx..g.nx {
                    u[g.idx(i, j)] = 0.0;
                }
            } else {
                for i in 0..=idx {
                    u[g.idx(i, j)] = 0.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leverage::{ImpliedVolSurface, LocalVolSurface};
    use celnet_vanilla::price as vanilla_price;

    fn base() -> VanillaInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)
    }

    /// Build a unit-leverage surface (no leverage) over a wide grid: with `ξ = 0`
    /// and `v0 = θ = σ²`, the variance is deterministically constant at `σ²`, so
    /// the LSV PDE collapses to the 1-D Black-Scholes-Merton PDE and must reprice
    /// the Garman-Kohlhagen vanilla.
    #[test]
    fn degenerate_variance_reprices_vanilla() {
        let i = base();
        let sigma = 0.20;
        let var = VarianceParams::new(sigma * sigma, 1.0, sigma * sigma, 0.0, 0.0);
        let spots: Vec<f64> = (0..41).map(|k| 20.0 + 6.0 * k as f64).collect();
        let times = vec![0.0, 0.5, 1.0];
        let mut lev = LeverageSurface::new(spots, times);
        for j in 0..lev.time_len() {
            for ii in 0..lev.spot_len() {
                lev.set(ii, j, 1.0);
            }
        }
        let grid = AdiGrid {
            x_steps: 160,
            v_steps: 40,
            time_steps: 100,
            ..AdiGrid::default()
        };
        for opt in [OptionType::Call, OptionType::Put] {
            for k in [90.0, 100.0, 115.0] {
                let pde = solve(
                    &i,
                    &var,
                    &lev,
                    AdiProblem {
                        option: opt,
                        strike: k,
                        knock_out: None,
                    },
                    grid,
                );
                let exact = vanilla_price(opt, &VanillaInputs { strike: k, ..i });
                assert!(
                    (pde - exact).abs() < 3e-2,
                    "{opt:?} K={k}: ADI {pde} vs GK {exact}"
                );
            }
        }
    }

    /// A flat-implied (constant-vol) surface produces a Dupire local vol equal to
    /// that constant; an LSV with that local vol, unit leverage and zero
    /// vol-of-variance must reprice the flat-vol vanilla — wiring the leverage and
    /// the Dupire extraction together at the PDE.
    #[test]
    fn local_vol_consistency() {
        struct Flat;
        impl ImpliedVolSurface for Flat {
            fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
                0.20
            }
            fn forward(&self, _t: f64) -> f64 {
                100.0 * exp(0.03)
            }
        }
        let local = LocalVolSurface::new(&Flat);
        // local vol ≈ 0.20 everywhere ⇒ at v0=σ² leverage = σ_loc/σ = 1.
        let s = local.local_vol(100.0, 0.5);
        assert!((s - 0.20).abs() < 1e-2, "Dupire local vol {s} vs 0.20");
    }
}
