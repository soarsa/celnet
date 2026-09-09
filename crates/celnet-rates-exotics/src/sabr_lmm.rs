//! Stochastic Volatility Libor/Forward Market Model (SABR-LMM).
#![deny(missing_docs)]

use crate::ExoticsError;

/// SABR volatility parameters for a specific forward rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SabrParams {
    /// Initial volatility alpha > 0.
    pub alpha: f64,
    /// Elasticity beta in [0.0, 1.0] (typically 0.5 for rates, 1.0 for lognormal).
    pub beta: f64,
    /// Volatility of volatility nu >= 0.
    pub nu: f64,
    /// Correlation rho in [-1.0, 1.0] between rate and volatility shocks.
    pub rho: f64,
}

impl Default for SabrParams {
    fn default() -> Self {
        Self {
            alpha: 0.03, // 300 bps
            beta: 0.5,   // CIR / CEV square-root backbone
            nu: 0.40,    // 40% vol-of-vol
            rho: -0.25,  // negative correlation => downward skew
        }
    }
}

/// SABR analytical implied volatility pricer.
pub struct SabrModel;

impl SabrModel {
    /// Hagan (2002) analytical implied Black volatility formula.
    pub fn implied_volatility(
        forward: f64,
        strike: f64,
        expiry: f64,
        params: &SabrParams,
    ) -> Result<f64, ExoticsError> {
        if forward <= 0.0 || strike <= 0.0 || expiry <= 0.0 {
            return Err(ExoticsError::InvalidParameter("forward, strike, and expiry must be > 0".into()));
        }
        if params.alpha <= 0.0 {
            return Err(ExoticsError::InvalidParameter("alpha must be > 0".into()));
        }
        if params.nu < 0.0 {
            return Err(ExoticsError::InvalidParameter("nu must be >= 0".into()));
        }
        if !(0.0..=1.0).contains(&params.beta) {
            return Err(ExoticsError::InvalidParameter("beta must be in [0, 1]".into()));
        }
        if !(-1.0..=1.0).contains(&params.rho) {
            return Err(ExoticsError::InvalidParameter("rho must be in [-1, 1]".into()));
        }

        let f = forward;
        let k = strike;
        let t = expiry;
        let alpha = params.alpha;
        let beta = params.beta;
        let nu = params.nu;
        let rho = params.rho.clamp(-0.999999, 0.999999);

        // ATM case
        if (f - k).abs() < 1e-8 {
            let f_b = libm::pow(f, 1.0 - beta);
            let term1 = alpha / f_b;
            let term2 = 1.0 + (
                ((1.0 - beta) * (1.0 - beta) / 24.0) * (alpha * alpha / (f_b * f_b))
                + 0.25 * (rho * beta * nu * alpha / f_b)
                + ((2.0 - 3.0 * rho * rho) / 24.0) * (nu * nu)
            ) * t;
            return Ok(term1 * term2);
        }

        let fk_b = libm::pow(f * k, (1.0 - beta) / 2.0);
        let log_fk = libm::log(f / k);
        let z = (nu / alpha) * fk_b * log_fk;

        let z_over_xz = if z.abs() < 1e-5 {
            // 2nd-order Taylor expansion of z / x(z) around z = 0:
            // 1 + 0.5 * rho * z + (3 * rho^2 - 1) * z^2 / 12
            1.0 + 0.5 * rho * z + (3.0 * rho * rho - 1.0) * z * z / 12.0
        } else {
            // x(z) = log((sqrt(1 - 2*rho*z + z^2) + z - rho) / (1 - rho))
            let disc = (1.0 - 2.0 * rho * z + z * z).max(1e-12);
            let arg = (libm::sqrt(disc) + z - rho) / (1.0 - rho);
            let x_z = libm::log(arg.max(1e-12));
            if x_z.abs() < 1e-12 {
                1.0
            } else {
                z / x_z
            }
        };

        let denom = fk_b * (
            1.0
            + ((1.0 - beta) * (1.0 - beta) / 24.0) * (log_fk * log_fk)
            + (libm::pow(1.0 - beta, 4.0) / 1920.0) * libm::pow(log_fk, 4.0)
        );

        let factor = 1.0 + (
            ((1.0 - beta) * (1.0 - beta) / 24.0) * (alpha * alpha / (fk_b * fk_b))
            + 0.25 * (rho * beta * nu * alpha / fk_b)
            + ((2.0 - 3.0 * rho * rho) / 24.0) * (nu * nu)
        ) * t;

        let vol = (alpha / denom) * z_over_xz * factor;
        Ok(vol.max(1e-6))
    }

