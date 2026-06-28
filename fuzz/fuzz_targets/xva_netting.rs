//! Fuzz target: netting / exposure / XVA adjustment contracts on arbitrary-but-finite
//! inputs, driving the complete `celnet_xva` pipeline.
//!
//! Drives `NettingSet::new` → `ExposureProfile::simulate` → `compute_xva` and asserts
//! the algebraic + monotonicity contracts documented in the `celnet_xva` crate, on any
//! in-domain draw:
//!
//!   1. No panic on valid LGD (clamped into `[0, 1]`) and positive hazards;
//!   2. Profile invariants: `grid[0] == 0.0`; `epe[k] >= 0`, `ene[k] <= 0` (ENE
//!      sign convention: stored as ≥ 0, representing negative exposure magnitude);
//!      `discount[k] == exp(-r_dom * t_k)` to 1e-12 tolerance; all finite;
//!   3. Determinism: same draw ⇒ `simulate` bit-identical on two calls;
//!   4. `XvaResult`: `cva >= 0`, `dva >= 0`, all three fields finite;
//!      `total_adjustment() == cva − dva + fva` exactly (to_bits match);
//!   5. Comparative monotonicity: doubling `lambda_cpty` does not decrease the
//!      counterparty's total default probability (CVA itself is NOT monotonic in
//!      hazard for a general exposure profile — see Contract 5 below)
//!      (survival mass shifts earlier; the EPE-weighted integral grows or stays).
//!
//! The panic contract on out-of-range LGD is unit-tested in the crate, not here:
//! this target clamps LGD into [0, 1] so the panic branch is never entered.
//!
//! A stable proptest mirror is in
//!   `crates/celnet-xva/tests/netting_fuzz.rs`
//! so this property gates the merge on the stable toolchain too.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run xva_netting -- -max_total_time=120

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

use celnet_types::OptionType;
use celnet_xva::{ExposureConfig, ExposureProfile, NettedTrade, NettingSet, SurvivalCurve, XvaInputs, compute_xva};

/// Map an arbitrary finite-or-not `f64` into `[lo, hi]`, treating NaN/inf as the
/// midpoint.
fn clamp_into(raw: f64, lo: f64, hi: f64) -> f64 {
    let mid = 0.5 * (lo + hi);
    if !raw.is_finite() {
        return mid;
    }
    let t = 0.5 * (libm::tanh(raw) + 1.0);
    lo + t * (hi - lo)
}

/// One drawn vanilla trade in the netting set.
#[derive(Debug)]
struct TradeDraw {
    is_call: bool,
    strike: f64,
    expiry: f64,
    vol: f64,
    notional: f64, // signed
}

impl<'a> Arbitrary<'a> for TradeDraw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(TradeDraw {
            is_call: bool::arbitrary(u)?,
            strike: clamp_into(f64::arbitrary(u)?, 1e-3, 1e4),
            expiry: clamp_into(f64::arbitrary(u)?, 1.0 / 365.0, 10.0),
            vol: clamp_into(f64::arbitrary(u)?, 1e-4, 3.0),
            notional: clamp_into(f64::arbitrary(u)?, -1e7, 1e7),
        })
    }
}

/// Top-level draw.
#[derive(Debug)]
struct Draw {
    trades: Vec<TradeDraw>,
    r_dom: f64,
    r_for: f64,
    spot0: f64,
    sigma: f64,
    paths_pow: u8,      // paths = 64 << (paths_pow % 3)  — bounded work: 64/128/256
    seed: u64,
    lambda_cpty: f64,   // counterparty flat hazard, clamped ≥ 0
    lambda_own: f64,
    lgd_c: f64,         // clamped into [0, 1]
    lgd_o: f64,
    funding_spread: f64,
    steps: u8,          // 4..=32 grid points
}

impl<'a> Arbitrary<'a> for Draw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let n_trades = 1 + (u8::arbitrary(u)? as usize % 16);
        let mut trades = Vec::with_capacity(n_trades);
        for _ in 0..n_trades {
            trades.push(TradeDraw::arbitrary(u)?);
        }
        Ok(Draw {
            trades,
            r_dom: clamp_into(f64::arbitrary(u)?, -0.25, 0.25),
            r_for: clamp_into(f64::arbitrary(u)?, -0.25, 0.25),
            spot0: clamp_into(f64::arbitrary(u)?, 1e-3, 1e4),
            sigma: clamp_into(f64::arbitrary(u)?, 1e-4, 3.0),
            paths_pow: u8::arbitrary(u)?,
            seed: u64::arbitrary(u)?,
            // Hazard rates: clamp to [1e-6, 2.0] — strictly positive (negative hazard panics by contract).
            lambda_cpty: clamp_into(f64::arbitrary(u)?, 1e-6, 2.0),
            lambda_own: clamp_into(f64::arbitrary(u)?, 1e-6, 2.0),
            lgd_c: clamp_into(f64::arbitrary(u)?, 0.0, 1.0),
            lgd_o: clamp_into(f64::arbitrary(u)?, 0.0, 1.0),
            funding_spread: clamp_into(f64::arbitrary(u)?, -0.05, 0.05),
            // 4..=32 grid points (avoid the 0 that would panic).
            steps: 4 + (u8::arbitrary(u)? % 29),
        })
    }
}

