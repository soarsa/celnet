//! A complete reference plugin built only on `celnet-types` + `celnet-core`.
//!
//! [`FlatSmilePricer`] is a *fully implemented* (not a mock) SDK model that ties
//! the three trait seams together: it wraps a [`celnet_core::FlatSmile`], so it
//! is a [`crate::SmileModel`], and it prices options off that constant vol via
//! the **generalized Black-Scholes** cost-of-carry form, so it is also a
//! [`crate::PricingModel`]. A matching [`FlatSmileCalibration`] fits the single
//! flat-vol parameter to a weighted set of vol quotes in closed form. Together
//! they demonstrate — and test — that the SDK contract is sufficient to author a
//! real model with no dependency beyond the two frozen interface crates.
//!
//! Because the pricing seam takes the carry-tagged [`celnet_core::CarryInputs`]
//! and prices off the discount rate `r` and net carry `b` of its
//! [`celnet_types::Carry`], this one model prices **either** asset class: the FX
//! arm ([`celnet_types::Carry::FxRates`], `r = r_dom`, `b = r_dom − r_for`) is
//! byte-identical to the FX Garman-Kohlhagen form, and the equity/commodity arm
//! ([`celnet_types::Carry::CostOfCarry`], `b = r − q`) is the same closed form
//! with a single discount rate. The rate sensitivities are reported through the
//! carry-tagged [`celnet_types::RateSensitivities`], so an FX result carries the
//! two FX rhos and a cost-of-carry result carries the discount/carry rho pair.
//!
//! This deliberately reproduces the generalized-Black-Scholes math from
//! `celnet_core::math` rather than depending on `celnet-vanilla`, keeping this
//! crate dependency-light per the SDK layering rules.

use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_core::{CarryGreeks, CarryInputs, FlatSmile, Smile};
use celnet_types::{Carry, OptionType, RateSensitivities, Vol};

use crate::calibration::{Calibration, CalibrationReport, CalibrationTarget};
use crate::descriptor::{GreekSupport, ModelDescriptor, ModelId, ModelKind};
use crate::error::{PluginError, PluginResult};
use crate::pricing::PricingModel;
use crate::rates::{RatesCurvePillar, RatesMeasures, RatesPricingModel, RatesTerms};
use crate::smile::SmileModel;

/// The stable identity advertised by the reference flat-vol pricer.
pub const FLAT_PRICER_ID: ModelId = ModelId("celnet.reference.flat-vol-pricer");
/// The stable identity advertised by the reference flat-vol calibrator.
pub const FLAT_CALIBRATION_ID: ModelId = ModelId("celnet.reference.flat-vol-calibration");

/// A reference vanilla pricer driven by a constant (flat) volatility smile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlatSmilePricer {
    /// The constant-vol smile this model prices off.
    pub smile: FlatSmile,
}

impl FlatSmilePricer {
    /// Construct a pricer at the given absolute flat volatility.
    #[must_use]
    pub const fn new(vol: f64) -> Self {
        Self {
            smile: FlatSmile::new(vol),
        }
    }

    /// Validate inputs once, shared by `price` and `price_and_greeks`.
    ///
    /// `is_finite()` guards precede the magnitude checks so that a `NaN` field is
    /// rejected as non-finite rather than slipping through a comparison (`NaN`
    /// compares false to everything). The discount rate `r` and net carry `b`
    /// (whichever [`Carry`] arm supplied them) must both be finite.
    fn validate(inputs: &CarryInputs) -> PluginResult<()> {
        if !inputs.spot.is_finite() || inputs.spot <= 0.0 {
            return Err(PluginError::InvalidInput("spot must be positive"));
        }
        if !inputs.strike.is_finite() || inputs.strike <= 0.0 {
            return Err(PluginError::InvalidInput("strike must be positive"));
        }
        if !inputs.t.is_finite() || inputs.t <= 0.0 {
            return Err(PluginError::InvalidInput("time must be positive"));
        }
        if !inputs.carry.discount_rate().is_finite() || !inputs.carry.carry_rate().is_finite() {
            return Err(PluginError::InvalidInput("rates must be finite"));
        }
        if !inputs.vol.is_finite() || inputs.vol <= 0.0 {
            return Err(PluginError::InvalidInput("vol must be positive and finite"));
        }
        Ok(())
    }
}

