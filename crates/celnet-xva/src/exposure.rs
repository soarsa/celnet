//! Expected-exposure profile (EPE/ENE) of a netting set.
//!
//! The exposure at a future date `t_k` is the net mark of the netting set,
//! floored at zero from each party's perspective:
//!   * **positive exposure** `E⁺(t_k) = max(V(t_k), 0)` — what *we* lose if the
//!     counterparty defaults (drives CVA);
//!   * **negative exposure** `E⁻(t_k) = max(−V(t_k), 0)` — what the counterparty
//!     loses if *we* default (drives DVA).
//!
//! Taking the risk-neutral expectation over spot paths gives the **expected
//! positive/negative exposure** profiles `EPE(t_k) = E[E⁺(t_k)]`,
//! `ENE(t_k) = E[E⁻(t_k)]`.
//!
//! ## Spot model
//!
//! Spot evolves under risk-neutral FX GBM with drift `r_dom − r_for`:
//! ```text
//! S(t_{k}) = S(t_{k-1})·exp((r_d − r_f − ½σ²)·Δt + σ·√Δt·Z_k),  Z_k ~ N(0,1).
//! ```
//! The `Z_k` come from scrambled Joe-Kuo Sobol points mapped through the inverse
//! normal CDF ([`celnet_qmc`]) — a low-discrepancy sequence that gives a far
//! smoother exposure profile than plain pseudo-random MC at equal path budget.
//! One Sobol dimension is consumed per exposure step (dimension = number of grid
//! steps), so each path is an independent low-discrepancy point in `[0,1)^m`.

use crate::netting::NettingSet;
use celnet_qmc::{SobolSequence, inv_norm_cdf};

/// Configuration for the Monte-Carlo / QMC exposure simulation.
#[derive(Debug, Clone, Copy)]
pub struct ExposureConfig {
    /// Initial spot at valuation (`t = 0`).
    pub spot0: f64,
    /// Diffusion volatility for the spot evolution (the *exposure* model vol — may
    /// differ from each trade's pricing vol).
    pub sigma: f64,
    /// Number of low-discrepancy paths.
    pub paths: usize,
    /// Owen-scramble replication seed (bit-reproducible).
    pub seed: u64,
}

/// One time bucket of the simulated exposure profile (PFE, EPE, ENE bands).
#[derive(Debug, Clone, PartialEq)]
pub struct ExposureBucket {
    /// Time from the as-of date to this bucket, in years.
    pub time_years: f64,
    /// Short tenor label for the bucket (e.g. "6M", "1Y").
    pub label: String,
    /// Expected (mean) positive exposure — the EE line.
    pub ee: f64,
    /// Inner-band lower quantile (25%).
    pub q25: f64,
    /// Inner-band upper quantile (75%).
    pub q75: f64,
    /// Outer-band lower quantile (5%).
    pub pfe_lo: f64,
    /// Outer-band upper quantile (95%) — the PFE envelope.
    pub pfe: f64,
    /// Expected (mean) negative exposure (<= 0).
    pub ene: f64,
    /// Negative exposure band lower quantile (<= 0).
    pub ene_band_lo: f64,
    /// Negative exposure band upper quantile (<= 0).
    pub ene_band_hi: f64,
}

fn format_bucket_label(t: f64) -> String {
    if t < 0.001 {
        "0".to_string()
    } else if (t - 0.25).abs() < 0.05 {
        "3M".to_string()
    } else if (t - 0.5).abs() < 0.05 {
        "6M".to_string()
    } else if (t - 1.0).abs() < 0.05 {
        "1Y".to_string()
    } else if (t - 2.0).abs() < 0.05 {
        "2Y".to_string()
    } else if (t - 3.0).abs() < 0.05 {
        "3Y".to_string()
    } else if (t - 5.0).abs() < 0.05 {
        "5Y".to_string()
    } else if (t - 10.0).abs() < 0.05 {
        "10Y".to_string()
    } else if t < 1.0 {
        let months = (t * 12.0).round() as i32;
        if months > 0 {
            format!("{months}M")
        } else {
            format!("{t:.2}Y")
        }
    } else {
        format!("{t:.1}Y")
    }
}

/// A discretized exposure profile on a fixed time grid.
///
/// `grid[0] = 0` always (the valuation date); `epe`/`ene` are aligned with
/// `grid`. The discount factors `discount[k] = e^{−r_dom·t_k}` are kept alongside
/// so the CVA aggregation discounts each interval's loss to present value.
#[derive(Debug, Clone, PartialEq)]
pub struct ExposureProfile {
    grid: Vec<f64>,
    epe: Vec<f64>,
    ene: Vec<f64>,
    discount: Vec<f64>,
    buckets: Vec<ExposureBucket>,
}

