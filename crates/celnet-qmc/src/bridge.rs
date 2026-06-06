//! Brownian-bridge path construction for quasi-Monte-Carlo.
//!
//! A naive sequential ("incremental") construction maps Sobol dimension `k` to
//! the `k`-th time-step increment, spreading the path's variance roughly evenly
//! across all dimensions. Sobol points are most uniform in their **first**
//! dimensions, so we instead use the **Brownian bridge**: dimension 0 sets the
//! terminal value `W(T)`, dimension 1 the midpoint conditioned on the endpoints,
//! and so on by recursive bisection. This loads the dominant variance onto the
//! low (best-distributed) Sobol dimensions — the entire point of pairing the
//! bridge with QMC — and is what produces the super-`O(n^{-1/2})` convergence.
//!
//! For an `m`-step path on a uniform grid `t_i = i·T/m`, `i = 1..=m`, the
//! construction consumes exactly `m` standard normals `z_0..z_{m-1}` (mapped from
//! the first `m` Sobol coordinates via the inverse-normal CDF) and produces the
//! Brownian path `W(t_1)..W(t_m)` with the **exact** discrete covariance
//! `Cov(W(t_i), W(t_j)) = min(t_i, t_j)`.
//!
//! # Method provenance (doc comments only)
//!
//! Brownian-bridge / principal-bisection path construction for QMC: Caflisch-
//! Morokoff-Owen (1997); Glasserman, *Monte Carlo Methods in Financial
//! Engineering* (2004), §3.1. Identifiers are purpose-named.

use celnet_core::math::sqrt;

/// Precomputed Brownian-bridge schedule for a fixed number of steps `m`.
///
/// Stores, for each construction step, the left/right anchor indices and the
/// interpolation/standard-deviation weights, so that turning a vector of `m`
/// standard normals into a bridged path is a fixed sequence of fused
/// multiply-adds — allocation-free on the hot path after construction.
pub struct BrownianBridge {
    m: usize,
    /// Uniform grid times `t_i = (i+1)·dt`, `i = 0..m` (so `t_{m-1} = T`).
    times: Vec<f64>,
    /// Per-step plan entry: `(out_idx, left_idx, right_idx, left_w, right_w, std)`.
    /// `left_idx == usize::MAX` denotes "anchored at 0" (the path origin `W(0)=0`).
    plan: Vec<BridgeStep>,
}

#[derive(Clone, Copy)]
struct BridgeStep {
    out: usize,
    left: usize,
    right: usize,
    left_w: f64,
    right_w: f64,
    std: f64,
}

const LEFT_ORIGIN: usize = usize::MAX;

impl BrownianBridge {
    /// Build the bridge schedule for `m` equally-spaced steps over `[0, T]`.
    ///
    /// # Panics
    ///
    /// Panics if `m == 0` or `t_total <= 0`.
    #[must_use]
    pub fn new(m: usize, t_total: f64) -> Self {
        assert!(m >= 1, "Brownian bridge needs at least one step");
        assert!(t_total > 0.0, "total time must be positive");
        let dt = t_total / m as f64;
        let times: Vec<f64> = (0..m).map(|i| (i + 1) as f64 * dt).collect();

        // Step 0: terminal point W(T) = W(t_{m-1}), conditioned on the origin
        // W(0) = 0 (variance t_{m-1}). Then recursively fill every interior index
        // by bisection: the midpoint of an interval whose two ends are already
        // determined is a Brownian bridge between them.
        let mut plan = Vec::with_capacity(m);
        plan.push(BridgeStep {
            out: m - 1,
            left: LEFT_ORIGIN,
            right: m - 1,
            left_w: 0.0,
            right_w: 0.0,
            std: sqrt(times[m - 1]),
        });
        // Bisect the open index interval between the origin (sentinel `left`) and
        // the terminal index `m-1`, both now determined.
        bisect(LEFT_ORIGIN, m - 1, &times, &mut plan);

        debug_assert_eq!(plan.len(), m);
        let _ = dt;
        Self { m, times, plan }
    }

    /// Number of steps (and of standard normals consumed).
    #[must_use]
    pub fn steps(&self) -> usize {
        self.m
    }

    /// Grid time `t_i` for step index `i` (`i < m`); `t_{m-1} == T`.
    #[must_use]
    pub fn time(&self, i: usize) -> f64 {
        self.times[i]
    }

    /// Build the Brownian path `W(t_1)..W(t_m)` from `m` standard normals.
    ///
    /// `z.len()` must equal [`Self::steps`]; writes the path into `path` (also
    /// length `m`). Allocation-free.
    ///
    /// # Panics
    ///
    /// Panics if `z.len() != m` or `path.len() != m`.
    pub fn build(&self, z: &[f64], path: &mut [f64]) {
        assert_eq!(z.len(), self.m, "need exactly m normals");
        assert_eq!(path.len(), self.m, "path buffer must be length m");
        for (k, step) in self.plan.iter().enumerate() {
            let left_val = if step.left == LEFT_ORIGIN {
                0.0
            } else {
                path[step.left]
            };
            let mean = step.left_w * left_val + step.right_w * path[step.right];
            path[step.out] = mean + step.std * z[k];
        }
    }