// --- Smile seam --------------------------------------------------------------

impl Smile for FlatSmilePricer {
    fn implied_vol(&self, strike: f64, forward: f64, t: f64) -> Vol {
        self.smile.implied_vol(strike, forward, t)
    }
}

impl SmileModel for FlatSmilePricer {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(FLAT_PRICER_ID, ModelKind::Smile, GreekSupport::PRICE_ONLY)
    }
}

// --- Pricing seam ------------------------------------------------------------

/// Intermediate quantities shared by price and Greeks (Garman-Kohlhagen).
struct Aux {
    d1: f64,
    d2: f64,
    sqt: f64,
    vsqt: f64,
}

impl FlatSmilePricer {
    fn aux(inputs: &CarryInputs) -> Aux {
        let b = inputs.carry.carry_rate();
        let sqt = sqrt(inputs.t);
        let vsqt = inputs.vol * sqt;
        let d1 = (ln(inputs.spot / inputs.strike) + (b + 0.5 * inputs.vol * inputs.vol) * inputs.t)
            / vsqt;
        let d2 = d1 - vsqt;
        Aux { d1, d2, sqt, vsqt }
    }

    /// The carry-discount factor on the spot leg, `e^{(b − r)·t}`. For FX
    /// (`b = r_dom − r_for`, `r = r_dom`) this is exactly `e^{−r_for·t} = df_for`.
    fn spot_disc(inputs: &CarryInputs) -> f64 {
        exp((inputs.carry.carry_rate() - inputs.carry.discount_rate()) * inputs.t)
    }
}