fuzz_target!(|draw: Draw| {
    // Build trades — all must have positive expiry (clamped above).
    let trades: Vec<NettedTrade> = draw
        .trades
        .iter()
        .map(|t| {
            NettedTrade::new(
                if t.is_call {
                    OptionType::Call
                } else {
                    OptionType::Put
                },
                t.strike,
                t.expiry,
                t.vol,
                t.notional,
            )
        })
        .collect();

    let set = NettingSet::new(trades, draw.r_dom, draw.r_for);

    // Horizon must be positive (at least one trade with positive expiry — guaranteed
    // by clamping above).
    if set.horizon() <= 0.0 {
        return;
    }

    let cfg = ExposureConfig {
        spot0: draw.spot0,
        sigma: draw.sigma,
        paths: 64_usize << (draw.paths_pow % 3), // 64, 128, or 256
        seed: draw.seed,
    };
    let steps = draw.steps as usize;

    // Contract 1: no panic on valid inputs.
    let profile = ExposureProfile::simulate(&set, &cfg, steps);

    // Contract 2: profile invariants.
    let grid = profile.grid();
    assert_eq!(grid[0], 0.0, "grid must start at t=0");
    for &epe in profile.epe() {
        assert!(epe >= 0.0 && epe.is_finite(), "EPE must be finite and ≥ 0");
    }
    for &ene in profile.ene() {
        assert!(ene >= 0.0 && ene.is_finite(), "ENE magnitude must be finite and ≥ 0");
    }
    for (k, (&t_k, &d_k)) in grid.iter().zip(profile.discount()).enumerate() {
        let expected = libm::exp(-draw.r_dom * t_k);
        assert!(
            (d_k - expected).abs() <= 1e-12 * (1.0 + expected),
            "discount[{k}] = {d_k} must equal exp(-r_dom*t_k) = {expected}"
        );
    }

    // Contract 3: determinism — identical re-run produces bit-identical profiles.
    let profile2 = ExposureProfile::simulate(&set, &cfg, steps);
    for ((&a, &b), (&c, &d)) in profile
        .epe()
        .iter()
        .zip(profile.ene())
        .zip(profile2.epe().iter().zip(profile2.ene()))
    {
        assert_eq!(a.to_bits(), b.to_bits(), "EPE must be bit-reproducible");
        assert_eq!(c.to_bits(), d.to_bits(), "ENE must be bit-reproducible");
    }

    // Contract 4 & 5: XVA adjustments.
    let cpty = SurvivalCurve::flat(draw.lambda_cpty);
    let own = SurvivalCurve::flat(draw.lambda_own);

    let xva = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &cpty,
        own: &own,
        lgd_counterparty: draw.lgd_c,
        lgd_own: draw.lgd_o,
        funding_spread: draw.funding_spread,
    });

    assert!(xva.cva >= 0.0 && xva.cva.is_finite(), "CVA must be finite and ≥ 0");
    assert!(xva.dva >= 0.0 && xva.dva.is_finite(), "DVA must be finite and ≥ 0");
    assert!(xva.fva.is_finite(), "FVA must be finite");

    // total_adjustment() == cva − dva + fva exactly.
    let expected_total = xva.cva - xva.dva + xva.fva;
    assert_eq!(
        xva.total_adjustment().to_bits(),
        expected_total.to_bits(),
        "total_adjustment must be cva - dva + fva exactly"
    );

    // Contract 5: doubling lambda_cpty must not decrease the counterparty's TOTAL
    // default probability over the horizon — the mathematically-exact hazard
    // monotonicity (S(t)=exp(-λt) ⇒ doubling λ never raises survival). NOTE: CVA
    // itself is NOT monotonic in the hazard rate for a general exposure profile —
    // CVA = Σ DF·EPE·ΔPD reweights toward the EARLIER intervals as λ rises, so a
    // rising / hump-shaped discounted-EPE leg can legitimately LOWER CVA when λ
    // doubles. Asserting CVA-monotonicity was a wrong invariant — the `compute_xva`
    // CVA formula is the standard discrete one and is correct; we assert the true
    // default-probability monotonicity and that the doubled-hazard XVA still
    // computes cleanly.
    let cpty2 = SurvivalCurve::flat(draw.lambda_cpty * 2.0);
    let horizon = set.horizon();
    let pd_before = 1.0 - cpty.survival(horizon);
    let pd_after = 1.0 - cpty2.survival(horizon);
    assert!(
        pd_after >= pd_before - 1e-12,
        "doubling hazard must not decrease total default probability: before={}, after={}",
        pd_before,
        pd_after
    );
    let xva2 = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &cpty2,
        own: &own,
        lgd_counterparty: draw.lgd_c,
        lgd_own: draw.lgd_o,
        funding_spread: draw.funding_spread,
    });
    assert!(
        xva2.cva >= 0.0 && xva2.cva.is_finite(),
        "doubled-hazard CVA must be ≥ 0 and finite: {}",
        xva2.cva
    );
});
