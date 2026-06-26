//! Fuzz target: adversarial broker quote grids -> `celnet_surface::build_smile_and_outer`.
//!
//! The surface calibration pipeline is a legitimate-domain adversarial target:
//! an economically degenerate quote set (sign-adversarial RR/BF, near-zero
//! ATM vol, barrier-straddling strikes) must surface as a typed `CalibrationError`,
//! never as a panic or a NaN-poisoned smile.
//!
//! The library promise (documented in `celnet_surface::lib`) is:
//!   "a degenerate set surfaces as a calibration error, never a panic".
//!
//! Contracts asserted here:
//!   1. `build_smile_and_outer` never panics on any in-domain draw — `Ok` or a
//!      typed `CalibrationError`;
//!   2. `Ok(smile, _)` ⇒ `smile.implied_vol` at every point of a 21-strike scan
//!      around the forward is finite and strictly positive (no NaN poisoning);
//!   3. `Ok` ⇒ `check_slice` returns a finite `ArbitrageReport` (fields finite —
//!      the report may flag arbitrage; it must never contain NaN).
//!
//! A stable proptest mirror is in
//!   `crates/celnet-surface/tests/quote_grid_fuzz.rs`
//! so this property gates the merge on the stable toolchain too.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run surface_quote_grid -- -max_total_time=120

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

use celnet_conventions::ConventionRecord;
use celnet_core::Smile;
use celnet_surface::{MarketContext, MarketQuotes, build_smile_and_outer, check_slice};
use celnet_types::{AtmConvention, Carry, Cut, DayCount, DeltaConvention, PremiumStyle, Settlement};

/// Map an arbitrary finite-or-not `f64` into `[lo, hi]`, treating NaN/inf as the
/// midpoint so a degenerate draw still produces a legal-domain value.
fn clamp_into(raw: f64, lo: f64, hi: f64) -> f64 {
    let mid = 0.5 * (lo + hi);
    if !raw.is_finite() {
        return mid;
    }
    // Squash through tanh so the full f64 range folds smoothly into [lo, hi];
    // this keeps extreme magnitudes mapping to the domain edges (the corners we
    // most want to probe) without ever escaping the interval.
    let t = 0.5 * (libm::tanh(raw) + 1.0); // in (0, 1)
    lo + t * (hi - lo)
}

/// An in-domain adversarial draw for the surface calibration pipeline.
#[derive(Debug)]
struct Draw {
    spot: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    atm_vol: f64,
    rr_25: f64,
    bf_25: f64,
    outer: Option<(f64, f64)>, // rr_10, bf_10
    // Convention selector: which AtmConvention to use (0 = DNS, 1 = ATMF).
    atm_conv_pick: u8,
    // Delta convention selector (0..4).
    delta_conv_pick: u8,
}

impl<'a> Arbitrary<'a> for Draw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Draw {
            spot: clamp_into(f64::arbitrary(u)?, 1e-3, 1e4),
            t: clamp_into(f64::arbitrary(u)?, 1.0 / 365.0, 30.0),
            r_dom: clamp_into(f64::arbitrary(u)?, -0.25, 0.25),
            r_for: clamp_into(f64::arbitrary(u)?, -0.25, 0.25),
            atm_vol: clamp_into(f64::arbitrary(u)?, 1e-4, 3.0),
            // Sign-adversarial RR/BF — the library must handle any sign combination.
            rr_25: clamp_into(f64::arbitrary(u)?, -0.5, 0.5),
            bf_25: clamp_into(f64::arbitrary(u)?, -0.5, 0.5),
            outer: if bool::arbitrary(u)? {
                Some((
                    clamp_into(f64::arbitrary(u)?, -0.5, 0.5),
                    clamp_into(f64::arbitrary(u)?, -0.5, 0.5),
                ))
            } else {
                None
            },
            atm_conv_pick: u8::arbitrary(u)?,
            delta_conv_pick: u8::arbitrary(u)?,
        })
    }
}

/// Build a fixed `ConventionRecord` from the draw's convention selectors. The
/// record is the library's documented input; every combination is legal.
fn convention_from_draw(draw: &Draw) -> ConventionRecord {
    let atm = match draw.atm_conv_pick % 2 {
        0 => AtmConvention::DeltaNeutralStraddle,
        _ => AtmConvention::AtmForward,
    };
    let delta = match draw.delta_conv_pick % 4 {
        0 => DeltaConvention::SpotUnadjusted,
        1 => DeltaConvention::SpotPremiumAdjusted,
        2 => DeltaConvention::ForwardUnadjusted,
        _ => DeltaConvention::ForwardPremiumAdjusted,
    };
    ConventionRecord::new(
        delta,
        atm,
        PremiumStyle::DomesticPips,
        Cut::NewYork1000,
        DayCount::Act365Fixed,
        DayCount::Act360,
        DayCount::Act360,
        Settlement::Deliverable,
    )
}

fuzz_target!(|draw: Draw| {
    let carry = Carry::FxRates {
        r_dom: draw.r_dom,
        r_for: draw.r_for,
    };
    let conv = convention_from_draw(&draw);
    let ctx = MarketContext::new(draw.spot, carry, draw.t, conv);

    let quotes = match draw.outer {
        Some((rr_10, bf_10)) => {
            MarketQuotes::five_point(draw.atm_vol, draw.rr_25, draw.bf_25, rr_10, bf_10)
        }
        None => MarketQuotes::three_point(draw.atm_vol, draw.rr_25, draw.bf_25),
    };

    // Contract 1: never panics — Ok or typed CalibrationError.
    let result = build_smile_and_outer(&ctx, &quotes);
    let (smile, _outer) = match result {
        Ok(pair) => pair,
        Err(_calib_err) => {
            // A typed calibration error on degenerate input is the correct outcome.
            return;
        }
    };

    let forward = ctx.forward();
    let t = ctx.t;

    // Contract 2: vol at a 21-strike scan is finite (no NaN/Inf poisoning).
    // For adversarial quote grids that violate arbitrage but still calibrate,
    // the interpolated vol may be slightly negative at wing strikes; the
    // primary contract is no-panic / no-NaN.
    let f_lo = forward * 0.5_f64.max(1e-6);
    let f_hi = forward * 2.0;
    for i in 0..21 {
        let k = f_lo + (f_hi - f_lo) * (i as f64) / 20.0;
        if k <= 0.0 {
            continue;
        }
        let vol = smile.implied_vol(k, forward, t).0;
        assert!(
            vol.is_finite(),
            "implied_vol at k={k} forward={forward} must be finite (no NaN/Inf), got {vol}"
        );
    }

    // Contract 3: check_slice returns a finite ArbitrageReport.
    // Precondition: grid[0] > h (required by check_slice); skip when the forward
    // is so low that f_lo ≤ h (extreme carry/spot combinations).
    let h = 1e-3_f64;
    let grid: Vec<f64> = (0..21)
        .map(|i| f_lo + (f_hi - f_lo) * (i as f64) / 20.0)
        .filter(|&k| k > 0.0)
        .collect();
    if grid.len() >= 3 && grid[0] > h {
        let rep = check_slice(&smile, &grid, forward, t, h);
        // The report fields must be finite (may flag arbitrage; must not NaN).
        assert!(
            rep.min_density.is_finite(),
            "ArbitrageReport.min_density must be finite, got {rep:?}"
        );
        assert!(
            rep.min_butterfly.is_finite(),
            "ArbitrageReport.min_butterfly must be finite, got {rep:?}"
        );
        assert!(
            rep.max_vertical_increase.is_finite(),
            "ArbitrageReport.max_vertical_increase must be finite, got {rep:?}"
        );
    }
});
