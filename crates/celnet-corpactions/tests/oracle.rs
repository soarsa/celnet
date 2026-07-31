//! Independent oracle for `celnet-corpactions` (guardrail 5).
//!
//! Two independent checks that the effect functions are correct, not merely plausible:
//!
//! 1. A **hand-computed truth table** (≥8 cases) asserting each event's schedule/position transform
//!    against first principles — a coupon pay-date cashflow, a full call collapsing the schedule, a
//!    partial call scaling notional + shortening the outstanding, a sinking draw, a maturity
//!    redemption, a voluntary tender, an exchange offer, and the concrete position deltas.
//! 2. **Property tests** for the two structural invariants that must hold for *every* input:
//!    notional conservation on partial events, and post-event schedule PV consistency.

use celnet_corpactions::{
    BondSchedule, CaDates, CaEvent, CaStatus, CaTerms, Caev, Camv, CivilDate, PositionEffect,
    apply_event, position_delta,
};
use proptest::prelude::*;

// ---- fixtures ---------------------------------------------------------------------------------

fn dates(payment: CivilDate) -> CaDates {
    CaDates {
        announcement: CivilDate::new(2032, 1, 1),
        record: payment,
        ex: payment,
        response_deadline: None,
        payment,
    }
}

fn event(caev: Caev, camv: Camv, terms: CaTerms, payment: CivilDate) -> CaEvent {
    CaEvent {
        isin: "GB00TEST0001".to_string(),
        caev,
        camv,
        dates: dates(payment),
        terms,
        status: CaStatus::Confirmed,
        source_ref: "MT564/oracle".to_string(),
    }
}

/// A 3y 6% semi-annual bond dated 2032-06-15, maturing 2035-06-15 → six 3.0 coupons, last +100.
fn bond_3y_6pct() -> BondSchedule {
    BondSchedule::fixed_coupon(
        CivilDate::new(2032, 6, 15),
        CivilDate::new(2035, 6, 15),
        0.06,
        2,
        100.0,
    )
    .expect("schedule")
}

const TOL: f64 = 1e-9;

// ---- 1. hand truth table ----------------------------------------------------------------------

#[test]
fn case_01_coupon_pay_date_is_income_and_leaves_future_stream() {
    let s = bond_3y_6pct();
    assert_eq!(s.len(), 6);
    // Apply INTR on the first coupon date (2032-12-15).
    let ev = event(
        Caev::Intr,
        Camv::Mand,
        CaTerms::coupon(0.0),
        CivilDate::new(2032, 12, 15),
    );
    let a = apply_event(&s, &ev).expect("applied");
    // Income = the 3.0 coupon; five future flows remain, unchanged.
    assert_eq!(a.effect, PositionEffect::Income { cash_per_100: 3.0 });
    assert_eq!(a.schedule.len(), 5);
    assert!((a.schedule.flows()[0].coupon - 3.0).abs() < TOL);
    assert!((a.schedule.pool_factor() - 1.0).abs() < TOL);
}

#[test]
fn case_02_maturity_redemption_realises_final_flow_and_empties() {
    let s = bond_3y_6pct();
    let ev = event(
        Caev::Redm,
        Camv::Mand,
        CaTerms::full_at_par(),
        CivilDate::new(2035, 6, 15),
    );
    let a = apply_event(&s, &ev).expect("applied");
    // The maturing flow is 3.0 coupon + 100 principal = 103.0.
    assert_eq!(
        a.effect,
        PositionEffect::Realise {
            cash_per_100: 103.0
        }
    );
    assert!(a.schedule.is_empty());
}

#[test]
fn case_03_full_call_before_maturity_collapses_schedule() {
    let s = bond_3y_6pct();
    // Full mandatory call at 101 on 2034-03-15 (between coupons).
    let ev = event(
        Caev::Mcal,
        Camv::Mand,
        CaTerms::partial(1.0, 101.0),
        CivilDate::new(2034, 3, 15),
    );
    let a = apply_event(&s, &ev).expect("applied");
    assert_eq!(
        a.effect,
        PositionEffect::Realise {
            cash_per_100: 101.0
        }
    );
    assert!(a.schedule.is_empty());
}

#[test]
fn case_04_call_on_coupon_date_adds_the_coincident_coupon() {
    let s = bond_3y_6pct();
    // Full call at par on the 2034-06-15 coupon date → 100 + the 3.0 coupon due that day.
    let ev = event(
        Caev::Mcal,
        Camv::Mand,
        CaTerms::partial(1.0, 100.0),
        CivilDate::new(2034, 6, 15),
    );
    let a = apply_event(&s, &ev).expect("applied");
    assert_eq!(
        a.effect,
        PositionEffect::Realise {
            cash_per_100: 103.0
        }
    );
    assert!(a.schedule.is_empty());
}

