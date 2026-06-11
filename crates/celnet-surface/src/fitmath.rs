//! Deterministic damped Gauss-Newton solvers (libm only).
//!
//! Each solver minimises the sum of squared residuals over a fixed parameter set
//! by repeatedly forming the finite-difference Jacobian Jᵀ, solving the normal
//! equations (JᵀJ + λI)·δ = −Jᵀr with a small Levenberg damping λ, projecting the
//! step into the admissible box, and accepting it only if it reduces the residual
//! norm (otherwise the damping is increased). The iteration count is fixed so the
//! solve is bit-reproducible; the residual-decrease guard makes it robust.
//!
//! The kit is quote-basis-neutral: nothing in it knows about FX delta pillars or
//! strike grids. It was hoisted **verbatim** out of [`crate::calibrate`]
//! (CRYPTO-SURFACE-LEAF-SPEC §2.3 — pure code motion, identical token sequence
//! inside each function, gated by the frozen-bits FX regression in
//! `tests/fx_fit_pin.rs`) so the strike-axis front-end ([`crate::strike_quotes`])
//! reuses the same gated numerics as the FX delta front-end.

/// Maximum Gauss-Newton iterations for the SABR / SVI fits.
pub(crate) const FIT_ITERS: usize = 60;
/// Finite-difference step for the parameter Jacobians.
pub(crate) const FIT_FD_H: f64 = 1e-6;

/// Clamp a value into `[lo, hi]` (deterministic; libm-free).
#[inline]
pub(crate) fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

/// Sum of squares of a residual vector.
pub(crate) fn sumsq(r: &[f64]) -> f64 {
    r.iter().map(|x| x * x).sum()
}

