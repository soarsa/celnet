//! A complete reference plugin built only on `celnet-types` + `celnet-core`.
//!
//! [`FlatSmilePricer`] is a *fully implemented* (not a mock) SDK model that ties
//! the three trait seams together: it wraps a [`celnet_core::FlatSmile`], so it
//! is a [`crate::SmileModel`], and it prices vanilla FX options off that constant
//! vol via the Garman-Kohlhagen forward form, so it is also a
//! [`crate::PricingModel`]. A matching [`FlatSmileCalibration`] fits the single
//! flat-vol parameter to a weighted set of vol quotes in closed form. Together
//! they demonstrate — and test — that the SDK contract is sufficient to author a
//! real model with no dependency beyond the two frozen interface crates.
//!
//! This deliberately reproduces the Black/Garman-Kohlhagen math from
//! `celnet_core::math` rather than depending on `celnet-vanilla`, keeping this
//! crate dependency-light per the SDK layering rules.

use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_core::{FlatSmile, Smile};
use celnet_types::{Greeks, OptionType, VanillaInputs, Vol};

use crate::calibration::{Calibration, CalibrationReport, CalibrationTarget};
use crate::descriptor::{GreekSupport, ModelDescriptor, ModelId, ModelKind};
use crate::error::{PluginError, PluginResult};
use crate::pricing::PricingModel;
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
    /// compares false to everything).
    fn validate(inputs: &VanillaInputs) -> PluginResult<()> {
        if !inputs.spot.is_finite() || inputs.spot <= 0.0 {
            return Err(PluginError::InvalidInput("spot must be positive"));
        }
        if !inputs.strike.is_finite() || inputs.strike <= 0.0 {
            return Err(PluginError::InvalidInput("strike must be positive"));
        }
        if !inputs.t.is_finite() || inputs.t <= 0.0 {
            return Err(PluginError::InvalidInput("time must be positive"));
        }
        if !inputs.r_dom.is_finite() || !inputs.r_for.is_finite() {
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
    fn aux(inputs: &VanillaInputs) -> Aux {
        let sqt = sqrt(inputs.t);
        let vsqt = inputs.vol * sqt;
        let d1 = (ln(inputs.spot / inputs.strike)
            + (inputs.r_dom - inputs.r_for + 0.5 * inputs.vol * inputs.vol) * inputs.t)
            / vsqt;
        let d2 = d1 - vsqt;
        Aux { d1, d2, sqt, vsqt }
    }
}

impl PricingModel for FlatSmilePricer {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(FLAT_PRICER_ID, ModelKind::Pricing, GreekSupport::FULL)
    }

    fn price(&self, opt: OptionType, inputs: &VanillaInputs) -> PluginResult<f64> {
        Self::validate(inputs)?;
        let a = Self::aux(inputs);
        let s_disc = inputs.spot * inputs.df_for();
        let k_disc = inputs.strike * inputs.df_dom();
        Ok(match opt {
            OptionType::Call => s_disc * norm_cdf(a.d1) - k_disc * norm_cdf(a.d2),
            OptionType::Put => k_disc * norm_cdf(-a.d2) - s_disc * norm_cdf(-a.d1),
        })
    }

    #[allow(clippy::similar_names)] // d1/d2, nd1/nd2 are the canonical option-pricing names
    fn price_and_greeks(&self, opt: OptionType, inputs: &VanillaInputs) -> PluginResult<Greeks> {
        Self::validate(inputs)?;
        let a = Self::aux(inputs);
        let (d1, d2, sqt, vsqt) = (a.d1, a.d2, a.sqt, a.vsqt);
        let (s, k, t, vol) = (inputs.spot, inputs.strike, inputs.t, inputs.vol);
        let df_dom = inputs.df_dom();
        let df_for = inputs.df_for();

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

        // Theta = ∂V/∂t (per year). Common term + carry terms differ by type.
        let theta_common = -s_disc * pd1 * vol / (2.0 * sqt);
        let theta = match opt {
            OptionType::Call => {
                theta_common + inputs.r_for * s_disc * nd1 - inputs.r_dom * k_disc * nd2
            }
            OptionType::Put => {
                theta_common - inputs.r_for * s_disc * nmd1 + inputs.r_dom * k_disc * nmd2
            }
        };

        // Charm = ∂(delta_spot)/∂T and Color = ∂(gamma)/∂T. We use the same
        // closed forms that are finite-difference-validated in `celnet-vanilla`:
        // with carry b = r_dom − r_for, ∂d1/∂T = b/(σ√T) − d1/(2T) + σ/(2√T).
        let b = inputs.r_dom - inputs.r_for;
        let dd1_dt = b / vsqt - d1 / (2.0 * t) + 0.5 * vol / sqt;
        let charm = match opt {
            OptionType::Call => -inputs.r_for * df_for * nd1 + df_for * pd1 * dd1_dt,
            OptionType::Put => inputs.r_for * df_for * nmd1 + df_for * pd1 * dd1_dt,
        };
        let color = gamma * (-inputs.r_for - 1.0 / (2.0 * t) - d1 * dd1_dt);

        // Rhos (per 1.0 of continuously-compounded rate).
        let rho_dom = match opt {
            OptionType::Call => k * t * df_dom * nd2,
            OptionType::Put => -k * t * df_dom * nmd2,
        };
        let rho_for = match opt {
            OptionType::Call => -s * t * df_for * nd1,
            OptionType::Put => s * t * df_for * nmd1,
        };

        Ok(Greeks {
            price,
            delta_spot,
            delta_forward,
            gamma,
            vega,
            theta,
            rho_dom,
            rho_for,
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

/// Closed-form Black/Garman-Kohlhagen reference value used by the tests, kept
/// next to the model so the example is self-checking. Returns the call price.
#[must_use]
#[doc(hidden)]
pub fn reference_call(inputs: &VanillaInputs) -> f64 {
    let sqt = sqrt(inputs.t);
    let vsqt = inputs.vol * sqt;
    let d1 = (ln(inputs.spot / inputs.strike)
        + (inputs.r_dom - inputs.r_for + 0.5 * inputs.vol * inputs.vol) * inputs.t)
        / vsqt;
    let d2 = d1 - vsqt;
    inputs.spot * exp(-inputs.r_for * inputs.t) * norm_cdf(d1)
        - inputs.strike * exp(-inputs.r_dom * inputs.t) * norm_cdf(d2)
}