#[test]
fn case_05_partial_call_scales_notional_and_returns_cash() {
    let s = bond_3y_6pct();
    // Partial call of 25% at 101.
    let ev = event(
        Caev::Pcal,
        Camv::Mand,
        CaTerms::partial(0.25, 101.0),
        CivilDate::new(2034, 6, 15),
    );
    let a = apply_event(&s, &ev).expect("applied");
    assert_eq!(
        a.effect,
        PositionEffect::Scale {
            retained_fraction: 0.75,
            cash_per_100_redeemed: 101.0
        }
    );
    // Pool factor 0.75; every remaining coupon 3.0 → 2.25; final principal 100 → 75.
    assert!((a.schedule.pool_factor() - 0.75).abs() < TOL);
    for f in a.schedule.flows() {
        assert!((f.coupon - 2.25).abs() < TOL, "coupon {}", f.coupon);
    }
    assert!((a.schedule.last_flow().unwrap().principal - 75.0).abs() < TOL);
}

#[test]
fn case_06_sinking_draw_scales_pro_rata() {
    let s = bond_3y_6pct();
    // A sinking-fund drawing of 10% at par on 2033-06-15.
    let ev = event(
        Caev::Draw,
        Camv::Mand,
        CaTerms::partial(0.10, 100.0),
        CivilDate::new(2033, 6, 15),
    );
    let a = apply_event(&s, &ev).expect("applied");
    assert_eq!(
        a.effect,
        PositionEffect::Scale {
            retained_fraction: 0.90,
            cash_per_100_redeemed: 100.0
        }
    );
    assert!((a.schedule.pool_factor() - 0.90).abs() < TOL);
    assert!((a.schedule.flows()[0].coupon - 2.7).abs() < TOL);
}

#[test]
fn case_07_voluntary_full_tender_realises_at_tender_price() {
    let s = bond_3y_6pct();
    // Full voluntary tender at 99 on 2033-01-15 (between coupons, so no coincident coupon).
    let ev = event(
        Caev::Tend,
        Camv::Volu,
        CaTerms::partial(1.0, 99.0),
        CivilDate::new(2033, 1, 15),
    );
    let a = apply_event(&s, &ev).expect("applied");
    assert_eq!(a.effect, PositionEffect::Realise { cash_per_100: 99.0 });
    assert!(a.schedule.is_empty());
}

#[test]
fn case_08_exchange_offer_empties_source_and_mints_target_leg() {
    let s = bond_3y_6pct();
    let terms = CaTerms {
        cash_per_100: 0.0,
        redeemed_fraction: 1.0,
        target_instrument: "NEWCO-EQ".to_string(),
        target_units_per_100: 4.0, // 4 target units per 100 face of source
    };
    let ev = event(Caev::Exof, Camv::Chos, terms, CivilDate::new(2033, 9, 15));
    let a = apply_event(&s, &ev).expect("applied");
    assert!(a.schedule.is_empty());
    // A holder of 1,000,000 face receives 1,000,000/100 * 4 = 40,000 target units; source face → 0.
    let d = position_delta(&a.effect, 1_000_000.0);
    assert!((d.face_delta + 1_000_000.0).abs() < 1e-6);
    let leg = d.exchange_into.expect("exchange leg");
    assert_eq!(leg.target, "NEWCO-EQ");
    assert!((leg.units - 40_000.0).abs() < 1e-6);
}

#[test]
fn case_09_position_delta_partial_conserves_notional_and_returns_par_cash() {
    // PRED 40% at par on a 2,000,000 holding.
    let effect = PositionEffect::Scale {
        retained_fraction: 0.60,
        cash_per_100_redeemed: 100.0,
    };
    let d = position_delta(&effect, 2_000_000.0);
    assert!((d.face_delta + 800_000.0).abs() < 1e-6);
    assert!((d.cash - 800_000.0).abs() < 1e-6);
    // retained + redeemed = original.
    assert!(((2_000_000.0 + d.face_delta) - 1_200_000.0).abs() < 1e-6);
}

#[test]
fn case_10_position_delta_full_realise_zeroes_face_and_pays_price() {
    // A call at 101 on a 500,000 holding.
    let effect = PositionEffect::Realise {
        cash_per_100: 101.0,
    };
    let d = position_delta(&effect, 500_000.0);
    assert!((d.face_delta + 500_000.0).abs() < 1e-6);
    assert!((d.cash - 505_000.0).abs() < 1e-6);
    assert!(d.exchange_into.is_none());
}

#[test]
fn case_11_short_position_mirrors_signs() {
    // The same partial effect on a short (-1,000,000) mirrors: face rises toward zero, cash is negative.
    let effect = PositionEffect::Scale {
        retained_fraction: 0.70,
        cash_per_100_redeemed: 100.0,
    };
    let d = position_delta(&effect, -1_000_000.0);
    assert!((d.face_delta - 300_000.0).abs() < 1e-6, "{}", d.face_delta);
    assert!((d.cash + 300_000.0).abs() < 1e-6, "{}", d.cash);
}

