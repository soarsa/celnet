//! `celnet-rates-exotics` — Nonlinear Rates Exotics and Multi-Name Credit Copula Engine.
//!
//! Provides institutional pricing for exotic interest rate derivatives and credit portfolios:
//!
//! 1. **Cheyette 1F & 2F Rates Engine (`cheyette`)**: Separable volatility HJM Markov-functional
//!    model for European and Bermudan swaptions and cancellable swaps.
//! 2. **SABR Forward Market Model (`sabr_lmm`)**: Multi-tenor forward rate evolution with
//!    stochastic volatility smiles and cross-tenor correlation.
//! 3. **Credit Portfolio Copula Engine (`credit_copula`)**: Factor Gaussian and Student-t
//!    copulas with Andersen-Sidenius-Basu recursive loss distribution for Synthetic CDO Tranches
//!    (Equity, Mezzanine, Senior) and First-to-Default (FtD) baskets.
//!
//! Strict `#![forbid(unsafe_code)]` with analytical reference validation.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod cheyette;
pub mod credit_copula;
pub mod sabr_lmm;
pub mod signature_vol;

use thiserror::Error;

/// Error conditions in rates exotics and credit copula pricing.
#[derive(Debug, Error, PartialEq, Clone)]
pub enum ExoticsError {
    /// Mathematical invalid argument (e.g. mean reversion <= 0, vol <= 0).
    #[error("invalid model parameter: {0}")]
    InvalidParameter(String),
    /// Copula integration convergence error.
    #[error("numerical solver did not converge: {0}")]
    ConvergenceFailure(String),
    /// Dimension mismatch in portfolio obligors or correlation matrix.
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected dimension.
        expected: usize,
        /// Actual dimension.
        actual: usize,
    },
}