    /// The bridge **weight matrix** `A` such that `W = A · z` (path = `A` times
    /// the standard-normal vector). Row `i` (length `m`) gives the linear
    /// combination producing `W(t_i)`. Used to verify the covariance identity
    /// `A·Aᵀ = Σ` with `Σ_{ij} = min(t_i, t_j)`.
    #[must_use]
    pub fn weight_matrix(&self) -> Vec<Vec<f64>> {
        // Compute A column-by-column: feed the k-th unit normal vector e_k
        // through the (linear, mean-only-from-prior) recurrence, then transpose
        // the resulting columns into the row-major weight matrix A.
        let cols: Vec<Vec<f64>> = (0..self.m)
            .map(|k| {
                let mut col = vec![0.0f64; self.m];
                // Replay the plan with z = e_k (only step k contributes its std
                // term, but earlier-filled points propagate through the means).
                for (kk, step) in self.plan.iter().enumerate() {
                    let left_val = if step.left == LEFT_ORIGIN {
                        0.0
                    } else {
                        col[step.left]
                    };
                    let mean = step.left_w * left_val + step.right_w * col[step.right];
                    col[step.out] = mean + if kk == k { step.std } else { 0.0 };
                }
                col
            })
            .collect();
        // Transpose: A[i][k] = cols[k][i].
        let mut a = vec![vec![0.0f64; self.m]; self.m];
        for (k, col) in cols.iter().enumerate() {
            for (i, &c) in col.iter().enumerate() {
                a[i][k] = c;
            }
        }
        a
    }
}

/// Schedule the indices strictly between anchors `left` and `right` by recursive
/// bisection. `left` is either [`LEFT_ORIGIN`] (the path origin, time 0) or an
/// already-determined index; `right` is an already-determined index. The
/// open index span is `(left_index, right)` where `left_index = 0` for the
/// origin and `left + 1` otherwise; if that span is empty, there is nothing to do.
fn bisect(left: usize, right: usize, times: &[f64], plan: &mut Vec<BridgeStep>) {
    let lo = if left == LEFT_ORIGIN { 0 } else { left + 1 };
    if lo >= right {
        return; // no interior index between the anchors
    }
    // Bisect the interior [lo, right-1]; pick its midpoint.
    let mid = lo + (right - lo - 1) / 2;

    let t_left = if left == LEFT_ORIGIN {
        0.0
    } else {
        times[left]
    };
    let t_mid = times[mid];
    let t_right = times[right];

    // Brownian-bridge interpolation of W(t_mid) given W(t_left), W(t_right):
    //   mean = W_left + (t_mid - t_left)/(t_right - t_left) · (W_right - W_left)
    //        = (1 - frac)·W_left + frac·W_right
    //   var  = (t_mid - t_left)(t_right - t_mid)/(t_right - t_left)
    let span = t_right - t_left;
    let frac = (t_mid - t_left) / span;
    let var = (t_mid - t_left) * (t_right - t_mid) / span;
    plan.push(BridgeStep {
        out: mid,
        left,
        right,
        left_w: 1.0 - frac,
        right_w: frac,
        std: sqrt(var),
    });

    // Recurse into the two sub-intervals now bounded by `mid`.
    bisect(left, mid, times, plan);
    bisect(mid, right, times, plan);
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    /// The bridge must fill every index exactly once (a valid permutation/plan).
    #[test]
    fn plan_is_complete_permutation() {
        for m in [1usize, 2, 3, 4, 5, 8, 13, 16, 32] {
            let bb = BrownianBridge::new(m, 1.5);
            assert_eq!(bb.plan.len(), m);
            let mut seen = vec![false; m];
            for s in &bb.plan {
                assert!(!seen[s.out], "index {} filled twice (m={m})", s.out);
                seen[s.out] = true;
            }
            assert!(seen.iter().all(|&b| b), "not all indices filled (m={m})");
        }
    }

    /// Exact discrete Brownian covariance: A·Aᵀ = Σ, Σ_{ij}=min(t_i,t_j).
    #[test]
    fn covariance_identity() {
        for m in [1usize, 2, 4, 7, 16] {
            let t_total = 2.0;
            let bb = BrownianBridge::new(m, t_total);
            let a = bb.weight_matrix();
            for i in 0..m {
                for j in 0..m {
                    let c: f64 = a[i].iter().zip(a[j].iter()).map(|(x, y)| x * y).sum();
                    let expected = bb.time(i).min(bb.time(j));
                    assert_close!(c, expected, 1e-12, 1e-12);
                }
            }
        }
    }
}
