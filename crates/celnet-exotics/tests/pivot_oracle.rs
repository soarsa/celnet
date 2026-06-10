//! Code-disjoint Monte-Carlo oracle + gate tests for the pivot Target-Redemption
//! Accumulator ([`celnet_exotics::pivot`]).
//!
//! This file is compiled as a **separate crate** against the public API, so the
//! oracle cannot share private helpers with `pivot.rs`. Every independence axis
//! differs from the production path:
//!
//! | Axis            | production (`pivot.rs`)      | oracle (here)                       |
//! |-----------------|------------------------------|-------------------------------------|
//! | RNG             | `CounterRng` (Philox 4×32)   | inline SplitMix64 (no crate RNG)    |
//! | Normal          | `inverse_cdf` (Acklam)       | inline Box-Muller `√(−2ln u)·cos`   |
//! | Path build      | running `ln_s`, antithetic   | full `Vec<f64>` of `S_k`, plain MC  |
//! | Payoff/redeem   | branch-on-`d_pivot`          | §3.2 indicator closed form          |
//! | Estimator       | Welford + control variate    | naive sum/sumsq mean + √(var/N)     |
//!
//! The oracle is genuinely free to disagree; the gates assert that it does not,
//! within Monte-Carlo confidence bands.

use std::f64::consts::PI;

use celnet_exotics::pivot::{PivotTra, PivotTraMcConfig, pivot_tra_price, pivot_tra_price_cv};
use celnet_exotics::tarf::{RedemptionStyle, Tarf, TarfMcConfig, tarf_price};
use celnet_types::{OptionType, VanillaInputs};

// ---------------------------------------------------------------------------
// Inline, self-contained RNG + normal transform (touch nothing in the crate).
// ---------------------------------------------------------------------------

