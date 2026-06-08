//! Parity row — **American / Bermudan early-exercise** vanilla pricing reproduces
//! genuinely independent oracles.
//!
//! [`celnet_exotics::american_fd`] prices the early-exercise vanilla by a
//! projected-SOR (PSOR) free-boundary Crank-Nicolson finite difference. There is
//! no simple closed form for the American premium, so it is validated against
//! three oracles that are entirely independent of the production PSOR solver:
//!
//!   (i)   **No-early-exercise limit** (~FD precision): an American CALL on an
//!         asset with no foreign carry (`r_for = 0`) is never optimally exercised
//!         early, so it must equal the European Garman-Kohlhagen call. The
//!         European value here is an *independent, from-scratch* GK closed form
//!         built in this test from [`celnet_core::math`] — it does **not** call
//!         the production `celnet_vanilla` pricer, so the two share no code.
//!   (ii)  **Published reference** (Longstaff & Schwartz, 2001, Table 1): the
//!         American PUT with `S₀=K=40, r=0.06, σ=0.20, T=1` (no dividend ⇒
//!         `r_for=0`) has a published finite-difference value of `2.314`. A
//!         hand-pinned external constant — the canonical independent oracle.
//!   (iii) **Early-exercise premium ≥ 0** (structural, model-free): an American
//!         put is worth at least its European counterpart (strictly more rights).
//!         The European leg is again the independent from-scratch GK.

use celnet_core::math::{exp, ln, norm_cdf, sqrt};
use celnet_exotics::{AmericanGrid, AmericanOption, ExerciseStyle, american_fd};
use celnet_types::{OptionType, VanillaInputs};

/// Independent, from-scratch Garman-Kohlhagen European price — the *oracle*. No
/// dependency on the production `celnet_vanilla` pricer.
fn gk_european(
    opt: OptionType,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    let srt = vol * sqrt(t);
    let d1 = (ln(spot / strike) + (r_dom - r_for + 0.5 * vol * vol) * t) / srt;
    let d2 = d1 - srt;
    let df_dom = exp(-r_dom * t);
    let df_for = exp(-r_for * t);
    match opt {
        OptionType::Call => spot * df_for * norm_cdf(d1) - strike * df_dom * norm_cdf(d2),
        OptionType::Put => strike * df_dom * norm_cdf(-d2) - spot * df_for * norm_cdf(-d1),
    }
}

fn american(opt: OptionType, strike: f64) -> AmericanOption {
    AmericanOption {
        option: opt,
        strike,
        style: ExerciseStyle::American,
    }
}

fn fine_grid() -> AmericanGrid {
    AmericanGrid {
        space_steps: 2000,
        time_steps: 2000,
        ..AmericanGrid::default()
    }
}

/// (i) An American call with no foreign carry is never exercised early ⇒ it equals
/// the European GK call (independent from-scratch oracle).
#[test]
fn american_call_no_carry_equals_european() {
    let (spot, strike, vol, t, r_dom, r_for) = (1.10, 1.12, 0.10, 1.0, 0.02, 0.0);
    let i = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
    let fd = american_fd(&i, &american(OptionType::Call, strike), fine_grid());
    let euro = gk_european(OptionType::Call, spot, strike, vol, t, r_dom, r_for);
    assert!(
        (fd - euro).abs() < 5e-4,
        "American call (r_for=0) FD {fd} must equal European GK {euro}"
    );
}

/// (ii) Longstaff & Schwartz (2001) Table 1 American put — published FD = 2.314.
#[test]
fn american_put_matches_published_longstaff_schwartz_2001_table1() {
    let i = VanillaInputs::new(40.0, 40.0, 0.20, 1.0, 0.06, 0.0);
    let fd = american_fd(&i, &american(OptionType::Put, 40.0), fine_grid());
    const PUBLISHED_FD: f64 = 2.314;
    assert!(
        (fd - PUBLISHED_FD).abs() < 1e-2,
        "American put (LS 2001 Table 1) FD {fd} vs published {PUBLISHED_FD}"
    );
}

/// (iii) The American put dominates the European put (early-exercise premium ≥ 0)
/// across an ITM-to-ATM spot ladder where the premium is unambiguously positive.
#[test]
fn american_put_early_exercise_premium_nonnegative() {
    let (strike, vol, t, r_dom, r_for) = (1.10_f64, 0.15, 1.0, 0.05, 0.0);
    let grid = AmericanGrid {
        space_steps: 1000,
        time_steps: 1000,
        ..AmericanGrid::default()
    };
    for &spot in &[1.00_f64, 1.05, 1.10] {
        let i = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
        let fd = american_fd(&i, &american(OptionType::Put, strike), grid);
        let euro = gk_european(OptionType::Put, spot, strike, vol, t, r_dom, r_for);
        assert!(
            fd >= euro - 1e-4,
            "American put {fd} must be ≥ European put {euro} at spot {spot}"
        );
    }
}