    /// Shifted SABR analytical implied Black-76 volatility formula.
    ///
    /// Extends standard Hagan SABR to negative and zero rates by shifting:
    /// F' = F + shift > 0,  K' = K + shift > 0.
    pub fn implied_volatility_shifted(
        forward: f64,
        strike: f64,
        expiry: f64,
        shift: f64,
        params: &SabrParams,
    ) -> Result<f64, ExoticsError> {
        let f_shifted = forward + shift;
        let k_shifted = strike + shift;
        if f_shifted <= 0.0 || k_shifted <= 0.0 {
            return Err(ExoticsError::InvalidParameter(
                format!("forward ({forward}) + shift ({shift}) and strike ({strike}) + shift ({shift}) must be > 0")
            ));
        }
        Self::implied_volatility(f_shifted, k_shifted, expiry, params)
    }

    /// Normal (Bachelier) implied volatility formula for SABR (Hagan 2002 / Ballotta & Bonfiglioli 2016).
    ///
    /// Computes normal volatility sigma_N (quoted in basis points or rate units)
    /// suitable for negative and near-zero rates without requiring positive forward bounds when beta = 0,
    /// or using shifted coordinates for beta > 0.
    pub fn normal_implied_volatility(
        forward: f64,
        strike: f64,
        expiry: f64,
        params: &SabrParams,
    ) -> Result<f64, ExoticsError> {
        if expiry <= 0.0 {
            return Err(ExoticsError::InvalidParameter("expiry must be > 0".into()));
        }
        if params.alpha <= 0.0 {
            return Err(ExoticsError::InvalidParameter("alpha must be > 0".into()));
        }
        if params.nu < 0.0 {
            return Err(ExoticsError::InvalidParameter("nu must be >= 0".into()));
        }
        if !(0.0..=1.0).contains(&params.beta) {
            return Err(ExoticsError::InvalidParameter("beta must be in [0, 1]".into()));
        }
        if !(-1.0..=1.0).contains(&params.rho) {
            return Err(ExoticsError::InvalidParameter("rho must be in [-1, 1]".into()));
        }

        let f = forward;
        let k = strike;
        let t = expiry;
        let alpha = params.alpha;
        let beta = params.beta;
        let nu = params.nu;
        let rho = params.rho.clamp(-0.999999, 0.999999);

        let delta = f - k;

        // Normal SABR with beta = 0 (exact Bachelier backbone)
        if beta.abs() < 1e-6 {
            if delta.abs() < 1e-8 {
                let factor = 1.0 + ((2.0 - 3.0 * rho * rho) / 24.0) * (nu * nu) * t;
                return Ok((alpha * factor).max(1e-7));
            }

            let z = (nu / alpha) * delta;
            let z_over_xz = if z.abs() < 1e-5 {
                1.0 + 0.5 * rho * z + (3.0 * rho * rho - 1.0) * z * z / 12.0
            } else {
                let disc = (1.0 - 2.0 * rho * z + z * z).max(1e-12);
                let arg = (libm::sqrt(disc) + z - rho) / (1.0 - rho);
                let x_z = libm::log(arg.max(1e-12));
                if x_z.abs() < 1e-12 {
                    1.0
                } else {
                    z / x_z
                }
            };

            let factor = 1.0 + ((2.0 - 3.0 * rho * rho) / 24.0) * (nu * nu) * t;
            let normal_vol = alpha * z_over_xz * factor;
            return Ok(normal_vol.max(1e-7));
        }

        // For beta > 0, evaluate general normal expansion
        if f <= 0.0 || k <= 0.0 {
            return Err(ExoticsError::InvalidParameter(
                "forward and strike must be > 0 for normal SABR with beta > 0 (use implied_volatility_shifted for negative rates)".into()
            ));
        }

        if delta.abs() < 1e-8 {
            let f_b = libm::pow(f, beta);
            let factor = 1.0 + (
                - (beta * (2.0 - beta) / 24.0) * (alpha * alpha / libm::pow(f, 2.0 * (1.0 - beta)))
                + 0.25 * (rho * beta * nu * alpha / libm::pow(f, 1.0 - beta))
                + ((2.0 - 3.0 * rho * rho) / 24.0) * (nu * nu)
            ) * t;
            return Ok((alpha * f_b * factor).max(1e-7));
        }

        let fk_mid = 0.5 * (f + k);
        let fk_b = libm::pow(fk_mid, beta);
        let log_fk = libm::log(f / k);
        let z = (nu / alpha) * libm::pow(fk_mid, 1.0 - beta) * log_fk;

        let z_over_xz = if z.abs() < 1e-5 {
            1.0 + 0.5 * rho * z + (3.0 * rho * rho - 1.0) * z * z / 12.0
        } else {
            let disc = (1.0 - 2.0 * rho * z + z * z).max(1e-12);
            let arg = (libm::sqrt(disc) + z - rho) / (1.0 - rho);
            let x_z = libm::log(arg.max(1e-12));
            if x_z.abs() < 1e-12 {
                1.0
            } else {
                z / x_z
            }
        };

        let factor = 1.0 + (
            - (beta * (2.0 - beta) / 24.0) * (alpha * alpha / libm::pow(fk_mid, 2.0 * (1.0 - beta)))
            + 0.25 * (rho * beta * nu * alpha / libm::pow(fk_mid, 1.0 - beta))
            + ((2.0 - 3.0 * rho * rho) / 24.0) * (nu * nu)
        ) * t;

        let normal_vol = alpha * fk_b * z_over_xz * factor;
        Ok(normal_vol.max(1e-7))
    }