impl PricingModel for FlatSmilePricer {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(FLAT_PRICER_ID, ModelKind::Pricing, GreekSupport::FULL)
    }

    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> PluginResult<f64> {
        Self::validate(inputs)?;
        let a = Self::aux(inputs);
        // Generalized Black-Scholes: carry-discount the spot leg by e^{(b−r)t}
        // and numeraire-discount the strike leg by e^{−rt}. For FX (`b = r_dom −
        // r_for`, `r = r_dom`) these are byte-identical to `df_for`/`df_dom`.
        let s_disc = inputs.spot * Self::spot_disc(inputs);
        let k_disc = inputs.strike * inputs.discount_df();
        Ok(match opt {
            OptionType::Call => s_disc * norm_cdf(a.d1) - k_disc * norm_cdf(a.d2),
            OptionType::Put => k_disc * norm_cdf(-a.d2) - s_disc * norm_cdf(-a.d1),
        })
    }

    #[allow(clippy::similar_names)] // d1/d2, nd1/nd2 are the canonical option-pricing names
    #[allow(clippy::too_many_lines)] // one cohesive generalized-BSM price+greeks block
    fn price_and_greeks(&self, opt: OptionType, inputs: &CarryInputs) -> PluginResult<CarryGreeks> {
        Self::validate(inputs)?;
        let a = Self::aux(inputs);
        let (d1, d2, sqt, vsqt) = (a.d1, a.d2, a.sqt, a.vsqt);
        let (s, k, t, vol) = (inputs.spot, inputs.strike, inputs.t, inputs.vol);
        // The discount rate `r` and net carry `b` are the only rate quantities the
        // generalized form needs; the cost-of-carry of the spot leg is `b − r`.
        let r = inputs.carry.discount_rate();
        let b = inputs.carry.carry_rate();
        let df_dom = inputs.discount_df();
        let df_for = Self::spot_disc(inputs);

        let pd1 = norm_pdf(d1);
        let nd1 = norm_cdf(d1);
        let nd2 = norm_cdf(d2);
        let nmd1 = norm_cdf(-d1);
        let nmd2 = norm_cdf(-d2);

        let s_disc = s * df_for;
        let k_disc = k * df_dom;

        let price = match opt {
            OptionType::Call => s_disc * nd1 - k_disc * nd2,
            OptionType::Put => k_disc * nmd2 - s_disc * nmd1,
        };

        // Spot / forward delta (premium-unadjusted).
        let delta_spot = match opt {
            OptionType::Call => df_for * nd1,
            OptionType::Put => -df_for * nmd1,
        };
        // Forward delta (driftless): N(d1) for a call, N(d1) - 1 for a put.
        let delta_forward = match opt {
            OptionType::Call => nd1,
            OptionType::Put => nd1 - 1.0,
        };

        // Second/third-order sensitivities (identical magnitude for call/put).
        let gamma = df_for * pd1 / (s * vsqt);
        let vega = s_disc * pd1 * sqt;
        let vanna = -df_for * pd1 * d2 / vol;
        let volga = vega * d1 * d2 / vol;
        let speed = -gamma / s * (d1 / vsqt + 1.0);
        let zomma = gamma * (d1 * d2 - 1.0) / vol;

        // The spot leg's carry drift `q ≡ r − b` (FX: `r_for`); it is the rate the
        // carry-discount factor `e^{−q·t}` decays at, and appears in theta/charm.
        let q = r - b;

        // Theta = ∂V/∂t (per year). Common term + carry terms differ by type.
        let theta_common = -s_disc * pd1 * vol / (2.0 * sqt);
        let theta = match opt {
            OptionType::Call => theta_common + q * s_disc * nd1 - r * k_disc * nd2,
            OptionType::Put => theta_common - q * s_disc * nmd1 + r * k_disc * nmd2,
        };

        // Charm = ∂(delta_spot)/∂T and Color = ∂(gamma)/∂T. Same closed forms that
        // are finite-difference-validated in `celnet-vanilla`: with net carry `b`,
        // ∂d1/∂T = b/(σ√T) − d1/(2T) + σ/(2√T).
        let dd1_dt = b / vsqt - d1 / (2.0 * t) + 0.5 * vol / sqt;
        let charm = match opt {
            OptionType::Call => -q * df_for * nd1 + df_for * pd1 * dd1_dt,
            OptionType::Put => q * df_for * nmd1 + df_for * pd1 * dd1_dt,
        };
        let color = gamma * (-q - 1.0 / (2.0 * t) - d1 * dd1_dt);

        // Rate sensitivities, tagged by the carry's asset class. The FX arm
        // reports the two FX rhos via the same closed forms as before (so the FX
        // projection is byte-identical); the cost-of-carry arm reports
        // ∂V/∂r (discount rho) and ∂V/∂b (carry rho).
        let rates = match inputs.carry {
            Carry::FxRates { .. } => {
                let rho_dom = match opt {
                    OptionType::Call => k * t * df_dom * nd2,
                    OptionType::Put => -k * t * df_dom * nmd2,
                };
                let rho_for = match opt {
                    OptionType::Call => -s * t * df_for * nd1,
                    OptionType::Put => s * t * df_for * nmd1,
                };
                RateSensitivities::Fx { rho_dom, rho_for }
            }
            Carry::CostOfCarry { .. } => {
                // ∂V/∂b = carry rho: differentiate F = S·e^{bt} ⇒ the spot leg
                // gains `S·t·e^{(b−r)t}·N(±d1)`.
                let carry_rho = match opt {
                    OptionType::Call => s * t * df_for * nd1,
                    OptionType::Put => -s * t * df_for * nmd1,
                };
                // ∂V/∂r = discount rho holding `b` fixed: only the e^{−rt} numeraire
                // discount on the strike leg responds.
                let discount_rho = match opt {
                    OptionType::Call => -k * t * df_dom * nd2,
                    OptionType::Put => k * t * df_dom * nmd2,
                };
                RateSensitivities::Carry {
                    discount_rho,
                    carry_rho,
                }
            }
        };

        Ok(CarryGreeks {
            price,
            delta_spot,
            delta_forward,
            gamma,
            vega,
            theta,
            rates,
            vanna,
            volga,
            charm,
            speed,
            zomma,
            color,
        })
    }
}

// --- Calibration seam --------------------------------------------------------

/// A closed-form calibrator that fits the single flat-vol parameter to a set of
/// weighted Black-vol quotes (the weighted mean, which is the exact
/// least-squares minimizer for a constant model). Produces a ready-to-use
/// [`FlatSmilePricer`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FlatSmileCalibration;