impl ExposureProfile {
    /// Build an exposure profile directly from pre-computed EPE/ENE values on a
    /// grid (the **deterministic / closed-form** path). `r_dom` sets the discount
    /// factors. Used for exact-limit validation, where the exposure profile is a
    /// known deterministic function rather than an MC estimate.
    ///
    /// Panics on shape mismatch, a non-zero first grid point, non-increasing grid,
    /// or negative exposures (EPE/ENE are non-negative by definition).
    #[must_use]
    pub fn deterministic(grid: Vec<f64>, epe: Vec<f64>, ene: Vec<f64>, r_dom: f64) -> Self {
        assert!(
            grid.len() == epe.len() && grid.len() == ene.len() && !grid.is_empty(),
            "grid/epe/ene must be non-empty and equal length"
        );
        assert!(grid[0] == 0.0, "grid must start at t = 0");
        for w in grid.windows(2) {
            assert!(w[1] > w[0], "grid must be strictly increasing");
        }
        for (&p, &n) in epe.iter().zip(&ene) {
            assert!(p >= 0.0 && n >= 0.0, "EPE/ENE must be non-negative");
        }
        let discount = grid.iter().map(|&t| libm::exp(-r_dom * t)).collect();
        let buckets = grid
            .iter()
            .enumerate()
            .map(|(i, &t)| ExposureBucket {
                time_years: t,
                label: format_bucket_label(t),
                ee: epe[i],
                q25: epe[i] * 0.6,
                q75: epe[i] * 1.4,
                pfe_lo: epe[i] * 0.2,
                pfe: epe[i] * 2.0,
                ene: -ene[i],
                ene_band_lo: -ene[i] * 2.0,
                ene_band_hi: -ene[i] * 0.2,
            })
            .collect();
        Self {
            grid,
            epe,
            ene,
            discount,
            buckets,
        }
    }

    /// Simulate the exposure profile of `set` on a uniform grid of `steps`
    /// intervals out to the netting set's horizon, using scrambled-Sobol QMC.
    ///
    /// The grid is `t_k = k·Δt`, `k = 0..=steps`, `Δt = horizon/steps`. Spot is
    /// evolved with one Sobol dimension per step; the net set value is repriced at
    /// every grid date and reduced to EPE/ENE across paths.
    #[must_use]
    pub fn simulate(set: &NettingSet, cfg: &ExposureConfig, steps: usize) -> Self {
        assert!(steps >= 1, "need at least one exposure step");
        assert!(cfg.paths >= 1, "need at least one path");
        let horizon = set.horizon();
        assert!(horizon > 0.0, "netting set has zero horizon");

        let dt = horizon / steps as f64;
        let grid: Vec<f64> = (0..=steps).map(|k| k as f64 * dt).collect();
        let r_dom = set.r_dom();
        let r_for = set.r_for();
        let drift = (r_dom - r_for - 0.5 * cfg.sigma * cfg.sigma) * dt;
        let vol_step = cfg.sigma * libm::sqrt(dt);

        // Sum of E⁺ / E⁻ across paths at each grid node (index 0 = t=0 is the
        // deterministic initial mark, identical on every path).
        let mut sum_pos = vec![0.0_f64; steps + 1];
        let mut sum_neg = vec![0.0_f64; steps + 1];
        let mut vals_pos = vec![vec![0.0_f64; cfg.paths]; steps];
        let mut vals_neg = vec![vec![0.0_f64; cfg.paths]; steps];

        // t = 0 node: deterministic net value at spot0.
        let v0 = set.net_value(0.0, cfg.spot0);
        sum_pos[0] = cfg.paths as f64 * v0.max(0.0);
        sum_neg[0] = cfg.paths as f64 * (-v0).max(0.0);

        // One Sobol point per path, dimension = steps; coordinate j drives step j.
        let sobol = SobolSequence::new(steps);
        let mut point = vec![0.0_f64; steps];
        let mut stream = sobol.stream(cfg.seed);
        for path_idx in 0..cfg.paths {
            stream.next_point(&mut point);
            let mut spot = cfg.spot0;
            for (k, &u) in point.iter().enumerate() {
                let z = inv_norm_cdf(u);
                spot *= libm::exp(drift + vol_step * z);
                let t = grid[k + 1];
                let v = set.net_value(t, spot);
                let pos = v.max(0.0);
                let neg = (-v).max(0.0);
                sum_pos[k + 1] += pos;
                sum_neg[k + 1] += neg;
                vals_pos[k][path_idx] = pos;
                vals_neg[k][path_idx] = neg;
            }
        }

        let inv_n = 1.0 / cfg.paths as f64;
        let epe: Vec<f64> = sum_pos.iter().map(|s| s * inv_n).collect();
        let ene: Vec<f64> = sum_neg.iter().map(|s| s * inv_n).collect();
        let discount = grid.iter().map(|&t| libm::exp(-r_dom * t)).collect();

        let mut buckets = Vec::with_capacity(steps + 1);
        let ee0 = epe[0];
        let ene0 = -ene[0];
        buckets.push(ExposureBucket {
            time_years: 0.0,
            label: "0".to_string(),
            ee: ee0,
            q25: ee0,
            q75: ee0,
            pfe_lo: ee0,
            pfe: ee0,
            ene: ene0,
            ene_band_lo: ene0,
            ene_band_hi: ene0,
        });

        let idx_p05 = ((cfg.paths as f64 * 0.05) as usize).min(cfg.paths - 1);
        let idx_p25 = ((cfg.paths as f64 * 0.25) as usize).min(cfg.paths - 1);
        let idx_p75 = ((cfg.paths as f64 * 0.75) as usize).min(cfg.paths - 1);
        let idx_p95 = ((cfg.paths as f64 * 0.95) as usize).min(cfg.paths - 1);

        for k in 0..steps {
            vals_pos[k].sort_by(|a, b| a.total_cmp(b));
            vals_neg[k].sort_by(|a, b| a.total_cmp(b));

            let ee = epe[k + 1];
            let q25 = vals_pos[k][idx_p25];
            let q75 = vals_pos[k][idx_p75];
            let pfe_lo = vals_pos[k][idx_p05];
            let pfe = vals_pos[k][idx_p95];

            let ene_mean = -ene[k + 1];
            let ene_band_lo = -vals_neg[k][idx_p95];
            let ene_band_hi = -vals_neg[k][idx_p05];

            buckets.push(ExposureBucket {
                time_years: grid[k + 1],
                label: format_bucket_label(grid[k + 1]),
                ee,
                q25,
                q75,
                pfe_lo,
                pfe,
                ene: ene_mean,
                ene_band_lo,
                ene_band_hi,
            });
        }

        Self {
            grid,
            epe,
            ene,
            discount,
            buckets,
        }
    }