/// SplitMix64 — a tiny, well-distributed 64-bit generator (Steele, Lea & Flood,
/// 2014). Independent of the production Philox `CounterRng`.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Open-interval (0,1) uniform — the 53 high bits mapped to [2^-53, 1).
    fn next_u01(&mut self) -> f64 {
        let bits = self.next_u64() >> 11; // 53 bits
        (bits as f64 + 0.5) * (1.0 / 9_007_199_254_740_992.0) // /2^53
    }

    /// One standard-normal draw via Box-Muller (cos branch). Distinct transform
    /// from the production Acklam quantile.
    fn next_normal(&mut self) -> f64 {
        let u1 = self.next_u01();
        let u2 = self.next_u01();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

// ---------------------------------------------------------------------------
// The oracle: plain (non-antithetic) MC, full S_k vector, indicator-form payoff.
// ---------------------------------------------------------------------------

/// Returns `(bank PV, std_err, E[redeem idx], E[overshoot])`.
fn oracle_price(
    i: &VanillaInputs,
    spec: PivotTra,
    n_paths: usize,
    seed: u64,
) -> (f64, f64, f64, f64) {
    let n = spec.fixings;
    let dt = i.t / n as f64;
    let drift = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt;
    let diff = i.vol * dt.sqrt();
    let g = match spec.favourable_side {
        OptionType::Call => 1.0,
        OptionType::Put => -1.0,
    };

    // Per-fixing discounts, computed independently.
    let dfs: Vec<f64> = (0..n)
        .map(|k| (-i.r_dom * ((k + 1) as f64) * dt).exp())
        .collect();

    let mut rng = SplitMix64::new(seed);
    let mut sum_pv = 0.0f64;
    let mut sum_pv2 = 0.0f64;
    let mut sum_idx = 0.0f64;
    let mut sum_over = 0.0f64;

    for _ in 0..n_paths {
        // Build the FULL spot path forward.
        let mut spots = Vec::with_capacity(n);
        let mut ln_s = i.spot.ln();
        for _ in 0..n {
            ln_s += drift + diff * rng.next_normal();
            spots.push(ln_s.exp());
        }

        // Separate accrual loop, payoff from the §3.2 indicator closed form.
        let mut accrued = 0.0f64;
        let mut bank_pv = 0.0f64;
        let mut redeem_idx = n as f64;
        let mut overshoot = 0.0f64;

        for (k, &s) in spots.iter().enumerate() {
            // Two-level piecewise-linear per-fixing cash flow to the client:
            //   favourable side of the pivot  (g(S−P) ≥ 0): c = g·(S−K)      (slope g)
            //   adverse    side of the pivot  (g(S−P) < 0):  c = L·g·(S−K)    (slope L·g)
            // i.e. the geared leg carries the SAME sign as the intrinsic g·(S−K):
            // a geared *favourable* cash flow in the dead band (P>K, K≤S<P), and a
            // geared *adverse* loss below the strike. This pins the TARF identity at
            // P=K: there `intrinsic < 0` on the adverse side, so the bank receives
            // `−c = −L·intrinsic = L·|intrinsic|`, matching `tarf.rs` exactly. (The
            // §3.2 indicator literal `−L·g(S−K)` has a sign typo; the gated `tarf.rs`
            // limit is the authority — independently re-derived here.)
            let fav_side = g * (s - spec.pivot) >= 0.0;
            let intrinsic = g * (s - spec.strike);
            let c = if fav_side {
                intrinsic
            } else {
                spec.leverage * intrinsic
            };

            if c > 0.0 {
                let remaining = spec.target - accrued;
                if c >= remaining {
                    let settled = match spec.redemption {
                        RedemptionStyle::FullGain => c,
                        RedemptionStyle::CappedGain => remaining,
                    };
                    bank_pv -= settled * spec.notional * dfs[k];
                    overshoot = match spec.redemption {
                        RedemptionStyle::FullGain => (c - remaining).max(0.0),
                        RedemptionStyle::CappedGain => 0.0,
                    };
                    redeem_idx = (k + 1) as f64;
                    break;
                }
                bank_pv -= c * spec.notional * dfs[k];
                accrued += c;
            } else if c < 0.0 {
                bank_pv += (-c) * spec.notional * dfs[k];
            }
        }

        sum_pv += bank_pv;
        sum_pv2 += bank_pv * bank_pv;
        sum_idx += redeem_idx;
        sum_over += overshoot;
    }

    let n_f = n_paths as f64;
    let mean = sum_pv / n_f;
    let var = (sum_pv2 / n_f - mean * mean).max(0.0);
    let std_err = (var / n_f).sqrt();
    (mean, std_err, sum_idx / n_f, sum_over / n_f)
}

// ---------------------------------------------------------------------------
// §5.1 GATE — degenerate→TARF limit, exact (to_bits).
// ---------------------------------------------------------------------------

#[test]
fn pivot_collapses_to_tarf_when_pivot_equals_strike() {
    let i = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01);
    let cfg_t = TarfMcConfig {
        pairs: 200_000,
        seed: 0x7A4F,
    };
    let cfg_p = PivotTraMcConfig {
        pairs: 200_000,
        seed: 0x7A4F, // SAME seed
    };
    for style in [RedemptionStyle::FullGain, RedemptionStyle::CappedGain] {
        let tarf = Tarf {
            strike: 1.32,
            fixings: 12,
            target: 0.06,
            leverage: 2.0,
            favourable_side: OptionType::Put,
            notional: 1.0,
            redemption: style,
        };
        let piv = PivotTra::as_tarf_slice(1.32, 12, 0.06, 2.0, OptionType::Put, 1.0, style); // pivot == strike == 1.32
        let t = tarf_price(&(&i).into(), tarf, cfg_t);
        let p = pivot_tra_price(&i, piv, cfg_p);
        assert_eq!(
            p.price.to_bits(),
            t.price.to_bits(),
            "pivot(P=K) must equal TARF bit-for-bit: {} vs {}",
            p.price,
            t.price
        );
        assert_eq!(p.std_error.to_bits(), t.std_error.to_bits());
        assert_eq!(
            p.expected_redemption_fixing.to_bits(),
            t.expected_redemption_fixing.to_bits()
        );
        assert_eq!(
            p.expected_overshoot.to_bits(),
            t.expected_overshoot.to_bits()
        );
    }
}

