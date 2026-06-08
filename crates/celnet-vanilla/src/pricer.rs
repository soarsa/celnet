//! The FX (Garman-Kohlhagen) implementation of the generalized [`CarryPricer`]
//! seam.
//!
//! This adapts the existing closed-form FX pricer ([`crate::price`]/
//! [`crate::greeks`]) to the asset-class-agnostic [`CarryPricer`] trait without
//! changing one line of the Garman-Kohlhagen arithmetic. It lowers an FX
//! [`CarryInputs`] to [`VanillaInputs`], calls the unchanged FX leaf, and lifts the
//! result back into the generalized strip — tagging the rate sensitivities as
//! [`celnet_types::RateSensitivities::Fx`]. The FX projection is byte-identical to
//! the direct `price`/`greeks` (proved over the full QuantLib golden grid by
//! `fx_carry_pricer_is_bit_identical_to_direct` in this module's tests).
//!
//! A non-FX [`celnet_types::Underlying`] or a non-`FxRates`
//! [`celnet_types::Carry`] is rejected with a typed [`CarryPriceError`]: the FX
//! leaf prices only FX, and never silently mis-prices another asset class under FX
//! arithmetic.

use celnet_core::{
    CarryGreeks, CarryInputs, CarryPriceError, CarryPricer, fx_carry_greeks, fx_vanilla_inputs,
};
use celnet_types::OptionType;

/// The FX Garman-Kohlhagen pricing leaf as a [`CarryPricer`].
///
/// A zero-sized handle: it carries no state, only the FX leaf's behaviour behind
/// the generalized seam, so a registry can hold it as `dyn CarryPricer`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FxPricer;

impl CarryPricer for FxPricer {
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> Result<f64, CarryPriceError> {
        let vi = fx_vanilla_inputs(inputs)?;
        Ok(crate::price(opt, &vi))
    }

    fn price_greeks(
        &self,
        opt: OptionType,
        inputs: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError> {
        let vi = fx_vanilla_inputs(inputs)?;
        Ok(fx_carry_greeks(&crate::greeks(opt, &vi)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{Carry, CcyPair, RateSensitivities, Underlying, VanillaInputs};
    use std::path::PathBuf;

    fn eurusd() -> Underlying {
        Underlying::Fx(CcyPair::parse("EURUSD").unwrap())
    }

    /// Path to the frozen QuantLib golden vanilla grid. `celnet-golden` dev-deps
    /// `celnet-vanilla`, so we cannot dev-dep it back (cycle); instead we read its
    /// committed CSV directly by a manifest-relative path. The file is the same
    /// grid the golden gate uses, so this exercises the full spot × moneyness × vol
    /// × maturity × rate-pair × call/put space.
    fn golden_csv() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../celnet-golden/data/vanilla_gk.csv")
    }

    /// The generalized `CarryPricer` path (FX leaf) returns a price and all 14
    /// Greeks (including the two rhos, surfaced via `RateSensitivities::Fx`) that
    /// are **bit-identical** (`to_bits`) to the direct `crate::price`/`crate::greeks`
    /// over the entire frozen QuantLib golden grid. This is the W1 no-regression
    /// invariant: the generalization is purely at the type level; the FX numbers do
    /// not move by a single ULP.
    #[test]
    fn fx_carry_pricer_is_bit_identical_to_direct() {
        let text =
            std::fs::read_to_string(golden_csv()).expect("read frozen golden vanilla_gk.csv");
        let pricer = FxPricer;
        let mut rows = 0usize;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("option_type") {
                continue;
            }
            let f: Vec<&str> = line.split(',').collect();
            // option_type,spot,strike,vol,t,r_dom,r_for,price,...
            assert!(f.len() >= 7, "malformed golden row: {line}");
            let opt = match f[0] {
                "CALL" => OptionType::Call,
                "PUT" => OptionType::Put,
                other => panic!("unexpected option_type {other}"),
            };
            let parse = |i: usize| f[i].parse::<f64>().expect("parse f64");
            let (spot, strike, vol, t, r_dom, r_for) =
                (parse(1), parse(2), parse(3), parse(4), parse(5), parse(6));

            let vi = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
            let ci = CarryInputs::new(
                spot,
                strike,
                vol,
                t,
                eurusd(),
                Carry::FxRates { r_dom, r_for },
            );

            // Price.
            let direct_price = crate::price(opt, &vi);
            let trait_price = pricer.price(opt, &ci).unwrap();
            assert_eq!(
                trait_price.to_bits(),
                direct_price.to_bits(),
                "price not bit-identical: {opt:?} {vi:?}"
            );

            // Full 14-Greek strip.
            let direct = crate::greeks(opt, &vi);
            let cg = pricer.price_greeks(opt, &ci).unwrap();
            assert_eq!(cg.price.to_bits(), direct.price.to_bits(), "price");
            assert_eq!(
                cg.delta_spot.to_bits(),
                direct.delta_spot.to_bits(),
                "delta_spot"
            );
            assert_eq!(
                cg.delta_forward.to_bits(),
                direct.delta_forward.to_bits(),
                "delta_forward"
            );
            assert_eq!(cg.gamma.to_bits(), direct.gamma.to_bits(), "gamma");
            assert_eq!(cg.vega.to_bits(), direct.vega.to_bits(), "vega");
            assert_eq!(cg.theta.to_bits(), direct.theta.to_bits(), "theta");
            assert_eq!(cg.vanna.to_bits(), direct.vanna.to_bits(), "vanna");
            assert_eq!(cg.volga.to_bits(), direct.volga.to_bits(), "volga");
            assert_eq!(cg.charm.to_bits(), direct.charm.to_bits(), "charm");
            assert_eq!(cg.speed.to_bits(), direct.speed.to_bits(), "speed");
            assert_eq!(cg.zomma.to_bits(), direct.zomma.to_bits(), "zomma");
            assert_eq!(cg.color.to_bits(), direct.color.to_bits(), "color");
            match cg.rates {
                RateSensitivities::Fx { rho_dom, rho_for } => {
                    assert_eq!(rho_dom.to_bits(), direct.rho_dom.to_bits(), "rho_dom");
                    assert_eq!(rho_for.to_bits(), direct.rho_for.to_bits(), "rho_for");
                }
                RateSensitivities::Carry { .. } => {
                    panic!("FX leaf must report RateSensitivities::Fx")
                }
            }
            rows += 1;
        }
        assert!(rows > 100, "expected the full golden grid, got {rows} rows");
    }

    /// The FX leaf rejects a non-FX carry rather than silently mis-pricing it.
    #[test]
    fn fx_pricer_rejects_cost_of_carry() {
        let ci = CarryInputs::new(
            100.0,
            100.0,
            0.2,
            1.0,
            eurusd(),
            Carry::CostOfCarry { r: 0.04, b: 0.01 },
        );
        assert_eq!(
            FxPricer.price(OptionType::Call, &ci),
            Err(CarryPriceError::UnsupportedCarry)
        );
        assert_eq!(
            FxPricer.price_greeks(OptionType::Call, &ci),
            Err(CarryPriceError::UnsupportedCarry)
        );
    }
}
