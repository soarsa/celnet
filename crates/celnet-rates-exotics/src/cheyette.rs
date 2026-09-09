//! Cheyette 1-Factor and 2-Factor Markov-Functional Interest Rate Engine.
#![deny(missing_docs)]

use crate::ExoticsError;

/// Parameters for a Cheyette 1-Factor model:
/// sigma(t, T) = sigma * exp(-kappa * (T - t))
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cheyette1FParams {
    /// Mean reversion speed kappa > 0.
    pub mean_reversion_kappa: f64,
    /// Instantaneous volatility sigma > 0.
    pub volatility_sigma: f64,
}

impl Default for Cheyette1FParams {
    fn default() -> Self {
        Self {
            mean_reversion_kappa: 0.03, // 3% mean reversion
            volatility_sigma: 0.012,    // 120 bps normal vol
        }
    }
}

/// Parameters for a Cheyette 2-Factor model with correlation rho:
/// sigma_1(t, T) = sigma_1 * exp(-kappa_1 * (T - t))
/// sigma_2(t, T) = sigma_2 * exp(-kappa_2 * (T - t))
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cheyette2FParams {
    /// Mean reversion speed for factor 1 (e.g. 0.02 for curve level).
    pub kappa_1: f64,
    /// Volatility for factor 1.
    pub sigma_1: f64,
    /// Mean reversion speed for factor 2 (e.g. 0.25 for curve slope).
    pub kappa_2: f64,
    /// Volatility for factor 2.
    pub sigma_2: f64,
    /// Correlation rho in [-1.0, 1.0] between factors.
    pub correlation_rho: f64,
}

impl Default for Cheyette2FParams {
    fn default() -> Self {
        Self {
            kappa_1: 0.02,
            sigma_1: 0.010,
            kappa_2: 0.20,
            sigma_2: 0.008,
            correlation_rho: -0.65, // Typical level-slope negative correlation
        }
    }
}

/// European Swaption specification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwaptionSpec {
    /// Option expiry / swap start time in years (e.g. 1.0 for 1Y).
    pub expiry_years: f64,
    /// Underlying swap tenor in years (e.g. 5.0 for 5Y swap).
    pub swap_tenor_years: f64,
    /// Fixed strike rate (e.g. 0.035 for 3.5%).
    pub strike_rate: f64,
    /// Notional.
    pub notional: f64,
    /// Pay fixed (Payer swaption) or receive fixed (Receiver swaption).
    pub is_payer: bool,
}

/// Cheyette Swaption pricer.
pub struct CheyettePricer;

impl CheyettePricer {
    /// Analytical pricing of European Swaption under Cheyette 1-Factor model.
    ///
    /// Uses Jamshidian decomposition on the deterministic zero-coupon bond volatility:
    /// sigma_P(t, T) = sigma / kappa * (1 - exp(-kappa * (T - t)))
    pub fn price_swaption_1f(
        spec: &SwaptionSpec,
        params: &Cheyette1FParams,
        forward_swap_rate: f64,
        annuity: f64,
    ) -> Result<f64, ExoticsError> {
        if params.mean_reversion_kappa <= 0.0 || params.volatility_sigma <= 0.0 {
            return Err(ExoticsError::InvalidParameter("kappa and sigma must be > 0".into()));
        }
        if spec.expiry_years <= 0.0 || spec.swap_tenor_years <= 0.0 {
            return Err(ExoticsError::InvalidParameter("tenors must be > 0".into()));
        }

        let t = spec.expiry_years;
        let k = params.mean_reversion_kappa;
        let sigma = params.volatility_sigma;

        // Integrated effective swap rate volatility:
        // sigma_S = sigma * [1 - exp(-k * tenor)] / (k * tenor) * sqrt([1 - exp(-2*k*t)] / (2*k*t))
        let factor_tenor = (1.0 - libm::exp(-k * spec.swap_tenor_years)) / (k * spec.swap_tenor_years);
        let factor_time = libm::sqrt((1.0 - libm::exp(-2.0 * k * t)) / (2.0 * k * t));
        let effective_vol = sigma * factor_tenor * factor_time;

        // Black-76 / Bachelier style normal swaption valuation
        let std_dev = effective_vol * libm::sqrt(t);
        let d = (forward_swap_rate - spec.strike_rate) / std_dev;

        let n_d = 0.5 * (1.0 + libm::erf(d / libm::sqrt(2.0)));
        let n_prime_d = libm::exp(-0.5 * d * d) / libm::sqrt(2.0 * std::f64::consts::PI);

        let unit_pv = if spec.is_payer {
            (forward_swap_rate - spec.strike_rate) * n_d + std_dev * n_prime_d
        } else {
            let neg_d = -d;
            let n_neg_d = 0.5 * (1.0 + libm::erf(neg_d / libm::sqrt(2.0)));
            (spec.strike_rate - forward_swap_rate) * n_neg_d + std_dev * n_prime_d
        };

        let pv = spec.notional * annuity * unit_pv;
        Ok(pv)
    }