impl Calibration for FlatSmileCalibration {
    type Model = FlatSmilePricer;
    type Target = CalibrationTarget;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(
            FLAT_CALIBRATION_ID,
            ModelKind::Calibration,
            GreekSupport::PRICE_ONLY,
        )
    }

    fn calibrate(
        &self,
        forward: f64,
        t: f64,
        targets: &[CalibrationTarget],
    ) -> PluginResult<(FlatSmilePricer, CalibrationReport)> {
        if !forward.is_finite() || forward <= 0.0 || !t.is_finite() || t <= 0.0 {
            return Err(PluginError::InvalidInput("forward and t must be positive"));
        }
        if targets.is_empty() {
            return Err(PluginError::InvalidInput("no calibration targets"));
        }
        let mut sw = 0.0_f64;
        let mut swv = 0.0_f64;
        for tg in targets {
            if !tg.weight.is_finite() || tg.weight < 0.0 {
                return Err(PluginError::InvalidInput("weights must be finite and >= 0"));
            }
            if !tg.observed.is_finite() {
                return Err(PluginError::InvalidInput("observed vol must be finite"));
            }
            sw += tg.weight;
            swv += tg.weight * tg.observed;
        }
        if !sw.is_finite() || sw <= 0.0 {
            return Err(PluginError::InvalidInput("weights sum to zero"));
        }
        let fitted = swv / sw;
        if !fitted.is_finite() || fitted <= 0.0 {
            return Err(PluginError::CalibrationFailed("fitted vol not positive"));
        }

        // Fit residuals against the closed-form minimizer.
        let mut sse = 0.0_f64;
        let mut max_residual = 0.0_f64;
        for tg in targets {
            let r = tg.observed - fitted;
            sse += tg.weight * r * r;
            max_residual = max_residual.max(r.abs());
        }
        let rms_residual = sqrt(sse / sw);

        let report = CalibrationReport {
            iterations: 1,
            rms_residual,
            max_residual,
        };
        Ok((FlatSmilePricer::new(fitted), report))
    }
}

/// Closed-form generalized-Black-Scholes reference value used by the tests, kept
/// next to the model so the example is self-checking. Returns the call price over
/// the carry vocabulary (discount rate `r`, net carry `b`).
#[must_use]
#[doc(hidden)]
pub fn reference_call(inputs: &CarryInputs) -> f64 {
    let r = inputs.carry.discount_rate();
    let b = inputs.carry.carry_rate();
    let sqt = sqrt(inputs.t);
    let vsqt = inputs.vol * sqt;
    let d1 =
        (ln(inputs.spot / inputs.strike) + (b + 0.5 * inputs.vol * inputs.vol) * inputs.t) / vsqt;
    let d2 = d1 - vsqt;
    inputs.spot * exp((b - r) * inputs.t) * norm_cdf(d1)
        - inputs.strike * exp(-r * inputs.t) * norm_cdf(d2)
}

// --- Linear fixed-income seam ------------------------------------------------

/// The stable identity advertised by the reference constant-quote rates model.
pub const CONSTANT_RATES_ID: ModelId = ModelId("celnet.reference.constant-rates");

/// A deterministic reference rates model that returns a fixed, caller-configured
/// [`RatesMeasures`] regardless of the curve — the minimal [`RatesPricingModel`],
/// modelling a desk that publishes a **fixed manual quote** for a product.
///
/// It is a real, deterministic model (not a placeholder): its output is a pure
/// function of the [`RatesMeasures`] it was constructed with. It exists to
/// exercise — and prove — the fixed-income registry + dispatch seam end-to-end,
/// exactly as [`FlatSmilePricer`] anchors the option seam. A desk's own model
/// legitimately computes its measures from the [`RatesTerms`] and curve; this one
/// deliberately does not, so a test can assert the dispatch routed *through* it
/// by recognising its configured sentinel result.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstantRatesModel {
    /// The fixed measures this model quotes for every request.
    measures: RatesMeasures,
}

impl ConstantRatesModel {
    /// Construct a constant-quote rates model returning `measures`.
    #[must_use]
    pub fn new(measures: RatesMeasures) -> Self {
        Self { measures }
    }
}

impl RatesPricingModel for ConstantRatesModel {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(
            CONSTANT_RATES_ID,
            ModelKind::RatesPricing,
            GreekSupport::PRICE_ONLY,
        )
    }

    fn price(
        &self,
        _terms: &RatesTerms,
        _curve: &[RatesCurvePillar],
    ) -> PluginResult<RatesMeasures> {
        Ok(self.measures.clone())
    }
}