    /// Buckets of the exposure profile (quantiles and averages).
    #[must_use]
    pub fn buckets(&self) -> &[ExposureBucket] {
        &self.buckets
    }

    /// Time grid (`grid[0] = 0`).
    #[must_use]
    pub fn grid(&self) -> &[f64] {
        &self.grid
    }

    /// Expected positive exposure profile, aligned with [`Self::grid`].
    #[must_use]
    pub fn epe(&self) -> &[f64] {
        &self.epe
    }

    /// Expected negative exposure profile, aligned with [`Self::grid`].
    #[must_use]
    pub fn ene(&self) -> &[f64] {
        &self.ene
    }

    /// Discount factors `e^{−r_dom·t_k}`, aligned with [`Self::grid`].
    #[must_use]
    pub fn discount(&self) -> &[f64] {
        &self.discount
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netting::NettedTrade;
    use celnet_types::OptionType;

    #[test]
    fn deterministic_round_trips() {
        let p = ExposureProfile::deterministic(
            vec![0.0, 1.0, 2.0],
            vec![0.0, 5.0, 3.0],
            vec![0.0, 1.0, 0.5],
            0.03,
        );
        assert_eq!(p.epe(), &[0.0, 5.0, 3.0]);
        assert!((p.discount()[1] - libm::exp(-0.03)).abs() < 1e-15);
    }

    #[test]
    fn long_call_epe_positive_ene_zero() {
        // A single long call: net value is always ≥ 0 ⇒ ENE must be identically 0,
        // EPE strictly positive on the interior.
        let set = NettingSet::new(
            vec![NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.12, 1.0)],
            0.02,
            0.01,
        );
        let cfg = ExposureConfig {
            spot0: 1.10,
            sigma: 0.12,
            paths: 4096,
            seed: 0xC0FFEE,
        };
        let p = ExposureProfile::simulate(&set, &cfg, 8);
        for (k, (&epe, &ene)) in p.epe().iter().zip(p.ene()).enumerate() {
            assert!(
                ene.abs() < 1e-12,
                "ENE must vanish for a long option (k={k})"
            );
            if k > 0 && k < 8 {
                assert!(epe > 0.0, "interior EPE must be positive (k={k})");
            }
        }
    }

    #[test]
    fn simulation_is_bit_reproducible() {
        let set = NettingSet::new(
            vec![NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.12, 1.0)],
            0.02,
            0.01,
        );
        let cfg = ExposureConfig {
            spot0: 1.10,
            sigma: 0.12,
            paths: 1024,
            seed: 7,
        };
        let a = ExposureProfile::simulate(&set, &cfg, 6);
        let b = ExposureProfile::simulate(&set, &cfg, 6);
        for (x, y) in a.epe().iter().zip(b.epe()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }
}
