//! The pure, deterministic effect functions — how each event transforms a schedule and a position.
//!
//! Every function here is a pure `(schedule + event) → (new schedule + effect)` or
//! `(effect + held face) → (concrete delta)` map: no IO, no clock, no rng, no mutation of the
//! inputs. This is the crate's whole reason to exist — the schedule/position math the ingestion
//! lifecycle (§7.5) drives and the pricing/hedging seams (§8) consume, validated against an
//! independent hand truth table in `tests/oracle.rs` (guardrail 5).
//!
//! The split between [`PositionEffect`] (the *shape* of the transform, in per-100-face terms) and
//! [`PositionDelta`] (the *concrete* change on a held face amount) mirrors the double-count guard
//! (§10.2): a redemption/coupon is attributed as a corporate action producing cash + a face change,
//! never as a trade.

use crate::date::CivilDate;
use crate::event::{CaEvent, Caev};
use crate::schedule::{BondSchedule, CaError};

/// The shape of the position transform an applied event makes, in per-100-face terms.
///
/// Deterministically derived from the event by [`apply_event`]; turned into a concrete book/inventory
/// movement for a specific holding by [`position_delta`].
#[derive(Debug, Clone, PartialEq)]
pub enum PositionEffect {
    /// A cash-income event ([`Caev::Intr`]): pays `cash_per_100` per 100 face; the position is
    /// unchanged.
    Income {
        /// Coupon cash per 100 face.
        cash_per_100: f64,
    },
    /// A full realisation (REDM / full MCAL / full BPUT / full TEND): the position goes to zero,
    /// returning `cash_per_100` per 100 face.
    Realise {
        /// Redemption / call / put / tender cash per 100 face.
        cash_per_100: f64,
    },
    /// A pro-rata scaling (PRED / PCAL / DRAW / partial BPUT / partial TEND): the position retains
    /// `retained_fraction` of its nominal; the redeemed part returns `cash_per_100_redeemed` per 100
    /// face of the *redeemed* portion.
    Scale {
        /// The fraction of nominal retained, in `[0, 1)`.
        retained_fraction: f64,
        /// Cash per 100 face on the redeemed portion.
        cash_per_100_redeemed: f64,
    },
    /// An exchange / conversion (EXOF / CONV): the source position goes to zero and a new position in
    /// `target` is created at `target_units_per_100` per 100 face of source. The *data* for these
    /// (the target and ratio) arrives via the pluggable corporate-CA adapter; the transform is exact.
    Exchange {
        /// The target instrument id.
        target: String,
        /// Units of the target created per 100 face of the source.
        target_units_per_100: f64,
    },
}

/// A leg of a holding created by an exchange / conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct ExchangeLeg {
    /// The target instrument id the holder now owns.
    pub target: String,
    /// The number of units of the target created for this holding.
    pub units: f64,
}

/// The concrete change an event makes to a specific holding of `held_face` nominal.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionDelta {
    /// The change in held face (nominal). Negative = a reduction; `-held_face` = fully redeemed.
    pub face_delta: f64,
    /// The cash thrown off by the event on the holding (a coupon, or principal returned), in the
    /// instrument's currency, for a `held_face`-sized position.
    pub cash: f64,
    /// For an exchange / conversion, the new instrument leg created; `None` otherwise.
    pub exchange_into: Option<ExchangeLeg>,
}

/// The result of applying an event: the post-event schedule and the position effect.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedEvent {
    /// The schedule of the (possibly scaled / collapsed / emptied) instrument after the event.
    pub schedule: BondSchedule,
    /// The shape of the position transform.
    pub effect: PositionEffect,
}

/// Validate a fraction is finite and in `[0, 1]`.
fn checked_fraction(f: f64) -> Result<f64, CaError> {
    if f.is_finite() && (0.0..=1.0).contains(&f) {
        Ok(f)
    } else {
        Err(CaError::InvalidFraction)
    }
}

/// Validate a cash term is finite and non-negative.
fn checked_cash(c: f64) -> Result<f64, CaError> {
    if c.is_finite() && c >= 0.0 {
        Ok(c)
    } else {
        Err(CaError::InvalidTerm)
    }
}

