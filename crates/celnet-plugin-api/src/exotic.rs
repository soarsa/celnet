//! Generalized multi-asset and exotic pricing model contract.
//!
//! Extends Celnet's plugin API beyond single-asset European vanilla options to
//! multi-underlying derivatives (Baskets, Quantos, Rainbows) and path-dependent
//! payoffs (Asians, Barriers, Cliquets, Accumulators).

use celnet_types::{Ccy, Underlying};

use crate::descriptor::ModelDescriptor;
use crate::error::{PluginError, PluginResult};

/// Generalized multi-asset and path-dependent market inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiAssetInputs {
    /// List of underlying instruments in the basket / structure.
    pub underlyings: Vec<Underlying>,
    /// Current spot prices matching `underlyings`.
    pub spots: Vec<f64>,
    /// Volatilities matching `underlyings`.
    pub vols: Vec<f64>,
    /// Flattened correlation matrix ($N \times N$, row-major order).
    pub correlation_matrix: Vec<f64>,
    /// Time to maturity in fractional years.
    pub expiry_years: f64,
    /// Observation fixing dates in fractional years (e.g. for Asian averaging or Barrier monitoring).
    pub observation_schedule: Vec<f64>,
    /// Historical past fixings already observed (if trade is mid-lifecycle).
    pub past_fixings: Vec<f64>,
    /// Numeraire currency of the valuation.
    pub numeraire: Ccy,
}

impl MultiAssetInputs {
    /// Validate structural integrity and dimension consistency.
    ///
    /// # Errors
    /// Returns [`PluginError::InvalidInput`] if dimensions mismatch or numbers are invalid.
    pub fn validate(&self) -> PluginResult<()> {
        let n = self.underlyings.len();
        if n == 0 {
            return Err(PluginError::InvalidInput("underlyings vector cannot be empty"));
        }
        if self.spots.len() != n {
            return Err(PluginError::InvalidInput("spots count does not match underlyings count"));
        }
        if self.vols.len() != n {
            return Err(PluginError::InvalidInput("vols count does not match underlyings count"));
        }
        if self.correlation_matrix.len() != n * n {
            return Err(PluginError::InvalidInput("correlation_matrix size must be N x N"));
        }
        if self.expiry_years < 0.0 || !self.expiry_years.is_finite() {
            return Err(PluginError::InvalidInput("expiry_years must be non-negative and finite"));
        }
        for (i, &s) in self.spots.iter().enumerate() {
            if s <= 0.0 || !s.is_finite() {
                return Err(PluginError::InvalidInput("spot prices must be positive and finite"));
            }
            if self.vols[i] < 0.0 || !self.vols[i].is_finite() {
                return Err(PluginError::InvalidInput("volatilities must be non-negative and finite"));
            }
        }
        // Verify diagonal of correlation matrix is 1.0
        for i in 0..n {
            let diag = self.correlation_matrix[i * n + i];
            if !celnet_core::is_close(diag, 1.0, 1e-6, 1e-6) {
                return Err(PluginError::InvalidInput("correlation matrix diagonal elements must be 1.0"));
            }
        }
        Ok(())
    }
}

/// Archetype category for exotic derivatives.
#[derive(Debug, Clone, PartialEq)]
pub enum ExoticArchetype {
    /// Multi-underlying basket or rainbow option.
    Basket,
    /// Path-dependent Asian option with averaging.
    Asian,
    /// Single or double barrier option with discrete or continuous monitoring.
    Barrier,
    /// Periodic cliquet with local caps and floors.
    Cliquet,
    /// Accumulator / decumulator structured contract.
    Accumulator,
    /// Desk-proprietary custom structured payoff.
    Custom(String),
}

/// Description of an exotic or path-dependent payoff structure.
#[derive(Debug, Clone, PartialEq)]
pub struct ExoticPayoffDescriptor {
    /// Open exotic archetype classification.
    pub archetype: ExoticArchetype,
    /// Strike price level (or base strike for multi-asset).
    pub strike: f64,
    /// Barrier level or upper boundary (if applicable).
    pub upper_barrier: Option<f64>,
    /// Lower barrier level (if applicable).
    pub lower_barrier: Option<f64>,
    /// Rebate payment amount.
    pub rebate: f64,
    /// Payoff weights across underlyings (for basket / rainbow options; sums to 1.0).
    pub weights: Vec<f64>,
}