    /// Cross-tenor forward rate exponential correlation decay:
    /// rho(i, j) = exp(-decay * |T_i - T_j|)
    pub fn cross_tenor_correlation(tenor_i: f64, tenor_j: f64, decay_param: f64) -> f64 {
        libm::exp(-decay_param * (tenor_i - tenor_j).abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sabr_shifted_negative_rates() {
        let params = SabrParams {
            alpha: 0.02,
            beta: 0.5,
            nu: 0.3,
            rho: -0.2,
        };
        // Negative forward and strike: -50 bps and -20 bps
        let f_neg = -0.0050;
        let k_neg = -0.0020;
        let shift = 0.0200; // 200 bps shift ensures positive shifted state
        let expiry = 1.0;

        let vol = SabrModel::implied_volatility_shifted(f_neg, k_neg, expiry, shift, &params).unwrap();
        assert!(vol > 0.0 && vol.is_finite());

        // Shift that does not make rates positive must error
        let err = SabrModel::implied_volatility_shifted(f_neg, k_neg, expiry, 0.0010, &params);
        assert!(err.is_err());
    }

    #[test]
    fn test_sabr_normal_bachelier_negative_and_zero_rates() {
        // Pure normal SABR (beta = 0)
        let params = SabrParams {
            alpha: 0.0060, // 60 bps normal vol
            beta: 0.0,
            nu: 0.25,
            rho: -0.15,
        };

        // Negative forward, zero strike
        let vol_atm = SabrModel::normal_implied_volatility(0.0, 0.0, 1.0, &params).unwrap();
        assert!(vol_atm > 0.0 && (vol_atm - 0.0060).abs() < 0.0010);

        let vol_neg = SabrModel::normal_implied_volatility(-0.0050, -0.0020, 1.0, &params).unwrap();
        assert!(vol_neg > 0.0 && vol_neg.is_finite());
    }
}