/// Apply a corporate-action event to a bond's schedule, producing the post-event schedule and the
/// position effect. **Pure and deterministic.**
///
/// The transform per CAEV (all in per-100-original-face terms):
/// - **INTR** — settle every coupon-only flow on/before the effective date as [`PositionEffect::Income`]
///   (the schedule's own coupon if `terms.cash_per_100 == 0`, else the stated amount); those flows leave
///   the remaining schedule, the future stream is untouched.
/// - **REDM** — realise the cashflow(s) on the effective (maturity) date; the schedule empties. Errors
///   [`CaError::NoFlowOnEffectiveDate`] if nothing matures there.
/// - **MCAL / full BPUT / full TEND** (or any event with `redeemed_fraction >= 1`) — realise at the
///   call/put/tender price plus any coupon coinciding with the effective date; every later flow is
///   dropped and the schedule empties.
/// - **PRED / PCAL / DRAW / partial BPUT / partial TEND** — scale every remaining flow (and the pool
///   factor) by `1 − redeemed_fraction`; the redeemed portion returns cash at the stated price.
/// - **EXOF / CONV** — the source schedule empties; a new position in the target is created at the ratio.
///
/// # Errors
/// [`CaError::InvalidFraction`] / [`CaError::InvalidTerm`] / [`CaError::InvalidExchange`] for malformed
/// terms; [`CaError::NoFlowOnEffectiveDate`] for a REDM with no maturing flow.
pub fn apply_event(schedule: &BondSchedule, event: &CaEvent) -> Result<AppliedEvent, CaError> {
    let effective = event.effective_date();
    let fraction = checked_fraction(event.terms.redeemed_fraction)?;
    let price = checked_cash(event.terms.cash_per_100)?;

    match event.caev {
        Caev::Intr => apply_income(schedule, effective, price),
        Caev::Redm => apply_full_redemption(schedule, effective, price, /*is_maturity=*/ true),
        Caev::Exof | Caev::Conv => apply_exchange(event),
        // Calls / puts / tenders / partial redemptions / sinking draws: full when the fraction is
        // whole (or the CAEV is inherently terminal), else a pro-rata scale.
        Caev::Mcal | Caev::Pcal | Caev::Pred | Caev::Draw | Caev::Bput | Caev::Tend => {
            if fraction >= 1.0 || event.caev.is_terminal() {
                apply_full_redemption(schedule, effective, price, /*is_maturity=*/ false)
            } else {
                apply_partial(schedule, fraction, price)
            }
        }
    }
}

/// INTR — settle coupon-only flows on/before the effective date as income; leave the future stream.
fn apply_income(
    schedule: &BondSchedule,
    effective: CivilDate,
    stated_coupon_per_100: f64,
) -> Result<AppliedEvent, CaError> {
    // The coupon actually due is the schedule's own coupon-only cash on/before the effective date;
    // a non-zero stated amount overrides it (a vendor feed that restates the coupon).
    let schedule_coupon: f64 = schedule
        .flows()
        .iter()
        .filter(|f| f.date <= effective && f.principal == 0.0)
        .map(|f| f.coupon)
        .sum();
    let cash_per_100 = if stated_coupon_per_100 > 0.0 {
        stated_coupon_per_100
    } else {
        schedule_coupon
    };
    Ok(AppliedEvent {
        schedule: schedule.without_coupons_through(effective),
        effect: PositionEffect::Income { cash_per_100 },
    })
}

/// REDM (maturity) / a full call/put/tender — realise and empty the schedule.
fn apply_full_redemption(
    schedule: &BondSchedule,
    effective: CivilDate,
    price_per_100: f64,
    is_maturity: bool,
) -> Result<AppliedEvent, CaError> {
    // Any coupon coinciding with the effective date is paid alongside the principal.
    let coincident_coupon: f64 = schedule
        .flows()
        .iter()
        .filter(|f| f.date == effective)
        .map(|f| f.coupon)
        .sum();

    let cash_per_100 = if is_maturity {
        // At true maturity the redemption cash is the maturing flow itself (principal + final
        // coupon), scaled to the current pool factor. Require a flow actually to mature here.
        let maturing: f64 = schedule
            .flows()
            .iter()
            .filter(|f| f.date == effective && f.principal > 0.0)
            .map(super::schedule::ScheduleFlow::total)
            .sum();
        if maturing == 0.0 {
            return Err(CaError::NoFlowOnEffectiveDate);
        }
        maturing
    } else {
        // A call/put/tender pays the (agreed) price on the still-outstanding nominal, plus the
        // coincident coupon. The price is per-100-*current*-face, so scale by the pool factor.
        price_per_100 * schedule.pool_factor() + coincident_coupon
    };

    // A full redemption realises the whole instrument: no future flows remain. (`truncated_after`
    // is exercised by the partial/sinking paths and the tests; here the collapse is total.)
    let _ = effective;
    Ok(AppliedEvent {
        schedule: BondSchedule::new(Vec::new())?,
        effect: PositionEffect::Realise { cash_per_100 },
    })
}

/// PRED / PCAL / DRAW / partial put / partial tender — scale the remaining stream pro-rata.
fn apply_partial(
    schedule: &BondSchedule,
    redeemed_fraction: f64,
    price_per_100: f64,
) -> Result<AppliedEvent, CaError> {
    let retained = 1.0 - redeemed_fraction;
    Ok(AppliedEvent {
        schedule: schedule.scaled(retained),
        effect: PositionEffect::Scale {
            retained_fraction: retained,
            cash_per_100_redeemed: price_per_100,
        },
    })
}