    /// Price European Swaption under Cheyette 2-Factor model.
    pub fn price_swaption_2f(
        spec: &SwaptionSpec,
        params: &Cheyette2FParams,
        forward_swap_rate: f64,
        annuity: f64,
    ) -> Result<f64, ExoticsError> {
        if params.kappa_1 <= 0.0 || params.kappa_2 <= 0.0 || params.sigma_1 <= 0.0 || params.sigma_2 <= 0.0 {
            return Err(ExoticsError::InvalidParameter("kappa and sigma parameters must be > 0".into()));
        }
        if !(-1.0..=1.0).contains(&params.correlation_rho) {
            return Err(ExoticsError::InvalidParameter("correlation rho must be in [-1, 1]".into()));
        }
        if spec.expiry_years <= 0.0 || spec.swap_tenor_years <= 0.0 {
            return Err(ExoticsError::InvalidParameter("tenors must be > 0".into()));
        }

        let t = spec.expiry_years;
        let tenor = spec.swap_tenor_years;

        let g1 = (1.0 - libm::exp(-params.kappa_1 * tenor)) / (params.kappa_1 * tenor);
        let g2 = (1.0 - libm::exp(-params.kappa_2 * tenor)) / (params.kappa_2 * tenor);

        let var1 = params.sigma_1 * params.sigma_1 * (1.0 - libm::exp(-2.0 * params.kappa_1 * t)) / (2.0 * params.kappa_1);
        let var2 = params.sigma_2 * params.sigma_2 * (1.0 - libm::exp(-2.0 * params.kappa_2 * t)) / (2.0 * params.kappa_2);
        let cov12 = 2.0 * params.correlation_rho * params.sigma_1 * params.sigma_2
            * (1.0 - libm::exp(-(params.kappa_1 + params.kappa_2) * t)) / (params.kappa_1 + params.kappa_2);

        let total_variance = g1 * g1 * var1 + g2 * g2 * var2 + g1 * g2 * cov12;
        let std_dev = libm::sqrt(total_variance.max(1.0e-12));

        let d = (forward_swap_rate - spec.strike_rate) / std_dev;
        let n_d = 0.5 * (1.0 + libm::erf(d / libm::sqrt(2.0)));
        let n_prime_d = libm::exp(-0.5 * d * d) / libm::sqrt(2.0 * std::f64::consts::PI);

        let unit_pv = if spec.is_payer {
            (forward_swap_rate - spec.strike_rate) * n_d + std_dev * n_prime_d
        } else {
            let neg_d = -d;
            let n_neg_d = 0.5 * (1.0 + libm::erf(neg_d / libm::sqrt(2.0)));
            (spec.strike_rate - forward_swap_rate) * n_neg_d + std_dev * n_prime_d
        };

        Ok(spec.notional * annuity * unit_pv)
    }
}
