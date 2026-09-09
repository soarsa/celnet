//! XVA (CVA / DVA / FVA) valuation adjustments for the `PricingService::PriceXva`
//! edge RPC.
//!
//! This module is the faithful, allocation-light bridge from the wire contract
//! ([`celnet_proto::PriceXvaRequest`]) to the [`celnet_xva`] valuation-adjustment
//! engine and back ([`celnet_proto::XvaResult`]). Like [`crate::rates_pricing`] it
//! is deliberately a **pure function** — no IO, no live-market read — because the
//! caller supplies the whole market explicitly: the netting set of vanilla FX
//! options, the exposure-simulation configuration, and the counterparty/own
//! hazard-rate survival curves. That keeps the gRPC handler in
//! [`crate::services::pricing`] a thin `Status`-wrapping shell and lets the whole
//! conversion + numeric path be unit-tested without a server.
//!
//! ## What it does
//!
//! 1. Rebuilds the [`celnet_xva::NettingSet`] from the wire trades and the shared
//!    domestic/foreign rate environment, and the [`celnet_xva::ExposureConfig`]
//!    from the exposure-model parameters.
//! 2. Rebuilds the two [`celnet_xva::SurvivalCurve`]s — flat (empty pillar times,
//!    one hazard) or piecewise-constant (strictly-increasing pillars, per-segment
//!    hazards).
//! 3. Simulates the expected-exposure profile with scrambled-Sobol QMC
//!    ([`celnet_xva::ExposureProfile::simulate`]) and aggregates the CVA / DVA /
//!    FVA ([`celnet_xva::compute_xva`]), returning the three adjustments and the
//!    all-in total.
//!
//! ## Error taxonomy
//!
//! Every malformed input is a typed [`XvaPriceError`] the handler maps to
//! `Status::invalid_argument`; the invariant guard [`XvaPriceError::NonFiniteResult`]
//! (an adjustment came back non-finite despite validated inputs) maps to
//! `Status::internal`. Each variant is validated **before** calling the engine, so
//! the engine's own `assert!` preconditions (LGD range, positive horizon, curve
//! shape) are never reached with an out-of-contract value — a bad request is a
//! typed error, never a panic on the async edge.

use celnet_proto::{
    ExposureBucket as WireExposureBucket, OptionType as WireOptionType, PriceXvaRequest,
    XvaResult as WireXvaResult, XvaSurvivalCurve,
};
use celnet_types::OptionType;
use celnet_xva::{
    ExposureConfig, ExposureProfile, NettedTrade, NettingSet, SurvivalCurve, XvaInputs, compute_xva,
};

/// A typed failure of [`price_xva`]. Every variant except
/// [`XvaPriceError::NonFiniteResult`] is a malformed request mapped to
/// `Status::invalid_argument`; `NonFiniteResult` is an internal numeric fault
/// mapped to `Status::internal`.
#[derive(Debug, Clone, PartialEq)]
pub enum XvaPriceError {
    /// The request carried no trades (a netting set needs at least one trade).
    EmptyNettingSet,
    /// A trade's `option_type` tag was not a known [`WireOptionType`].
    UnknownOptionType(i32),
    /// A trade strike was not finite and strictly positive.
    NonPositiveStrike,
    /// A trade expiry (years) was not finite and strictly positive.
    NonPositiveExpiry,
    /// A trade vol was not finite and strictly positive.
    NonPositiveVol,
    /// A trade notional was not finite.
    NonFiniteNotional,
    /// A rate (`r_dom` / `r_for`) was not finite.
    NonFiniteRate,
    /// The initial spot `spot0` was not finite and strictly positive.
    NonPositiveSpot,
    /// The exposure-model diffusion vol `sigma` was not finite and non-negative.
    InvalidSigma,
    /// `paths` was zero (need at least one exposure path).
    NonPositivePaths,
    /// `exposure_steps` was zero (need at least one exposure step).
    NonPositiveSteps,
    /// The counterparty survival curve was absent.
    MissingCounterpartyCurve,
    /// The own survival curve was absent.
    MissingOwnCurve,
    /// A survival curve carried no hazard rates.
    EmptyHazards,
    /// A flat survival curve (empty pillar times) did not carry exactly one hazard.
    FlatCurveNeedsSingleHazard,
    /// A piecewise survival curve's pillar-time and hazard arrays differed in length.
    CurveLengthMismatch,
    /// A survival curve's pillar times were not strictly increasing and positive.
    NonIncreasingPillars,
    /// A survival curve carried a negative or non-finite hazard rate.
    InvalidHazard,
    /// A loss-given-default (`lgd_counterparty` / `lgd_own`) was outside `[0, 1]`.
    LgdOutOfRange,
    /// The funding spread was not finite.
    NonFiniteFundingSpread,
    /// A computed adjustment (CVA / DVA / FVA) came back non-finite — an internal
    /// numeric fault on otherwise-valid input.
    NonFiniteResult,
}