/// EXOF / CONV — the source empties; a new target position is created at the ratio.
fn apply_exchange(event: &CaEvent) -> Result<AppliedEvent, CaError> {
    if event.terms.target_instrument.is_empty()
        || !(event.terms.target_units_per_100.is_finite() && event.terms.target_units_per_100 > 0.0)
    {
        return Err(CaError::InvalidExchange);
    }
    Ok(AppliedEvent {
        schedule: BondSchedule::new(Vec::new())?,
        effect: PositionEffect::Exchange {
            target: event.terms.target_instrument.clone(),
            target_units_per_100: event.terms.target_units_per_100,
        },
    })
}

/// Turn a per-100 [`PositionEffect`] into the concrete change on a `held_face` holding.
///
/// - **Income** — cash only (`held_face / 100 · cash_per_100`); no face change.
/// - **Realise** — face → 0 (`face_delta = −held_face`) and the redemption cash on the whole holding.
/// - **Scale** — face reduced to `retained_fraction · held_face`; cash on the redeemed portion.
/// - **Exchange** — face → 0 and an [`ExchangeLeg`] of `held_face / 100 · target_units_per_100` units.
///
/// **Pure**; the same effect applied to a negative `held_face` (a short) mirrors the signs, so the
/// book's realise/scale path works unchanged for both sides.
#[must_use]
pub fn position_delta(effect: &PositionEffect, held_face: f64) -> PositionDelta {
    let per_unit = held_face / 100.0;
    match effect {
        PositionEffect::Income { cash_per_100 } => PositionDelta {
            face_delta: 0.0,
            cash: per_unit * cash_per_100,
            exchange_into: None,
        },
        PositionEffect::Realise { cash_per_100 } => PositionDelta {
            face_delta: -held_face,
            cash: per_unit * cash_per_100,
            exchange_into: None,
        },
        PositionEffect::Scale {
            retained_fraction,
            cash_per_100_redeemed,
        } => {
            let redeemed_face = held_face * (1.0 - retained_fraction);
            PositionDelta {
                face_delta: -redeemed_face,
                cash: (redeemed_face / 100.0) * cash_per_100_redeemed,
                exchange_into: None,
            }
        }
        PositionEffect::Exchange {
            target,
            target_units_per_100,
        } => PositionDelta {
            face_delta: -held_face,
            cash: 0.0,
            exchange_into: Some(ExchangeLeg {
                target: target.clone(),
                units: per_unit * target_units_per_100,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::CivilDate;
    use crate::event::{CaDates, CaStatus, CaTerms, Camv};

    fn dates(payment: CivilDate) -> CaDates {
        CaDates {
            announcement: CivilDate::new(2034, 1, 1),
            record: payment,
            ex: payment,
            response_deadline: None,
            payment,
        }
    }

    fn event(caev: Caev, terms: CaTerms, payment: CivilDate) -> CaEvent {
        CaEvent {
            isin: "TESTISIN0001".to_string(),
            caev,
            camv: Camv::Mand,
            dates: dates(payment),
            terms,
            status: CaStatus::Confirmed,
            source_ref: "t".to_string(),
        }
    }

    fn semi_2y() -> BondSchedule {
        BondSchedule::fixed_coupon(
            CivilDate::new(2033, 6, 15),
            CivilDate::new(2035, 6, 15),
            0.04,
            2,
            100.0,
        )
        .expect("schedule")
    }

    #[test]
    fn partial_redemption_scales_and_conserves_notional() {
        let s = semi_2y();
        let ev = event(
            Caev::Pred,
            CaTerms::partial(0.30, 100.0),
            CivilDate::new(2034, 6, 15),
        );
        let applied = apply_event(&s, &ev).expect("applied");
        // Retained 0.70, pool factor 0.70, coupons scaled to 2.0 * 0.70 = 1.4.
        assert!((applied.schedule.pool_factor() - 0.70).abs() < 1e-12);
        assert!((applied.schedule.flows()[0].coupon - 1.4).abs() < 1e-12);

        let delta = position_delta(&applied.effect, 1_000_000.0);
        assert!(
            (delta.face_delta + 300_000.0).abs() < 1e-6,
            "{}",
            delta.face_delta
        );
        assert!((delta.cash - 300_000.0).abs() < 1e-6, "{}", delta.cash);
        // Notional conservation: retained + redeemed = original.
        let retained = 1_000_000.0 + delta.face_delta;
        assert!((retained - 700_000.0).abs() < 1e-6);
    }

    #[test]
    fn full_call_collapses_schedule_and_realises() {
        let s = semi_2y();
        // Full call at 101 on 2034-09-15 (between the Jun/Dec coupons).
        let ev = event(
            Caev::Mcal,
            CaTerms::partial(1.0, 101.0),
            CivilDate::new(2034, 9, 15),
        );
        let applied = apply_event(&s, &ev).expect("applied");
        assert!(applied.schedule.is_empty());
        match applied.effect {
            PositionEffect::Realise { cash_per_100 } => {
                assert!((cash_per_100 - 101.0).abs() < 1e-12)
            }
            other => panic!("expected Realise, got {other:?}"),
        }
    }
}