/// A model that prices exotic, path-dependent, and multi-asset derivatives.
pub trait ExoticPricingModel: Send + Sync {
    /// Self-description used by the registry to discover and route work to this model.
    fn descriptor(&self) -> ModelDescriptor;

    /// Present value in the numeraire currency.
    ///
    /// # Errors
    /// Returns [`PluginError::InvalidInput`] if inputs fail validation,
    /// [`PluginError::Unsupported`] if the model does not price the given exotic kind,
    /// or [`PluginError::DidNotConverge`] if numerical routines fail.
    fn price_exotic(
        &self,
        payoff: &ExoticPayoffDescriptor,
        inputs: &MultiAssetInputs,
    ) -> PluginResult<f64>;

    /// First-order delta sensitivity vector across each underlying asset.
    ///
    /// # Errors
    /// Returns [`PluginError::Unsupported`] if the model does not produce sensitivities.
    fn deltas(
        &self,
        payoff: &ExoticPayoffDescriptor,
        inputs: &MultiAssetInputs,
    ) -> PluginResult<Vec<f64>> {
        let _ = (payoff, inputs);
        Err(PluginError::Unsupported("model does not produce deltas"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::CcyPair;
    use crate::descriptor::{GreekSupport, ModelId, ModelKind};

    struct FlatBasketModel;
    impl ExoticPricingModel for FlatBasketModel {
        fn descriptor(&self) -> ModelDescriptor {
            ModelDescriptor::new(
                ModelId("house.flat_basket"),
                ModelKind::ExoticPricing,
                GreekSupport::FULL,
            )
        }

        fn price_exotic(
            &self,
            payoff: &ExoticPayoffDescriptor,
            inputs: &MultiAssetInputs,
        ) -> PluginResult<f64> {
            inputs.validate()?;
            let mut weighted_spot = 0.0;
            for (i, &w) in payoff.weights.iter().enumerate() {
                weighted_spot += w * inputs.spots[i];
            }
            let intrinsic = (weighted_spot - payoff.strike).max(0.0);
            Ok(intrinsic)
        }

        fn deltas(
            &self,
            payoff: &ExoticPayoffDescriptor,
            inputs: &MultiAssetInputs,
        ) -> PluginResult<Vec<f64>> {
            inputs.validate()?;
            Ok(payoff.weights.clone())
        }
    }

    #[test]
    fn test_multi_asset_inputs_validation() {
        let ccy_eur = Ccy::parse("EUR").unwrap();
        let u1 = Underlying::Fx(CcyPair::parse("EURUSD").unwrap());
        let u2 = Underlying::Fx(CcyPair::parse("GBPUSD").unwrap());

        let valid_inputs = MultiAssetInputs {
            underlyings: vec![u1.clone(), u2.clone()],
            spots: vec![1.10, 1.30],
            vols: vec![0.10, 0.12],
            correlation_matrix: vec![1.0, 0.65, 0.65, 1.0],
            expiry_years: 1.0,
            observation_schedule: vec![],
            past_fixings: vec![],
            numeraire: ccy_eur,
        };
        assert!(valid_inputs.validate().is_ok());

        let model = FlatBasketModel;
        assert_eq!(model.descriptor().kind, ModelKind::ExoticPricing);

        let payoff = ExoticPayoffDescriptor {
            archetype: ExoticArchetype::Basket,
            strike: 1.15,
            upper_barrier: None,
            lower_barrier: None,
            rebate: 0.0,
            weights: vec![0.5, 0.5],
        };

        let pv = model.price_exotic(&payoff, &valid_inputs).unwrap();
        // weighted_spot = 0.5 * 1.10 + 0.5 * 1.30 = 1.20. Intrinsic = 1.20 - 1.15 = 0.05
        assert!(celnet_core::is_close(pv, 0.05, 1e-9, 1e-9));

        let deltas = model.deltas(&payoff, &valid_inputs).unwrap();
        assert_eq!(deltas, vec![0.5, 0.5]);
    }
}