impl core::fmt::Display for XvaPriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyNettingSet => f.write_str("`trades` is empty (a netting set needs ≥ 1 trade)"),
            Self::UnknownOptionType(t) => write!(f, "unknown `option_type` tag {t}"),
            Self::NonPositiveStrike => f.write_str("a trade `strike` must be finite and > 0"),
            Self::NonPositiveExpiry => f.write_str("a trade `expiry_years` must be finite and > 0"),
            Self::NonPositiveVol => f.write_str("a trade `vol` must be finite and > 0"),
            Self::NonFiniteNotional => f.write_str("a trade `notional` must be finite"),
            Self::NonFiniteRate => f.write_str("`r_dom` and `r_for` must be finite"),
            Self::NonPositiveSpot => f.write_str("`spot0` must be finite and > 0"),
            Self::InvalidSigma => f.write_str("`sigma` must be finite and ≥ 0"),
            Self::NonPositivePaths => f.write_str("`paths` must be ≥ 1"),
            Self::NonPositiveSteps => f.write_str("`exposure_steps` must be ≥ 1"),
            Self::MissingCounterpartyCurve => f.write_str("missing `counterparty` survival curve"),
            Self::MissingOwnCurve => f.write_str("missing `own` survival curve"),
            Self::EmptyHazards => f.write_str("a survival curve carried no `hazard_rates`"),
            Self::FlatCurveNeedsSingleHazard => f.write_str(
                "a flat survival curve (empty `pillar_times`) must carry exactly one `hazard_rates` entry",
            ),
            Self::CurveLengthMismatch => {
                f.write_str("a survival curve's `pillar_times` and `hazard_rates` must be equal length")
            }
            Self::NonIncreasingPillars => {
                f.write_str("a survival curve's `pillar_times` must be strictly increasing and positive")
            }
            Self::InvalidHazard => f.write_str("a survival curve `hazard_rates` entry must be finite and ≥ 0"),
            Self::LgdOutOfRange => f.write_str("`lgd_counterparty` and `lgd_own` must be in [0, 1]"),
            Self::NonFiniteFundingSpread => f.write_str("`funding_spread` must be finite"),
            Self::NonFiniteResult => f.write_str("a computed XVA adjustment was non-finite"),
        }
    }
}

/// Decode a proto [`WireOptionType`] tag into the engine [`OptionType`].
fn decode_option_type(tag: i32) -> Result<OptionType, XvaPriceError> {
    WireOptionType::try_from(tag)
        .map(Into::into)
        .map_err(|_| XvaPriceError::UnknownOptionType(tag))
}

