//! Celnet user-extensibility SDK contract.
//!
//! This crate defines the **single, current** contract that both the native
//! trait registry (compiled-in first-party models, the hot path) and the Wasm
//! sandbox host (untrusted user models) implement, so first-party and user
//! plugins are *interchangeable* behind one registry (see
//! `docs/ARCHITECTURE.md` §6). It is intentionally dependency-light — it depends
//! only on the two frozen interface crates, [`celnet_types`] (the vocabulary and
//! DTOs) and [`celnet_core`] (the deterministic math and the [`celnet_core::Smile`]
//! seam) — and pulls in **no** runtime (no interpreter, no pricing-engine crate).
//! The host runtime weight lives entirely in `celnet-plugin-host`, which runs the
//! untrusted tier on the `wasmi` interpreter (the contract is runtime-agnostic).
//!
//! # The three model seams
//!
//! - [`PricingModel`] — price (and optionally Greeks) for a vanilla FX option
//!   given [`celnet_types::VanillaInputs`].
//! - [`SmileModel`] — a volatility smile/surface; extends [`celnet_core::Smile`]
//!   and adds an arbitrage self-check.
//! - [`Calibration`] — fits a model's parameters to market targets, producing a
//!   ready-to-use model.
//!
//! Each model advertises a [`ModelDescriptor`] so a [`ModelRegistry`] can route
//! work to it by purpose without knowing its concrete type or whether it is
//! native or Wasm. Errors crossing the boundary use the portable [`PluginError`].
//!
//! # Determinism obligations (contract, not convention)
//!
//! Implementors must be deterministic (identical inputs ⇒ bit-identical output),
//! must route transcendentals through `celnet_core::math` (`rust-lang/libm`), and
//! must never compare floats with `==` or assert on `NaN` (use
//! `celnet_core::is_close`). The plugin host's deterministic replay harness
//! asserts these for untrusted user models.
//!
//! # The WIT mirror
//!
//! `wit/celnet.wit` describes the identical contract as a runtime-agnostic
//! `world`. The Rust traits here and that WIT world are kept in lock-step: the
//! records/enums map one-to-one. The host lowers those flat POD shapes onto a
//! core-module `(ptr,len)` ABI for the `wasmi` interpreter (wasmi hosts core
//! modules, not the Component Model), so a guest authored against the world
//! exposes the same shape these traits do.

#![forbid(unsafe_code)]

mod calibration;
mod descriptor;
mod error;
pub mod example;
mod pricing;
mod registry;
mod smile;

pub use calibration::{Calibration, CalibrationReport, CalibrationTarget};
pub use descriptor::{GreekSupport, ModelDescriptor, ModelId, ModelKind};
pub use error::{PluginError, PluginResult};
pub use pricing::PricingModel;
pub use registry::ModelRegistry;
pub use smile::SmileModel;

#[cfg(test)]
mod tests {
    use celnet_core::{Smile, assert_close};
    use celnet_types::{OptionType, VanillaInputs};

    use crate::example::{
        FLAT_CALIBRATION_ID, FLAT_PRICER_ID, FlatSmileCalibration, FlatSmilePricer, reference_call,
    };
    use crate::{
        Calibration, CalibrationTarget, GreekSupport, ModelDescriptor, ModelId, ModelKind,
        ModelRegistry, PluginError, PricingModel, SmileModel,
    };