/// A 2-parameter damped Gauss-Newton solve with a projection.
pub(crate) fn gauss_newton_2<R, P>(
    p0: &mut f64,
    p1: &mut f64,
    residuals: R,
    _b0: (f64, f64),
    project: P,
) where
    R: Fn(f64, f64) -> Vec<f64>,
    P: Fn(f64, f64) -> (f64, f64),
{
    let mut lambda = 1e-3;
    let mut r = residuals(*p0, *p1);
    let mut cost = sumsq(&r);
    for _ in 0..FIT_ITERS {
        let n = r.len();
        // Jacobian columns by forward differences.
        let r0 = residuals(*p0 + FIT_FD_H, *p1);
        let r1 = residuals(*p0, *p1 + FIT_FD_H);
        let mut jtj = [[0.0_f64; 2]; 2];
        let mut jtr = [0.0_f64; 2];
        for i in 0..n {
            let j0 = (r0[i] - r[i]) / FIT_FD_H;
            let j1 = (r1[i] - r[i]) / FIT_FD_H;
            jtj[0][0] += j0 * j0;
            jtj[0][1] += j0 * j1;
            jtj[1][0] += j1 * j0;
            jtj[1][1] += j1 * j1;
            jtr[0] += j0 * r[i];
            jtr[1] += j1 * r[i];
        }
        // (JᵀJ + λI) δ = −Jᵀr.
        let a = jtj[0][0] + lambda;
        let d = jtj[1][1] + lambda;
        let bc = jtj[0][1];
        let det = a * d - bc * bc;
        if det.abs() < 1e-300 {
            break;
        }
        let dx0 = -(d * jtr[0] - bc * jtr[1]) / det;
        let dx1 = -(a * jtr[1] - bc * jtr[0]) / det;
        let (np0, np1) = project(*p0 + dx0, *p1 + dx1);
        let nr = residuals(np0, np1);
        let ncost = sumsq(&nr);
        if ncost < cost {
            *p0 = np0;
            *p1 = np1;
            r = nr;
            cost = ncost;
            lambda = (lambda * 0.5).max(1e-9);
        } else {
            lambda *= 4.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
}

/// A 3-parameter damped Gauss-Newton solve with per-parameter box clamps.
pub(crate) fn gauss_newton_3<R>(
    p0: &mut f64,
    p1: &mut f64,
    p2: &mut f64,
    residuals: R,
    b0: (f64, f64),
    b1: (f64, f64),
    b2: (f64, f64),
) where
    R: Fn(f64, f64, f64) -> Vec<f64>,
{
    let project = |a: f64, b: f64, c: f64| {
        (
            clamp(a, b0.0, b0.1),
            clamp(b, b1.0, b1.1),
            clamp(c, b2.0, b2.1),
        )
    };
    let mut lambda = 1e-3;
    let mut r = residuals(*p0, *p1, *p2);
    let mut cost = sumsq(&r);
    for _ in 0..FIT_ITERS {
        let n = r.len();
        let ra = residuals(*p0 + FIT_FD_H, *p1, *p2);
        let rb = residuals(*p0, *p1 + FIT_FD_H, *p2);
        let rc = residuals(*p0, *p1, *p2 + FIT_FD_H);
        let mut jtj = [[0.0_f64; 3]; 3];
        let mut jtr = [0.0_f64; 3];
        for i in 0..n {
            let j = [
                (ra[i] - r[i]) / FIT_FD_H,
                (rb[i] - r[i]) / FIT_FD_H,
                (rc[i] - r[i]) / FIT_FD_H,
            ];
            for a in 0..3 {
                jtr[a] += j[a] * r[i];
                for b in 0..3 {
                    jtj[a][b] += j[a] * j[b];
                }
            }
        }
        for (a, row) in jtj.iter_mut().enumerate() {
            row[a] += lambda;
        }
        let Some(delta) = solve3(&jtj, &[-jtr[0], -jtr[1], -jtr[2]]) else {
            break;
        };
        let (np0, np1, np2) = project(*p0 + delta[0], *p1 + delta[1], *p2 + delta[2]);
        let nr = residuals(np0, np1, np2);
        let ncost = sumsq(&nr);
        if ncost < cost {
            *p0 = np0;
            *p1 = np1;
            *p2 = np2;
            r = nr;
            cost = ncost;
            lambda = (lambda * 0.5).max(1e-9);
        } else {
            lambda *= 4.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
}

/// A 4-parameter damped Gauss-Newton solve with a custom projection.
pub(crate) fn gauss_newton_4<R, P>(
    p0: &mut f64,
    p1: &mut f64,
    p2: &mut f64,
    p3: &mut f64,
    residuals: R,
    project: P,
) where
    R: Fn(f64, f64, f64, f64) -> Vec<f64>,
    P: Fn(f64, f64, f64, f64) -> (f64, f64, f64, f64),
{
    let mut lambda = 1e-3;
    let mut r = residuals(*p0, *p1, *p2, *p3);
    let mut cost = sumsq(&r);
    for _ in 0..FIT_ITERS {
        let n = r.len();
        let rcols = [
            residuals(*p0 + FIT_FD_H, *p1, *p2, *p3),
            residuals(*p0, *p1 + FIT_FD_H, *p2, *p3),
            residuals(*p0, *p1, *p2 + FIT_FD_H, *p3),
            residuals(*p0, *p1, *p2, *p3 + FIT_FD_H),
        ];
        let mut jtj = [[0.0_f64; 4]; 4];
        let mut jtr = [0.0_f64; 4];
        for i in 0..n {
            let mut j = [0.0_f64; 4];
            for (c, col) in rcols.iter().enumerate() {
                j[c] = (col[i] - r[i]) / FIT_FD_H;
            }
            for a in 0..4 {
                jtr[a] += j[a] * r[i];
                for b in 0..4 {
                    jtj[a][b] += j[a] * j[b];
                }
            }
        }
        for (a, row) in jtj.iter_mut().enumerate() {
            row[a] += lambda;
        }
        let Some(delta) = solve4(&jtj, &[-jtr[0], -jtr[1], -jtr[2], -jtr[3]]) else {
            break;
        };
        let (np0, np1, np2, np3) = project(
            *p0 + delta[0],
            *p1 + delta[1],
            *p2 + delta[2],
            *p3 + delta[3],
        );
        let nr = residuals(np0, np1, np2, np3);
        let ncost = sumsq(&nr);
        if ncost < cost {
            *p0 = np0;
            *p1 = np1;
            *p2 = np2;
            *p3 = np3;
            r = nr;
            cost = ncost;
            lambda = (lambda * 0.5).max(1e-9);
        } else {
            lambda *= 4.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
}

/// Solve a 3×3 linear system by Gaussian elimination with partial pivoting.
pub(crate) fn solve3(a: &[[f64; 3]; 3], b: &[f64; 3]) -> Option<[f64; 3]> {
    let mut m = [
        [a[0][0], a[0][1], a[0][2], b[0]],
        [a[1][0], a[1][1], a[1][2], b[1]],
        [a[2][0], a[2][1], a[2][2], b[2]],
    ];
    gaussian_eliminate::<3, 4>(&mut m)
}

/// Solve a 4×4 linear system by Gaussian elimination with partial pivoting.
pub(crate) fn solve4(a: &[[f64; 4]; 4], b: &[f64; 4]) -> Option<[f64; 4]> {
    let mut m = [[0.0_f64; 5]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        row[..4].copy_from_slice(&a[i]);
        row[4] = b[i];
    }
    gaussian_eliminate::<4, 5>(&mut m)
}

/// Gaussian elimination with partial pivoting on an augmented `N×C` matrix
/// (`C == N + 1`), returning the `N` solution components or `None` for a singular
/// system. Const-generic so the 3- and 4-parameter normal equations share one
/// numerically-careful implementation (the elimination uses a copied pivot row so
/// the row updates are disjoint borrows — no aliasing, no index-loop lint).
pub(crate) fn gaussian_eliminate<const N: usize, const C: usize>(
    m: &mut [[f64; C]; N],
) -> Option<[f64; N]> {
    for col in 0..N {
        // Partial pivot: swap in the row with the largest magnitude in this column.
        let mut piv = col;
        for row in (col + 1)..N {
            if m[row][col].abs() > m[piv][col].abs() {
                piv = row;
            }
        }
        m.swap(col, piv);
        if m[col][col].abs() < 1e-300 {
            return None;
        }
        let pivot_row = m[col];
        let denom = pivot_row[col];
        for row in m.iter_mut().skip(col + 1) {
            let f = row[col] / denom;
            for (rk, &pk) in row.iter_mut().zip(pivot_row.iter()).skip(col) {
                *rk -= f * pk;
            }
        }
    }
    // Back-substitution.
    let mut x = [0.0_f64; N];
    for col in (0..N).rev() {
        let mut s = m[col][N];
        for (k, &xk) in x.iter().enumerate().skip(col + 1) {
            s -= m[col][k] * xk;
        }
        x[col] = s / m[col][col];
    }
    Some(x)
}
