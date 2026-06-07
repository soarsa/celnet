//! CVA / DVA / FVA aggregation over an exposure profile and survival curves.
//!
//! ## Unilateral CVA
//!
//! The discrete standard form (Basel III / ISDA SA-CVA discretization; Gregory,
//! *The xVA Challenge* 2015, eq. 12.x; Brigo-Morini-Pallavicini 2013):
//!
//! ```text
//! CVA = LGD_cpty · Σ_{k=1..n} D(t_k) · EPE(t_k) · [S_c(t_{k-1}) − S_c(t_k)]
//! ```
//!
//! where `LGD = 1 − R` is the counterparty loss-given-default, `D(t_k)` the
//! risk-free discount factor, `EPE(t_k)` the expected positive exposure, and
//! `S_c(t_{k-1}) − S_c(t_k)` the counterparty marginal default probability over
//! `(t_{k-1}, t_k]`. The exposure at `t_k` is the loss conditional on default in
//! that interval; pairing it with the interval default probability and summing is
//! the rectangle discretization of `∫ LGD·D(t)·EPE(t)·(−dS_c(t))`.
//!
//! ## Bilateral DVA
//!
//! Symmetric on our own credit and the **negative** exposure (a benefit — our own
//! default extinguishes a liability):
//!
//! ```text
//! DVA = LGD_own · Σ_k D(t_k) · ENE(t_k) · [S_o(t_{k-1}) − S_o(t_k)]
//! ```
//!
//! ## FVA
//!
//! The funding cost/benefit of the uncollateralized expected exposure, charged at
//! the funding spread `s_f` over each interval on the joint-survival measure
//! (only-alive cashflows are funded):
//!
//! ```text
//! FVA = s_f · Σ_k D(t_k) · (EPE(t_k) − ENE(t_k)) · Δt_k · S_c(t_k)·S_o(t_k)
//! ```
//!
//! Positive net exposure (`EPE > ENE`) is a funding **cost** (FVA > 0); a net
//! liability is a funding **benefit** (FVA < 0). This is the symmetric
//! FCA−FBA form (Burgard-Kjaer 2011; Gregory 2015, ch. 14), reduced to a single
//! funding spread on the net expected exposure.
//!
//! All three are computed from one [`crate::ExposureProfile`] and the two
//! [`crate::SurvivalCurve`]s in a single pass.

use crate::exposure::ExposureProfile;
use crate::survival::SurvivalCurve;

/// The XVA result: the three adjustments and the all-in counterparty-risk-adjusted
/// correction `CVA − DVA + FVA` that would be subtracted from the risk-free value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XvaResult {
    /// Credit valuation adjustment (≥ 0): expected loss from counterparty default.
    pub cva: f64,
    /// Debit valuation adjustment (≥ 0): expected benefit from own default.
    pub dva: f64,
    /// Funding valuation adjustment (signed): cost (+) or benefit (−) of funding
    /// the uncollateralized net expected exposure.
    pub fva: f64,
}

impl XvaResult {
    /// The total adjustment to the risk-free price: `CVA − DVA + FVA`. A dealer
    /// charges this on top of the risk-free value.
    #[must_use]
    pub fn total_adjustment(&self) -> f64 {
        self.cva - self.dva + self.fva
    }
}

/// Inputs to the XVA aggregation.
#[derive(Debug, Clone)]
pub struct XvaInputs<'a> {
    /// The exposure profile (EPE/ENE on a grid with discount factors).
    pub profile: &'a ExposureProfile,
    /// Counterparty survival curve (drives CVA and the joint-survival funding
    /// weight).
    pub counterparty: &'a SurvivalCurve,
    /// Own survival curve (drives DVA and the joint-survival funding weight).
    pub own: &'a SurvivalCurve,
    /// Counterparty loss-given-default `LGD = 1 − R_cpty`, in `[0, 1]`.
    pub lgd_counterparty: f64,
    /// Own loss-given-default `LGD = 1 − R_own`, in `[0, 1]`.
    pub lgd_own: f64,
    /// Funding spread (annualized) charged on the net expected exposure.
    pub funding_spread: f64,
}

