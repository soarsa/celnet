//! The `RiskService.CombinedTailRisk` compute core — the wire surface of the C2c
//! unified risk cube [`celnet_risk_cube::combined_tail_risk`] (the R-a "true single
//! VaR engine").
//!
//! The vertical is *inline portfolio → build books + scenarios → ONE joint
//! bump-and-revalue → wire*:
//!
//! 1. Build the vanilla FX option legs into [`celnet_risk_normalize::PositionRisk`]
//!    (the exact [`PositionRisk::fx`] arguments — the canonical convention-free risk
//!    is re-derived from the raw Garman-Kohlhagen inputs, never a convention-baked
//!    Greek), reusing the same enum/`CcyPair` conversions as
//!    [`super::convert::position_to_fact`].
//! 2. Build the linear-FI legs into [`celnet_rates_risk::FiPosition`] and the base
//!    discount curve into [`celnet_rates_risk::RatePillars`].
//! 3. Build the aligned joint scenarios into [`celnet_risk_cube::JointScenario`].
//! 4. Call [`combined_tail_risk`] ONCE (the joint tail + FI key-rate axis + signed
//!    parallel DV01) and map the [`celnet_risk_cube::CombinedTailRisk`] to the wire.
//!
//! Pure with respect to the edge: the whole portfolio + scenario config travels on
//! the request (no store read, no live-market read), so every replica computes the
//! identical result — the joint-tail analogue of [`super::super::rates_risk::aggregate`].
//! The closed-form exotic pricer ([`super::exotic_pricer::ExoticEngine`]) is injected
//! so the seam is production-real; exotic legs are not yet carried on this wire (the
//! joint-tail engine receives an empty exotic-leg slice), a purely additive follow-on.

// `tonic::Status` is a large error type carried by value across the `RiskService`
// surface (mirrors the rest of the risk edge).
#![allow(clippy::result_large_err)]

use celnet_proto::{
    CombinedTailRiskRequest, CombinedTailRiskResponse, JointTailScenario, TailRiskCurvePillar,
    TailRiskFiPosition, TailRiskKeyRate, TailRiskOptionLeg, VarEs as WireVarEs,
    tail_risk_fi_position,
};
use celnet_rates::{FixedPeriod, OisSchedule};
use celnet_rates_risk::{FiPosition, RatePillars, RateShock};
use celnet_risk_cube::{FiBook, JointScenario, Scenario, combined_tail_risk};
use celnet_risk_normalize::{AssetPricer, PositionRisk};
use celnet_types::{CcyPair, DeltaConvention, OptionType, PremiumStyle, Rate, Time, VanillaInputs};
use tonic::Status;

use super::exotic_pricer::ExoticEngine;

/// The default VaR/ES confidence level used when the request leaves `alpha` at the
/// proto3 zero (mirrors the `AggregateRisk` `var_alpha` default of 0.99).
const DEFAULT_ALPHA: f64 = 0.99;

/// Compute the joint options+FI tail risk for one inline request.
///
/// Builds the option book, the FI book + base curve, and the aligned joint
/// scenarios, then invokes [`combined_tail_risk`] once and maps the result to the
/// wire response — a faithful transport of the cube engine.
///
/// # Errors
/// `invalid_argument` for a malformed option leg (bad pair / unknown enum tag), a
/// malformed FI leg (missing arm, invalid OIS schedule), an empty or non-increasing
/// base curve, or a rate shock whose length does not match the base-curve pillar
/// count (surfaced by the cube's shocked-curve construction).
pub fn combined_tail_risk_response(
    req: &CombinedTailRiskRequest,
) -> Result<CombinedTailRiskResponse, Status> {
    let positions = build_option_legs(&req.option_legs)?;
    let fi_positions = build_fi_positions(&req.fi_positions)?;
    let base = build_base_curve(&req.base_curve)?;
    let scenarios = build_scenarios(&req.scenarios);
    let alpha = if req.alpha == 0.0 {
        DEFAULT_ALPHA
    } else {
        req.alpha
    };

    // The one joint bump-and-revalue: options (vanilla) + FI legs under aligned
    // (spot/vol/carry, rate) shocks, reduced by the single platform-wide tail
    // primitive, plus the FI key-rate axis and signed parallel DV01.
    let report = combined_tail_risk(
        &AssetPricer,
        &ExoticEngine,
        &positions,
        &[],
        FiBook::new(&fi_positions, &base),
        &scenarios,
        alpha,
    )
    .map_err(|e| Status::invalid_argument(format!("combined tail risk: {e}")))?;

    Ok(CombinedTailRiskResponse {
        joint_var_es: Some(WireVarEs {
            var: report.joint_var_es.var,
            es: report.joint_var_es.es,
        }),
        key_rate: report
            .key_rate
            .points()
            .iter()
            .map(|p| TailRiskKeyRate {
                tenor_years: p.tenor_years,
                dv01: p.dv01,
            })
            .collect(),
        fi_parallel_dv01: report.fi_parallel_dv01,
        correlation_id: req.correlation_id,
    })
}