    // A textbook Black-Scholes benchmark: S=K=100, σ=20%, T=1, r_d=5%, r_f=0.
    // The call price is the canonical 10.4506 (matches `celnet-vanilla`).
    fn bench_inputs() -> VanillaInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.0)
    }

    #[test]
    fn pricing_model_matches_known_value() {
        let model = FlatSmilePricer::new(0.20);
        let i = bench_inputs();
        let call = model.price(OptionType::Call, &i).unwrap();
        // Known textbook benchmark value.
        assert_close!(call, 10.450_583_572_185_565, 1e-12, 1e-9);
        // And the model agrees with the closed-form reference.
        assert_close!(call, reference_call(&i), 1e-12, 1e-12);
    }

    #[test]
    fn pricing_model_put_call_parity() {
        let model = FlatSmilePricer::new(0.18);
        let i = bench_inputs();
        let c = model.price(OptionType::Call, &i).unwrap();
        let p = model.price(OptionType::Put, &i).unwrap();
        // C - P = S·e^{-r_f T} - K·e^{-r_d T}.
        let parity = i.spot * i.df_for() - i.strike * i.df_dom();
        assert_close!(c - p, parity, 1e-12, 1e-12);
    }

    #[test]
    fn greeks_price_field_matches_price() {
        let model = FlatSmilePricer::new(0.22);
        let i = bench_inputs();
        let g = model.price_and_greeks(OptionType::Call, &i).unwrap();
        let p = model.price(OptionType::Call, &i).unwrap();
        assert_close!(g.price, p, 1e-12, 1e-12);
        // Vega is positive; gamma is positive for a vanilla.
        assert!(g.vega > 0.0);
        assert!(g.gamma > 0.0);
    }

    #[test]
    fn greeks_delta_against_finite_difference() {
        let model = FlatSmilePricer::new(0.20);
        let i = bench_inputs();
        let g = model.price_and_greeks(OptionType::Call, &i).unwrap();
        let h = 1e-5 * i.spot;
        let up = VanillaInputs {
            spot: i.spot + h,
            ..i
        };
        let dn = VanillaInputs {
            spot: i.spot - h,
            ..i
        };
        let fd = (model.price(OptionType::Call, &up).unwrap()
            - model.price(OptionType::Call, &dn).unwrap())
            / (2.0 * h);
        assert_close!(g.delta_spot, fd, 1e-5, 1e-7);
    }

    #[test]
    fn greeks_vega_against_finite_difference() {
        let model = FlatSmilePricer::new(0.20);
        let i = bench_inputs();
        let g = model.price_and_greeks(OptionType::Call, &i).unwrap();
        let h = 1e-6;
        let up = VanillaInputs {
            vol: i.vol + h,
            ..i
        };
        let dn = VanillaInputs {
            vol: i.vol - h,
            ..i
        };
        let fd = (model.price(OptionType::Call, &up).unwrap()
            - model.price(OptionType::Call, &dn).unwrap())
            / (2.0 * h);
        assert_close!(g.vega, fd, 1e-5, 1e-7);
    }

    /// The example advertises `GreekSupport::FULL`, so EVERY Greek it produces is
    /// cross-checked against a central finite difference of its own price (or of
    /// the relevant first-order Greek, for the second-order ones), across several
    /// regimes. This is the gate that proves charm/color/vanna/volga/speed/zomma
    /// — not just delta and vega — are correct.
    #[test]
    #[allow(clippy::similar_names)]
    fn full_greek_set_against_finite_difference() {
        // Central difference of `f` between the up/down bumps of one input field.
        fn cd(
            f: impl Fn(&VanillaInputs) -> f64,
            up: &VanillaInputs,
            dn: &VanillaInputs,
            h: f64,
        ) -> f64 {
            (f(up) - f(dn)) / (2.0 * h)
        }
        let regimes = [
            VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.0),
            VanillaInputs::new(1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
            VanillaInputs::new(1.35, 1.20, 0.14, 2.0, 0.04, 0.015),
            VanillaInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.03),
        ];
        for i in regimes {
            // The model's flat vol equals the regime vol, so it reproduces the
            // Garman-Kohlhagen price and its sensitivities exactly.
            let model = FlatSmilePricer::new(i.vol);
            for opt in [OptionType::Call, OptionType::Put] {
                let g = model.price_and_greeks(opt, &i).unwrap();
                let price = |x: &VanillaInputs| model.price(opt, x).unwrap();
                let delta = |x: &VanillaInputs| model.price_and_greeks(opt, x).unwrap().delta_spot;
                let gamma_of = |x: &VanillaInputs| model.price_and_greeks(opt, x).unwrap().gamma;
                let vega_of = |x: &VanillaInputs| model.price_and_greeks(opt, x).unwrap().vega;

                let (hs, hv, ht, hr) = (1e-4 * i.spot, 1e-5, 1e-5, 1e-6);
                let s_up = VanillaInputs {
                    spot: i.spot + hs,
                    ..i
                };
                let s_dn = VanillaInputs {
                    spot: i.spot - hs,
                    ..i
                };
                let v_up = VanillaInputs {
                    vol: i.vol + hv,
                    ..i
                };
                let v_dn = VanillaInputs {
                    vol: i.vol - hv,
                    ..i
                };
                let t_up = VanillaInputs { t: i.t + ht, ..i };
                let t_dn = VanillaInputs { t: i.t - ht, ..i };
                let rd_up = VanillaInputs {
                    r_dom: i.r_dom + hr,
                    ..i
                };
                let rd_dn = VanillaInputs {
                    r_dom: i.r_dom - hr,
                    ..i
                };
                let rf_up = VanillaInputs {
                    r_for: i.r_for + hr,
                    ..i
                };
                let rf_dn = VanillaInputs {
                    r_for: i.r_for - hr,
                    ..i
                };

                // First order.
                assert_close!(g.delta_spot, cd(price, &s_up, &s_dn, hs), 1e-4, 1e-7);
                assert_close!(g.vega, cd(price, &v_up, &v_dn, hv), 1e-4, 1e-7);
                assert_close!(g.theta, -cd(price, &t_up, &t_dn, ht), 5e-4, 1e-6);
                assert_close!(g.rho_dom, cd(price, &rd_up, &rd_dn, hr), 1e-4, 1e-7);
                assert_close!(g.rho_for, cd(price, &rf_up, &rf_dn, hr), 1e-4, 1e-7);
                // Second order (difference the relevant first-order Greek).
                assert_close!(g.gamma, cd(delta, &s_up, &s_dn, hs), 1e-3, 1e-6);
                assert_close!(g.vanna, cd(delta, &v_up, &v_dn, hv), 1e-3, 1e-6);
                assert_close!(g.volga, cd(vega_of, &v_up, &v_dn, hv), 1e-3, 1e-6);
                assert_close!(g.charm, cd(delta, &t_up, &t_dn, ht), 1e-3, 1e-6);
                assert_close!(g.speed, cd(gamma_of, &s_up, &s_dn, hs), 1e-2, 1e-5);
                assert_close!(g.zomma, cd(gamma_of, &v_up, &v_dn, hv), 1e-2, 1e-5);
                assert_close!(g.color, cd(gamma_of, &t_up, &t_dn, ht), 1e-2, 1e-5);
            }
        }
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let model = FlatSmilePricer::new(0.20);
        let bad = VanillaInputs::new(-1.0, 100.0, 0.2, 1.0, 0.05, 0.0);
        assert_eq!(
            model.price(OptionType::Call, &bad),
            Err(PluginError::InvalidInput("spot must be positive"))
        );
    }

    #[test]
    fn smile_model_is_flat_and_arbitrage_free() {
        let model = FlatSmilePricer::new(0.15);
        // Smile supertrait: constant vol everywhere.
        assert_close!(model.implied_vol(80.0, 100.0, 0.5).0, 0.15);
        assert_close!(model.try_implied_vol(120.0, 100.0, 0.5).unwrap().0, 0.15);
        // A flat smile carries no static (butterfly) arbitrage.
        let strikes = [80.0, 90.0, 100.0, 110.0, 120.0];
        model.check_no_arbitrage(100.0, 0.5, &strikes).unwrap();
    }

    #[test]
    fn arbitrage_check_rejects_bad_grid() {
        let model = FlatSmilePricer::new(0.15);
        assert_eq!(
            model.check_no_arbitrage(100.0, 0.5, &[100.0, 90.0, 110.0]),
            Err(PluginError::InvalidInput(
                "strikes must be strictly increasing"
            ))
        );
        assert_eq!(
            model.check_no_arbitrage(-1.0, 0.5, &[80.0, 100.0, 120.0]),
            Err(PluginError::InvalidInput("forward and t must be positive"))
        );
    }

    #[test]
    fn calibration_recovers_weighted_mean() {
        let cal = FlatSmileCalibration;
        let targets = [
            CalibrationTarget::new(90.0, 0.18),
            CalibrationTarget::new(100.0, 0.20),
            CalibrationTarget::new(110.0, 0.22),
        ];
        let (model, report) = cal.calibrate(100.0, 1.0, &targets).unwrap();
        // Unit weights ⇒ the mean (0.20).
        assert_close!(model.smile.vol, 0.20, 1e-12, 1e-12);
        assert_eq!(report.iterations, 1);
        // The fitted model prices at the recovered vol.
        let i = bench_inputs();
        assert_close!(
            model.price(OptionType::Call, &i).unwrap(),
            reference_call(&i),
            1e-12,
            1e-12
        );
    }

    #[test]
    fn calibration_honours_weights() {
        let cal = FlatSmileCalibration;
        let targets = [
            CalibrationTarget {
                abscissa: 90.0,
                observed: 0.10,
                weight: 3.0,
            },
            CalibrationTarget {
                abscissa: 110.0,
                observed: 0.20,
                weight: 1.0,
            },
        ];
        let (model, _) = cal.calibrate(100.0, 1.0, &targets).unwrap();
        // Weighted mean: (3·0.10 + 1·0.20) / 4 = 0.125.
        assert_close!(model.smile.vol, 0.125, 1e-12, 1e-12);
    }

    #[test]
    fn calibration_rejects_empty_and_zero_weight() {
        let cal = FlatSmileCalibration;
        assert_eq!(
            cal.calibrate(100.0, 1.0, &[]),
            Err(PluginError::InvalidInput("no calibration targets"))
        );
        let zero = [CalibrationTarget {
            abscissa: 100.0,
            observed: 0.2,
            weight: 0.0,
        }];
        assert_eq!(
            cal.calibrate(100.0, 1.0, &zero),
            Err(PluginError::InvalidInput("weights sum to zero"))
        );
    }

    #[test]
    fn descriptors_describe_each_seam() {
        let model = FlatSmilePricer::new(0.2);
        let pd = PricingModel::descriptor(&model);
        assert_eq!(pd.id, FLAT_PRICER_ID);
        assert_eq!(pd.kind, ModelKind::Pricing);
        assert_eq!(pd.greeks, GreekSupport::FULL);

        let sd = SmileModel::descriptor(&model);
        assert_eq!(sd.kind, ModelKind::Smile);

        let cd = FlatSmileCalibration.descriptor();
        assert_eq!(cd.id, FLAT_CALIBRATION_ID);
        assert_eq!(cd.kind, ModelKind::Calibration);
    }

    /// A minimal static registry, exercising the [`ModelRegistry`] default
    /// methods that both the native and Wasm backends will rely on.
    struct StaticRegistry {
        items: Vec<ModelDescriptor>,
    }
    impl ModelRegistry for StaticRegistry {
        fn descriptors(&self) -> &[ModelDescriptor] {
            &self.items
        }
    }

    #[test]
    fn registry_lookup_and_routing() {
        let model = FlatSmilePricer::new(0.2);
        let reg = StaticRegistry {
            items: vec![
                PricingModel::descriptor(&model),
                FlatSmileCalibration.descriptor(),
            ],
        };
        assert!(reg.provides(FLAT_PRICER_ID, ModelKind::Pricing));
        assert!(!reg.provides(FLAT_PRICER_ID, ModelKind::Calibration));
        assert_eq!(reg.of_kind(ModelKind::Calibration).len(), 1);
        assert_eq!(
            reg.descriptor(FLAT_PRICER_ID).unwrap().kind,
            ModelKind::Pricing
        );
        assert_eq!(
            reg.descriptor(ModelId("nope")),
            Err(PluginError::NotFound("model id"))
        );
    }

    #[test]
    fn error_display_is_stable() {
        assert_eq!(
            PluginError::InvalidInput("x").to_string(),
            "invalid input: x"
        );
        assert_eq!(
            PluginError::Unsupported("butterfly arbitrage").to_string(),
            "unsupported: butterfly arbitrage"
        );
    }
}