// ---------------------------------------------------------------------------
// §5.2 GATE — independent oracle agreement (the oracle CAN disagree; it doesn't).
// Parametrized over both styles × both sides × {dead-band P>K, overlap P<K}.
// ---------------------------------------------------------------------------

#[test]
fn pivot_matches_independent_oracle() {
    let i = VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01);

    for side in [OptionType::Call, OptionType::Put] {
        for style in [RedemptionStyle::FullGain, RedemptionStyle::CappedGain] {
            // Dead band (P>K) and overlap (P<K) relative to strike 1.30.
            for &(strike, pivot) in &[(1.28, 1.33), (1.33, 1.28)] {
                let spec = PivotTra {
                    strike,
                    pivot,
                    fixings: 12,
                    target: 0.08,
                    leverage: 2.0,
                    favourable_side: side,
                    notional: 1.0,
                    redemption: style,
                };
                let prod = pivot_tra_price_cv(
                    &i,
                    spec,
                    PivotTraMcConfig {
                        pairs: 300_000,
                        seed: 0xC0FFEE,
                    },
                );
                let (op, ose, ofix, oov) = oracle_price(&i, spec, 1_500_000, 0xA11CE);
                let tol = 4.0 * (prod.std_error + ose) + 1e-9;
                assert!(
                    (prod.price - op).abs() < tol,
                    "price disagreement side={side:?} style={style:?} K={strike} P={pivot}: \
                     prod {} vs oracle {} (tol {tol})",
                    prod.price,
                    op
                );
                assert!(
                    (prod.expected_redemption_fixing - ofix).abs() < 0.05,
                    "redeem-idx disagreement side={side:?} style={style:?} K={strike} P={pivot}: \
                     prod {} vs oracle {}",
                    prod.expected_redemption_fixing,
                    ofix
                );
                assert!(
                    (prod.expected_overshoot - oov).abs() < 4.0 * (prod.std_error + ose) + 1e-3,
                    "overshoot disagreement side={side:?} style={style:?} K={strike} P={pivot}: \
                     prod {} vs oracle {}",
                    prod.expected_overshoot,
                    oov
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §5.3 GATE — Greeks finite-difference, cross-checked against the oracle's OWN
// common-random-number FD (never a self-oracle).
// ---------------------------------------------------------------------------

#[test]
fn pivot_greeks_finite_difference() {
    let i = VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01);
    // Dead-band, call-favourable, leverage 2.0.
    let spec = PivotTra {
        strike: 1.28,
        pivot: 1.33,
        fixings: 12,
        target: 0.08,
        leverage: 2.0,
        favourable_side: OptionType::Call,
        notional: 1.0,
        redemption: RedemptionStyle::FullGain,
    };

    // --- Spot delta (common random numbers across the bump). ---
    let cfg = PivotTraMcConfig {
        pairs: 400_000,
        seed: 0xDDEE,
    };
    let h = 1e-3 * i.spot;
    let up = pivot_tra_price_cv(
        &VanillaInputs {
            spot: i.spot + h,
            ..i
        },
        spec,
        cfg,
    );
    let dn = pivot_tra_price_cv(
        &VanillaInputs {
            spot: i.spot - h,
            ..i
        },
        spec,
        cfg,
    );
    let delta_bank = (up.price - dn.price) / (2.0 * h);

    // Oracle's OWN CRN-FD (same seed both bumps ⇒ common random numbers).
    let (op_u, ose_u, ..) = oracle_price(
        &VanillaInputs {
            spot: i.spot + h,
            ..i
        },
        spec,
        1_000_000,
        0xBEE,
    );
    let (op_d, ose_d, ..) = oracle_price(
        &VanillaInputs {
            spot: i.spot - h,
            ..i
        },
        spec,
        1_000_000,
        0xBEE,
    );
    let delta_oracle = (op_u - op_d) / (2.0 * h);

    // Combined band: FD of two MC estimates ⇒ SE scaled by 1/(2h).
    let combined_band = 4.0 * (up.std_error + dn.std_error + ose_u + ose_d) / (2.0 * h);
    assert!(
        (delta_bank - delta_oracle).abs() < 0.05 * delta_bank.abs().max(1e-3) + combined_band,
        "bank delta {delta_bank} vs oracle {delta_oracle} (band {combined_band})"
    );
    // Bank is SHORT the favourable (long) leg ⇒ negative spot delta.
    assert!(
        delta_bank < 0.0,
        "bank short the favourable leg ⇒ negative spot delta, got {delta_bank}"
    );

    // --- Vega (bump vol by 1e-3); sign asserted against the oracle. ---
    let hv = 1e-3;
    let cfgv = PivotTraMcConfig {
        pairs: 400_000,
        seed: 0xDDEE,
    };
    let vup = pivot_tra_price_cv(
        &VanillaInputs {
            vol: i.vol + hv,
            ..i
        },
        spec,
        cfgv,
    );
    let vdn = pivot_tra_price_cv(
        &VanillaInputs {
            vol: i.vol - hv,
            ..i
        },
        spec,
        cfgv,
    );
    let vega_bank = (vup.price - vdn.price) / (2.0 * hv);

    let (ovu, oseu, ..) = oracle_price(
        &VanillaInputs {
            vol: i.vol + hv,
            ..i
        },
        spec,
        1_000_000,
        0xBEE,
    );
    let (ovd, osed, ..) = oracle_price(
        &VanillaInputs {
            vol: i.vol - hv,
            ..i
        },
        spec,
        1_000_000,
        0xBEE,
    );
    let vega_oracle = (ovu - ovd) / (2.0 * hv);
    let vband = 4.0 * (vup.std_error + vdn.std_error + oseu + osed) / (2.0 * hv);
    assert!(
        (vega_bank - vega_oracle).abs() < 0.10 * vega_bank.abs().max(1e-3) + vband,
        "bank vega {vega_bank} vs oracle {vega_oracle} (band {vband})"
    );
    // Sign agrees with the oracle (not asserted in isolation — avoids self-oracle).
    assert_eq!(
        vega_bank.signum(),
        vega_oracle.signum(),
        "bank vega sign {vega_bank} should match oracle {vega_oracle}"
    );
}

// ---------------------------------------------------------------------------
// §5.4 GATE — the pivot is a real lever (moving it off the strike moves price).
// ---------------------------------------------------------------------------

#[test]
fn pivot_away_from_strike_changes_price() {
    let i = VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01);
    let base_spec = PivotTra {
        strike: 1.28,
        pivot: 1.28,
        fixings: 12,
        target: 0.08,
        leverage: 2.0,
        favourable_side: OptionType::Call,
        notional: 1.0,
        redemption: RedemptionStyle::FullGain,
    };
    let cfg = PivotTraMcConfig {
        pairs: 400_000,
        seed: 0x1E5E2,
    };
    let p_eq = pivot_tra_price_cv(
        &i,
        PivotTra {
            pivot: 1.28,
            strike: 1.28,
            ..base_spec
        },
        cfg,
    );
    let p_db = pivot_tra_price_cv(
        &i,
        PivotTra {
            pivot: 1.33,
            strike: 1.28,
            ..base_spec
        },
        cfg,
    );
    assert!(
        (p_eq.price - p_db.price).abs() > 4.0 * (p_eq.std_error + p_db.std_error),
        "moving the pivot off the strike must move the price beyond MC noise: \
         {} vs {} (SE {} / {})",
        p_eq.price,
        p_db.price,
        p_eq.std_error,
        p_db.std_error
    );
}