/// Rebuild a [`SurvivalCurve`] from its wire form, validating every engine
/// precondition first (so [`SurvivalCurve::flat`] / [`SurvivalCurve::piecewise`]
/// are only ever called with valid inputs and never panic).
fn decode_survival_curve(c: &XvaSurvivalCurve) -> Result<SurvivalCurve, XvaPriceError> {
    if c.hazard_rates.is_empty() {
        return Err(XvaPriceError::EmptyHazards);
    }
    for &h in &c.hazard_rates {
        if !(h.is_finite() && h >= 0.0) {
            return Err(XvaPriceError::InvalidHazard);
        }
    }
    if c.pillar_times.is_empty() {
        // Flat curve: exactly one hazard.
        match c.hazard_rates.as_slice() {
            [lambda] => Ok(SurvivalCurve::flat(*lambda)),
            _ => Err(XvaPriceError::FlatCurveNeedsSingleHazard),
        }
    } else {
        // Piecewise-constant curve: equal-length, strictly-increasing positive pillars.
        if c.pillar_times.len() != c.hazard_rates.len() {
            return Err(XvaPriceError::CurveLengthMismatch);
        }
        let mut prev = 0.0;
        for &p in &c.pillar_times {
            if !(p.is_finite() && p > prev) {
                return Err(XvaPriceError::NonIncreasingPillars);
            }
            prev = p;
        }
        Ok(SurvivalCurve::piecewise(
            c.pillar_times.clone(),
            c.hazard_rates.clone(),
        ))
    }
}