/// Compute CVA, DVA and FVA from an exposure profile and the two survival curves.
///
/// Panics on out-of-range LGDs (must be in `[0, 1]`) — a recovery rate outside
/// `[0, 1]` is a modelling error.
#[must_use]
pub fn compute_xva(inp: &XvaInputs<'_>) -> XvaResult {
    assert!(
        (0.0..=1.0).contains(&inp.lgd_counterparty) && (0.0..=1.0).contains(&inp.lgd_own),
        "LGD must be in [0, 1]"
    );
    let grid = inp.profile.grid();
    let epe = inp.profile.epe();
    let ene = inp.profile.ene();
    let disc = inp.profile.discount();
    let n = grid.len();

    let mut cva = 0.0;
    let mut dva = 0.0;
    let mut fva = 0.0;

    // Survival at each grid node, computed once.
    // Interval k spans (t_{k-1}, t_k], k = 1..n.
    let mut s_c_prev = inp.counterparty.survival(grid[0]); // = 1 at t=0
    let mut s_o_prev = inp.own.survival(grid[0]);
    for k in 1..n {
        let s_c = inp.counterparty.survival(grid[k]);
        let s_o = inp.own.survival(grid[k]);

        let dp_c = s_c_prev - s_c; // counterparty marginal default prob on the interval
        let dp_o = s_o_prev - s_o; // own marginal default prob

        cva += inp.lgd_counterparty * disc[k] * epe[k] * dp_c;
        dva += inp.lgd_own * disc[k] * ene[k] * dp_o;

        // FVA: funding spread on the net expected exposure, time-weighted over the
        // interval, on the joint-survival measure (both parties alive at t_k).
        let dt = grid[k] - grid[k - 1];
        let net_ee = epe[k] - ene[k];
        fva += inp.funding_spread * disc[k] * net_ee * dt * s_c * s_o;

        s_c_prev = s_c;
        s_o_prev = s_o;
    }

    XvaResult { cva, dva, fva }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_default_prob_gives_zero_cva_dva() {
        let profile = ExposureProfile::deterministic(
            vec![0.0, 1.0, 2.0],
            vec![0.0, 5.0, 4.0],
            vec![0.0, 2.0, 1.0],
            0.02,
        );
        let alive = SurvivalCurve::flat(0.0);
        let r = compute_xva(&XvaInputs {
            profile: &profile,
            counterparty: &alive,
            own: &alive,
            lgd_counterparty: 0.6,
            lgd_own: 0.6,
            funding_spread: 0.01,
        });
        assert!(r.cva.abs() < 1e-15);
        assert!(r.dva.abs() < 1e-15);
        // FVA survives (it does not depend on default events, only on funding).
        assert!(r.fva > 0.0);
    }

    #[test]
    fn cva_monotone_increasing_in_lgd_and_hazard() {
        let profile = ExposureProfile::deterministic(
            vec![0.0, 1.0, 2.0, 3.0],
            vec![0.0, 6.0, 5.0, 3.0],
            vec![0.0, 1.0, 1.0, 0.5],
            0.02,
        );
        let own = SurvivalCurve::flat(0.0);
        let base = |lgd: f64, lam: f64| {
            compute_xva(&XvaInputs {
                profile: &profile,
                counterparty: &SurvivalCurve::flat(lam),
                own: &own,
                lgd_counterparty: lgd,
                lgd_own: 0.6,
                funding_spread: 0.0,
            })
            .cva
        };
        assert!(base(0.6, 0.03) > base(0.3, 0.03)); // increasing in LGD
        assert!(base(0.6, 0.06) > base(0.6, 0.03)); // increasing in hazard
    }
}