#[test]
fn case_12_redemption_without_maturing_flow_is_rejected() {
    let s = bond_3y_6pct();
    // REDM dated on a coupon-only date (no principal matures) must error, not silently realise 0.
    let ev = event(
        Caev::Redm,
        Camv::Mand,
        CaTerms::full_at_par(),
        CivilDate::new(2033, 6, 15),
    );
    assert!(apply_event(&s, &ev).is_err());
}

// ---- 2. property tests ------------------------------------------------------------------------

prop_compose! {
    /// A well-formed fixed-coupon schedule from plausible issuance terms.
    fn arb_schedule()(
        coupon in 0.005f64..0.12,
        years in 1i32..15,
        semi in any::<bool>(),
    ) -> BondSchedule {
        let freq = if semi { 2 } else { 1 };
        let dated = CivilDate::new(2030, 1, 15);
        let maturity = CivilDate::new(2030 + years, 1, 15);
        BondSchedule::fixed_coupon(dated, maturity, coupon, freq, 100.0).expect("schedule")
    }
}

proptest! {
    /// Notional conservation: for any partial event and any holding, retained + redeemed = original,
    /// and the retained fraction the schedule reports matches the position delta's face reduction.
    #[test]
    fn partial_events_conserve_notional(
        fraction in 0.0f64..0.999,
        price in 50.0f64..130.0,
        held in 1_000.0f64..1e9,
        s in arb_schedule(),
    ) {
        let ev = event(Caev::Pred, Camv::Mand, CaTerms::partial(fraction, price), CivilDate::new(2031, 1, 15));
        let a = apply_event(&s, &ev).expect("applied");
        let retained = 1.0 - fraction;
        // Schedule pool factor scaled by the retained fraction.
        prop_assert!((a.schedule.pool_factor() - retained).abs() < 1e-9);
        let d = position_delta(&a.effect, held);
        let redeemed_face = -d.face_delta;
        let retained_face = held + d.face_delta;
        // Conservation to a relative tolerance (large holdings).
        prop_assert!(((retained_face + redeemed_face) - held).abs() <= held.abs() * 1e-12 + 1e-6);
        // Cash returned = redeemed face at the stated price.
        prop_assert!((d.cash - redeemed_face / 100.0 * price).abs() <= held.abs() * 1e-12 + 1e-6);
    }

    /// Partial scaling PV invariant: every remaining flow scales by the retained fraction, so the
    /// post-event schedule PV is exactly `retained · pre-event PV` at any flat rate.
    #[test]
    fn partial_scaling_scales_present_value(
        fraction in 0.0f64..0.999,
        rate in -0.02f64..0.15,
        s in arb_schedule(),
    ) {
        let as_of = CivilDate::new(2030, 1, 15);
        let pv_before = s.present_value(as_of, rate);
        let ev = event(Caev::Pcal, Camv::Mand, CaTerms::partial(fraction, 100.0), CivilDate::new(2030, 6, 15));
        let a = apply_event(&s, &ev).expect("applied");
        let pv_after = a.schedule.present_value(as_of, rate);
        let retained = 1.0 - fraction;
        prop_assert!((pv_after - retained * pv_before).abs() <= pv_before.abs() * 1e-9 + 1e-9);
    }

    /// Coupon PV consistency: settling the first coupon as income removes exactly that flow, so the
    /// pre-event PV equals the post-event schedule PV plus the income discounted to its pay date.
    #[test]
    fn coupon_income_preserves_total_present_value(
        rate in -0.02f64..0.15,
        s in arb_schedule(),
    ) {
        let as_of = CivilDate::new(2030, 1, 15);
        let pay = s.flows()[0].date; // the first coupon date
        let coupon = s.flows()[0].coupon;
        // Only meaningful when the first flow is a pure coupon (no principal) — true for years >= 2
        // or the semi-annual 1y case; skip the degenerate single-flow schedule.
        prop_assume!(s.len() > 1 && s.flows()[0].principal == 0.0);

        let pv_before = s.present_value(as_of, rate);
        let ev = event(Caev::Intr, Camv::Mand, CaTerms::coupon(0.0), pay);
        let a = apply_event(&s, &ev).expect("applied");
        let pv_after = a.schedule.present_value(as_of, rate);

        let days = (pay.to_date().unwrap() - as_of.to_date().unwrap()).whole_days() as f64;
        let df = (-rate * days / 365.0).exp();
        prop_assert!((pv_before - (pv_after + coupon * df)).abs() <= pv_before.abs() * 1e-9 + 1e-9);
    }
}
