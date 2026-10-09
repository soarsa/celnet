//! End-to-end validation against independently worked bond examples and the crate's own identities.
//!
//! ## Oracle status (honest disclosure)
//!
//! QuantLib is **not runnable on this workstation** (the golden-env Python 3.14 toolchain is broken —
//! no `pip`, no importable `QuantLib`), so these vectors are **not** QuantLib-validated. The
//! independent oracles used here are instead:
//!
//! 1. **Closed-form annuity price** — `P = c·(1 − v^N)/i + F·v^N` (a geometric-series closed form),
//!    a different formula from the engine's explicit per-cashflow summation.
//! 2. **Hand-derived weighted-cashflow duration / convexity** for the primary vector, from the
//!    textbook definitions (independent of the engine's analytic derivatives).
//! 3. **Published street values** (price ≈ 94.76; durations ≈ 2.78 / 2.68) for the primary vector.
//! 4. **Round-trip** `ytm(dirty_price(y)) == y` and **par** (`price == F` on a coupon date ⇒
//!    `ytm == coupon`) identities across a family of bonds.
//!
//! All figures below are literals with the derivation in the comment, per GUIDE.md guardrail #5.

use celnet_bond::{
    AccrualBasis, Bond, PaymentFrequency, bond_risk, dirty_price, yield_to_maturity,
};
use celnet_types::Rate;
use time::{Date, Month};

fn d(y: i32, m: Month, day: u8) -> Date {
    Date::from_calendar_date(y, m, day).expect("valid date")
}

/// Closed-form level-coupon bond price on a coupon date (independent of the engine's summation).
fn annuity_price(coupon_rate: f64, face: f64, freq: f64, periods: u32, y: f64) -> f64 {
    let i = y / freq;
    let v = 1.0 / (1.0 + i);
    let vn = v.powi(periods as i32);
    coupon_rate / freq * face * (1.0 - vn) / i + face * vn
}

/// Vector 1 — 6% semi-annual, 3 years, on a coupon date, yield 8%.
///
/// Worked values (v = 1/1.04, N = 6):
///   dirty price = 94.75786  (closed-form annuity and published street value)
///   YTM(94.75786) = 8%      (round-trip)
///   Macaulay = 2.78306 y, modified = 2.67602 y, convexity = 8.77787 y²  (hand-derived).
#[test]
fn vector_6pct_3y_at_8pct() {
    let bond = Bond::new(
        d(2032, Month::June, 15),
        d(2035, Month::June, 15),
        0.06,
        PaymentFrequency::SemiAnnual,
        AccrualBasis::Thirty360BondBasis,
        100.0,
    )
    .expect("valid bond");

    let price = dirty_price(&bond, Rate(0.08)).expect("price");
    assert!((price - 94.75786).abs() < 1e-4, "price {price}");
    assert!(
        (price - annuity_price(0.06, 100.0, 2.0, 6, 0.08)).abs() < 1e-9,
        "engine vs closed-form annuity"
    );

    let risk = bond_risk(&bond, price).expect("risk");
    assert!(
        (risk.yield_to_maturity.0 - 0.08).abs() < 1e-10,
        "ytm {}",
        risk.yield_to_maturity.0
    );
    assert!(
        (risk.macaulay_duration - 2.78306).abs() < 1e-4,
        "macaulay {}",
        risk.macaulay_duration
    );
    assert!(
        (risk.modified_duration - 2.67602).abs() < 1e-4,
        "modified {}",
        risk.modified_duration
    );
    assert!(
        (risk.convexity - 8.77787).abs() < 1e-3,
        "convexity {}",
        risk.convexity
    );
    // On a coupon date there is no accrued interest, so clean == dirty.
    assert!(risk.accrued_interest.abs() < 1e-12);
    assert!((risk.clean_price - risk.dirty_price).abs() < 1e-12);
}

/// Vector 2 — 5% semi-annual, 5 years, on a coupon date, yield 6%.
///
/// Worked value (v = 1/1.03, N = 10): dirty price = 95.73490 (closed-form annuity), and
/// YTM(95.73490) = 6% (round-trip).
#[test]
fn vector_5pct_5y_at_6pct() {
    let bond = Bond::new(
        d(2030, Month::June, 15),
        d(2035, Month::June, 15),
        0.05,
        PaymentFrequency::SemiAnnual,
        AccrualBasis::Thirty360BondBasis,
        100.0,
    )
    .expect("valid bond");

    let price = dirty_price(&bond, Rate(0.06)).expect("price");
    assert!((price - 95.73490).abs() < 1e-4, "price {price}");
    assert!(
        (price - annuity_price(0.05, 100.0, 2.0, 10, 0.06)).abs() < 1e-9,
        "engine vs closed-form annuity"
    );
    let ytm = yield_to_maturity(&bond, price).expect("ytm");
    assert!((ytm.0 - 0.06).abs() < 1e-10, "ytm {}", ytm.0);
}

/// Vector 3 — quarterly 4% par bond, 7 years, priced at 100 on a coupon date ⇒ YTM == coupon.
///
/// The par identity holds at any frequency; here it also exercises the quarterly (f = 4) path.
#[test]
fn vector_par_bond_yields_coupon_quarterly() {
    let bond = Bond::new(
        d(2028, Month::June, 15),
        d(2035, Month::June, 15),
        0.04,
        PaymentFrequency::Quarterly,
        AccrualBasis::Thirty360BondBasis,
        100.0,
    )
    .expect("valid bond");

    // Priced at par on a coupon date, the yield equals the coupon exactly.
    let ytm = yield_to_maturity(&bond, 100.0).expect("ytm");
    assert!((ytm.0 - 0.04).abs() < 1e-10, "ytm {}", ytm.0);
    // ...and repricing at the coupon rate returns par.
    let price = dirty_price(&bond, Rate(0.04)).expect("price");
    assert!((price - 100.0).abs() < 1e-10, "par price {price}");
}