/// Compute the CVA / DVA / FVA of the [`PriceXvaRequest`]'s netting set.
///
/// The mapping is a faithful passthrough: the returned [`WireXvaResult`] carries
/// exactly the [`celnet_xva::compute_xva`] figures over the netting set, exposure
/// profile, and survival curves the request describes — the serving layer adds no
/// numerics of its own.
pub fn price_xva(req: &PriceXvaRequest) -> Result<WireXvaResult, XvaPriceError> {
    // --- netting set ------------------------------------------------------------
    if req.trades.is_empty() {
        return Err(XvaPriceError::EmptyNettingSet);
    }
    let mut trades = Vec::with_capacity(req.trades.len());
    for t in &req.trades {
        let option = decode_option_type(t.option_type)?;
        if !(t.strike.is_finite() && t.strike > 0.0) {
            return Err(XvaPriceError::NonPositiveStrike);
        }
        if !(t.expiry_years.is_finite() && t.expiry_years > 0.0) {
            return Err(XvaPriceError::NonPositiveExpiry);
        }
        if !(t.vol.is_finite() && t.vol > 0.0) {
            return Err(XvaPriceError::NonPositiveVol);
        }
        if !t.notional.is_finite() {
            return Err(XvaPriceError::NonFiniteNotional);
        }
        trades.push(NettedTrade::new(
            option,
            t.strike,
            t.expiry_years,
            t.vol,
            t.notional,
        ));
    }
    if !(req.r_dom.is_finite() && req.r_for.is_finite()) {
        return Err(XvaPriceError::NonFiniteRate);
    }
    let set = NettingSet::new(trades, req.r_dom, req.r_for);

    // --- exposure-model configuration ------------------------------------------
    if !(req.spot0.is_finite() && req.spot0 > 0.0) {
        return Err(XvaPriceError::NonPositiveSpot);
    }
    if !(req.sigma.is_finite() && req.sigma >= 0.0) {
        return Err(XvaPriceError::InvalidSigma);
    }
    let paths = usize::try_from(req.paths)
        .ok()
        .filter(|&p| p >= 1)
        .ok_or(XvaPriceError::NonPositivePaths)?;
    let steps = usize::try_from(req.exposure_steps)
        .ok()
        .filter(|&s| s >= 1)
        .ok_or(XvaPriceError::NonPositiveSteps)?;
    let cfg = ExposureConfig {
        spot0: req.spot0,
        sigma: req.sigma,
        paths,
        seed: req.seed,
    };

    // --- survival curves --------------------------------------------------------
    let counterparty = decode_survival_curve(
        req.counterparty
            .as_ref()
            .ok_or(XvaPriceError::MissingCounterpartyCurve)?,
    )?;
    let own = decode_survival_curve(req.own.as_ref().ok_or(XvaPriceError::MissingOwnCurve)?)?;

    // --- LGDs + funding ---------------------------------------------------------
    if !((0.0..=1.0).contains(&req.lgd_counterparty) && (0.0..=1.0).contains(&req.lgd_own)) {
        return Err(XvaPriceError::LgdOutOfRange);
    }
    if !req.funding_spread.is_finite() {
        return Err(XvaPriceError::NonFiniteFundingSpread);
    }

    // --- simulate the exposure profile + aggregate ------------------------------
    let profile = ExposureProfile::simulate(&set, &cfg, steps);
    let result = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &counterparty,
        own: &own,
        lgd_counterparty: req.lgd_counterparty,
        lgd_own: req.lgd_own,
        funding_spread: req.funding_spread,
    });

    let total = result.total_adjustment();
    if !(result.cva.is_finite()
        && result.dva.is_finite()
        && result.fva.is_finite()
        && total.is_finite())
    {
        return Err(XvaPriceError::NonFiniteResult);
    }

    let buckets = profile
        .buckets()
        .iter()
        .map(|b| WireExposureBucket {
            time_years: b.time_years,
            label: b.label.clone(),
            ee: b.ee,
            q25: b.q25,
            q75: b.q75,
            pfe_lo: b.pfe_lo,
            pfe: b.pfe,
            ene: b.ene,
            ene_band_lo: b.ene_band_lo,
            ene_band_hi: b.ene_band_hi,
        })
        .collect();

    Ok(WireXvaResult {
        cva: result.cva,
        dva: result.dva,
        fva: result.fva,
        total_adjustment: total,
        buckets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{XvaSurvivalCurve, XvaTrade};

    /// Build a representative request: a long call + short put netting set (so the
    /// net mark spans positive and negative exposure and every one of CVA / DVA /
    /// FVA is non-trivial), a piecewise counterparty curve and a flat own curve.
    fn representative_request() -> PriceXvaRequest {
        PriceXvaRequest {
            request_id: 42,
            trades: vec![
                XvaTrade {
                    option_type: WireOptionType::Call as i32,
                    strike: 1.10,
                    expiry_years: 1.0,
                    vol: 0.12,
                    notional: 1.0,
                },
                XvaTrade {
                    option_type: WireOptionType::Put as i32,
                    strike: 1.05,
                    expiry_years: 1.5,
                    vol: 0.14,
                    notional: -1.0,
                },
            ],
            r_dom: 0.03,
            r_for: 0.01,
            spot0: 1.10,
            sigma: 0.13,
            paths: 2048,
            seed: 0x00AB_CDEF,
            exposure_steps: 8,
            counterparty: Some(XvaSurvivalCurve {
                pillar_times: vec![0.5, 2.0],
                hazard_rates: vec![0.02, 0.05],
            }),
            own: Some(XvaSurvivalCurve {
                pillar_times: vec![],
                hazard_rates: vec![0.015],
            }),
            lgd_counterparty: 0.6,
            lgd_own: 0.55,
            funding_spread: 0.008,
            correlation_id: None,
        }
    }

    /// The serving layer is a faithful passthrough: a `PriceXva` request over a
    /// known netting set returns exactly the CVA / DVA / FVA a direct
    /// [`celnet_xva::compute_xva`] call over the same netting set, exposure config,
    /// and survival curves produces — to ≤ 1e-12 (in fact bit-identical, because
    /// the QMC exposure simulation is bit-reproducible on the seed).
    #[test]
    fn price_xva_is_a_faithful_passthrough_to_the_engine() {
        let mut req = representative_request();
        req.correlation_id = Some(7);
        let served = price_xva(&req).expect("prices");

        // The independent direct call — the same inputs assembled by hand against
        // the engine's own constructors, NOT by re-running the serving layer.
        let set = NettingSet::new(
            vec![
                NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.12, 1.0),
                NettedTrade::new(OptionType::Put, 1.05, 1.5, 0.14, -1.0),
            ],
            0.03,
            0.01,
        );
        let cfg = ExposureConfig {
            spot0: 1.10,
            sigma: 0.13,
            paths: 2048,
            seed: 0x00AB_CDEF,
        };
        let profile = ExposureProfile::simulate(&set, &cfg, 8);
        let counterparty = SurvivalCurve::piecewise(vec![0.5, 2.0], vec![0.02, 0.05]);
        let own = SurvivalCurve::flat(0.015);
        let direct = compute_xva(&XvaInputs {
            profile: &profile,
            counterparty: &counterparty,
            own: &own,
            lgd_counterparty: 0.6,
            lgd_own: 0.55,
            funding_spread: 0.008,
        });

        assert!(
            (served.cva - direct.cva).abs() <= 1e-12,
            "CVA passthrough: served {} vs direct {}",
            served.cva,
            direct.cva
        );
        assert!(
            (served.dva - direct.dva).abs() <= 1e-12,
            "DVA passthrough: served {} vs direct {}",
            served.dva,
            direct.dva
        );
        assert!(
            (served.fva - direct.fva).abs() <= 1e-12,
            "FVA passthrough: served {} vs direct {}",
            served.fva,
            direct.fva
        );
        assert!(
            (served.total_adjustment - direct.total_adjustment()).abs() <= 1e-12,
            "total passthrough: served {} vs direct {}",
            served.total_adjustment,
            direct.total_adjustment()
        );

        // The test genuinely exercises all three adjustments (not a trivial zero
        // passthrough): the long/short set has both positive and negative exposure.
        assert!(
            direct.cva > 0.0,
            "CVA should be positive, got {}",
            direct.cva
        );
        assert!(
            direct.dva > 0.0,
            "DVA should be positive, got {}",
            direct.dva
        );
        assert!(
            direct.fva.abs() > 0.0,
            "FVA should be non-zero, got {}",
            direct.fva
        );
    }

    #[test]
    fn empty_netting_set_is_rejected() {
        let mut req = representative_request();
        req.trades.clear();
        assert_eq!(price_xva(&req), Err(XvaPriceError::EmptyNettingSet));
    }

    #[test]
    fn lgd_out_of_range_is_rejected() {
        let mut req = representative_request();
        req.lgd_counterparty = 1.5;
        assert_eq!(price_xva(&req), Err(XvaPriceError::LgdOutOfRange));
    }

    #[test]
    fn non_positive_vol_is_rejected() {
        let mut req = representative_request();
        req.trades[0].vol = 0.0;
        assert_eq!(price_xva(&req), Err(XvaPriceError::NonPositiveVol));
    }

    #[test]
    fn zero_paths_is_rejected() {
        let mut req = representative_request();
        req.paths = 0;
        assert_eq!(price_xva(&req), Err(XvaPriceError::NonPositivePaths));
    }

    #[test]
    fn missing_counterparty_curve_is_rejected() {
        let mut req = representative_request();
        req.counterparty = None;
        assert_eq!(
            price_xva(&req),
            Err(XvaPriceError::MissingCounterpartyCurve)
        );
    }

    #[test]
    fn flat_curve_with_multiple_hazards_is_rejected() {
        let mut req = representative_request();
        req.own = Some(XvaSurvivalCurve {
            pillar_times: vec![],
            hazard_rates: vec![0.01, 0.02],
        });
        assert_eq!(
            price_xva(&req),
            Err(XvaPriceError::FlatCurveNeedsSingleHazard)
        );
    }

    #[test]
    fn non_increasing_pillars_are_rejected() {
        let mut req = representative_request();
        req.counterparty = Some(XvaSurvivalCurve {
            pillar_times: vec![2.0, 1.0],
            hazard_rates: vec![0.02, 0.05],
        });
        assert_eq!(price_xva(&req), Err(XvaPriceError::NonIncreasingPillars));
    }
}