/// Build every vanilla FX option leg into its canonical [`PositionRisk`].
fn build_option_legs(legs: &[TailRiskOptionLeg]) -> Result<Vec<PositionRisk>, Status> {
    legs.iter().map(option_leg_to_position).collect()
}

/// One wire option leg → [`PositionRisk::fx`] (the identical conversion primitives as
/// [`super::convert::position_to_fact`], minus the store identity/attribution).
fn option_leg_to_position(leg: &TailRiskOptionLeg) -> Result<PositionRisk, Status> {
    let wire_pair = leg
        .pair
        .clone()
        .ok_or_else(|| Status::invalid_argument("TailRiskOptionLeg missing `pair`"))?;
    let pair = CcyPair::try_from(wire_pair).map_err(|e| Status::invalid_argument(e.to_string()))?;
    let option = OptionType::from(
        celnet_proto::OptionType::try_from(leg.option_type)
            .map_err(|_| Status::invalid_argument("unknown OptionType"))?,
    );
    let quoted_delta = DeltaConvention::from(
        celnet_proto::DeltaConvention::try_from(leg.quoted_delta)
            .map_err(|_| Status::invalid_argument("unknown DeltaConvention"))?,
    );
    let premium_style = PremiumStyle::from(
        celnet_proto::PremiumStyle::try_from(leg.premium_style)
            .map_err(|_| Status::invalid_argument("unknown PremiumStyle"))?,
    );
    let inputs = VanillaInputs::new(leg.spot, leg.strike, leg.vol, leg.t, leg.r_dom, leg.r_for);
    Ok(PositionRisk::fx(
        pair,
        option,
        leg.notional_base,
        inputs,
        quoted_delta,
        premium_style,
    ))
}

/// Build every linear-FI leg into its domain [`FiPosition`].
fn build_fi_positions(legs: &[TailRiskFiPosition]) -> Result<Vec<FiPosition>, Status> {
    legs.iter().map(fi_position_to_domain).collect()
}

/// One wire FI leg → [`FiPosition`]. The `ois_swap` arm builds an explicit
/// [`OisSchedule`] in curve year-fraction coordinates (the schedule construction
/// validates monotone, positive-accrual periods).
fn fi_position_to_domain(p: &TailRiskFiPosition) -> Result<FiPosition, Status> {
    match &p.position {
        Some(tail_risk_fi_position::Position::OisSwap(s)) => {
            let periods: Vec<FixedPeriod> = s
                .periods
                .iter()
                .map(|c| FixedPeriod {
                    pay: Time(c.pay),
                    accrual: Time(c.accrual),
                })
                .collect();
            let schedule = OisSchedule::new(Time(s.start), periods)
                .map_err(|e| Status::invalid_argument(format!("OIS schedule: {e}")))?;
            Ok(FiPosition::OisSwap {
                schedule,
                fixed_rate: Rate(s.fixed_rate),
                notional: s.notional,
                receive_fixed: s.receive_fixed,
            })
        }
        None => Err(Status::invalid_argument(
            "TailRiskFiPosition needs its `position` arm set (ois_swap)",
        )),
    }
}

/// Build the base discount-curve zero-rate pillars into a validated [`RatePillars`]
/// (rejects an empty or non-increasing pillar set — the discount curve is required).
fn build_base_curve(pillars: &[TailRiskCurvePillar]) -> Result<RatePillars, Status> {
    let grid: Vec<(Time, Rate)> = pillars
        .iter()
        .map(|p| (Time(p.t), Rate(p.zero_rate)))
        .collect();
    RatePillars::new(grid).map_err(|e| Status::invalid_argument(format!("base curve: {e}")))
}

/// Build the aligned joint scenarios into [`JointScenario`]s. Infallible: a rate
/// shock's pillar-length is validated only when it is actually applied (FI legs
/// present), by the cube's shocked-curve construction.
fn build_scenarios(scenarios: &[JointTailScenario]) -> Vec<JointScenario> {
    scenarios
        .iter()
        .map(|s| {
            JointScenario::new(
                Scenario {
                    spot_rel: s.spot_rel,
                    vol_abs: s.vol_abs,
                    discount_abs: s.discount_abs,
                    carry_abs: s.carry_abs,
                },
                RateShock::new(s.rate_shifts.clone()),
            )
        })
        .collect()
}
