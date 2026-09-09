//! Integration test for Phase 2: Dynamic Feature Distribution & Component Runtime.
//!
//! Verifies:
//! 1. Content-addressed signed artifact creation and verification.
//! 2. Registration into `ModelRegistry` via `load_licensed_artifact`.
//! 3. Execution of dynamic Wasm pricing model loaded from signed artifact.
//! 4. Tampered artifact rejection with cryptographic proof.
//! 5. Untrusted signature rejection.

use celnet_core::CarryInputs;
use celnet_license::artifact::ComponentArtifact;
use celnet_license::manifest::LicenseTier;
use celnet_plugin_api::{GreekSupport, ModelDescriptor, ModelId, ModelKind};
use celnet_plugin_host::{FuelBudget, ModelRegistry};
use celnet_types::{Carry, CcyPair, OptionType, Underlying};

const FWD_PRICER_WAT: &str = r#"
(module
  (import "celnet_math" "exp" (func $exp (param f64) (result f64)))
  (memory (export "memory") 1)
  (func $df_dom (param $in i32) (result f64)
    (call $exp (f64.mul (f64.neg (f64.load offset=32 (local.get $in)))
                        (f64.load offset=24 (local.get $in)))))
  (func $df_for (param $in i32) (result f64)
    (call $exp (f64.mul (f64.neg (f64.load offset=40 (local.get $in)))
                        (f64.load offset=24 (local.get $in)))))
  (func (export "celnet_scratch_in") (result i32) (i32.const 1024))
  (func (export "celnet_scratch_out") (result i32) (i32.const 2048))
  (func (export "celnet_price") (param $opt i32) (param $in i32) (param $len i32) (result f64)
    (local $s_disc f64) (local $k_disc f64)
    (local.set $s_disc (f64.mul (f64.load (local.get $in)) (call $df_for (local.get $in))))
    (local.set $k_disc (f64.mul (f64.load offset=8 (local.get $in)) (call $df_dom (local.get $in))))
    (if (result f64) (i32.eqz (local.get $opt))
      (then (f64.sub (local.get $s_disc) (local.get $k_disc)))
      (else (f64.sub (local.get $k_disc) (local.get $s_disc)))))
  (func (export "celnet_price_greeks")
        (param $opt i32) (param $in i32) (param $inlen i32) (param $out i32) (param $outlen i32)
        (result i32)
    (local $dfd f64) (local $dff f64) (local $s f64) (local $k f64) (local $t f64)
    (local $sign f64) (local $price f64)
    (local.set $s (f64.load (local.get $in)))
    (local.set $k (f64.load offset=8 (local.get $in)))
    (local.set $t (f64.load offset=24 (local.get $in)))
    (local.set $dfd (call $df_dom (local.get $in)))
    (local.set $dff (call $df_for (local.get $in)))
    (local.set $sign (if (result f64) (i32.eqz (local.get $opt))
      (then (f64.const 1)) (else (f64.const -1))))
    (local.set $price (f64.mul (local.get $sign)
      (f64.sub (f64.mul (local.get $s) (local.get $dff))
               (f64.mul (local.get $k) (local.get $dfd)))))
    (f64.store (local.get $out) (local.get $price))
    (f64.store offset=8  (local.get $out) (f64.mul (local.get $sign) (local.get $dff)))
    (f64.store offset=16 (local.get $out) (local.get $sign))
    (f64.store offset=24 (local.get $out) (f64.const 0))
    (f64.store offset=32 (local.get $out) (f64.const 0))
    (f64.store offset=40 (local.get $out)
      (f64.mul (local.get $sign)
        (f64.sub
          (f64.mul (f64.mul (f64.load offset=32 (local.get $in)) (local.get $k)) (local.get $dfd))
          (f64.mul (f64.mul (f64.load offset=40 (local.get $in)) (local.get $s)) (local.get $dff)))))
    (f64.store offset=48 (local.get $out)
      (f64.mul (local.get $sign)
        (f64.mul (f64.mul (local.get $k) (local.get $t)) (local.get $dfd))))
    (f64.store offset=56 (local.get $out)
      (f64.mul (local.get $sign)
        (f64.mul (f64.neg (f64.mul (local.get $s) (local.get $t))) (local.get $dff))))
    (f64.store offset=64  (local.get $out) (f64.const 0))
    (f64.store offset=72  (local.get $out) (f64.const 0))
    (f64.store offset=80  (local.get $out) (f64.const 0))
    (f64.store offset=88  (local.get $out) (f64.const 0))
    (f64.store offset=96  (local.get $out) (f64.const 0))
    (f64.store offset=104 (local.get $out) (f64.const 0))
    ;; rate_kind discriminant: 0 = Fx (rho_dom @48, rho_for @56).
    (i32.store offset=112 (local.get $out) (i32.const 0))
    (i32.const 0)))
"#;

fn eurusd() -> Underlying {
    Underlying::Fx(CcyPair::parse("EURUSD").unwrap())
}

fn sample_inputs() -> CarryInputs {
    CarryInputs::new(
        1.2150,
        1.2000,
        0.11,
        0.75,
        eurusd(),
        Carry::FxRates {
            r_dom: 0.043,
            r_for: 0.011,
        },
    )
}

#[test]
fn test_licensed_artifact_publish_and_dynamic_load() {
    let authority_key = [42u8; 32];
    let wasm_bytes = wat::parse_str(FWD_PRICER_WAT).expect("WAT parsing must succeed");

    let component_id = "fx_dynamic_forward_pricer";
    let version = "1.0.0";
    let artifact = ComponentArtifact::package_and_sign(
        component_id,
        version,
        LicenseTier::CorePricing,
        Some("FX".to_string()),
        wasm_bytes,
        &authority_key,
    );

    // Direct signature verification
    assert!(artifact.verify_signature(&authority_key).is_ok());

    let mut registry = ModelRegistry::new();
    let model_id = ModelId(component_id);
    let descriptor = ModelDescriptor::new(model_id, ModelKind::Pricing, GreekSupport::PRICE_ONLY);

    let result = registry.load_licensed_artifact(
        descriptor,
        &artifact,
        &authority_key,
        FuelBudget::DEFAULT,
    );
    assert!(result.is_ok(), "Failed to load licensed artifact: {:?}", result.err());

    let model = registry.model(model_id).expect("model must exist in registry");
    let inputs = sample_inputs();
    let price_res = model.price(OptionType::Call, &inputs);
    assert!(price_res.is_ok(), "Pricing failed: {:?}", price_res.err());

    let price = price_res.unwrap();
    assert!(price > 0.0, "Expected positive forward call price, got {}", price);
}

#[test]
fn test_tampered_artifact_rejected() {
    let authority_key = [42u8; 32];
    let wasm_bytes = wat::parse_str(FWD_PRICER_WAT).expect("WAT parsing must succeed");

    let component_id = "tampered_pricer";
    let version = "1.0.0";
    let mut artifact = ComponentArtifact::package_and_sign(
        component_id,
        version,
        LicenseTier::CorePricing,
        None,
        wasm_bytes,
        &authority_key,
    );

    // Tamper with payload byte
    artifact.payload[0] ^= 0xFF;

    let mut registry = ModelRegistry::new();
    let model_id = ModelId(component_id);
    let descriptor = ModelDescriptor::new(model_id, ModelKind::Pricing, GreekSupport::PRICE_ONLY);

    let res = registry.load_licensed_artifact(
        descriptor,
        &artifact,
        &authority_key,
        FuelBudget::DEFAULT,
    );
    assert!(res.is_err(), "Tampered artifact must be rejected");
}

#[test]
fn test_wrong_authority_key_rejected() {
    let authority_key = [42u8; 32];
    let attacker_key = [99u8; 32];
    let wasm_bytes = wat::parse_str(FWD_PRICER_WAT).expect("WAT parsing must succeed");

    let component_id = "signed_pricer";
    let version = "1.0.0";
    let artifact = ComponentArtifact::package_and_sign(
        component_id,
        version,
        LicenseTier::CorePricing,
        None,
        wasm_bytes,
        &attacker_key,
    );

    let mut registry = ModelRegistry::new();
    let model_id = ModelId(component_id);
    let descriptor = ModelDescriptor::new(model_id, ModelKind::Pricing, GreekSupport::PRICE_ONLY);

    let res = registry.load_licensed_artifact(
        descriptor,
        &artifact,
        &authority_key, // Legitimate authority key expected
        FuelBudget::DEFAULT,
    );
    assert!(res.is_err(), "Artifact signed with unknown key must be rejected");
}

#[test]
fn test_dynamic_hot_swap_and_cache_hydration() {
    let authority_key = [42u8; 32];
    let wasm_bytes = wat::parse_str(FWD_PRICER_WAT).expect("WAT parsing must succeed");

    let component_id = "fx_dynamic_pricer";
    let artifact_v1 = ComponentArtifact::package_and_sign(
        component_id,
        "1.0.0",
        LicenseTier::CorePricing,
        Some("FX".to_string()),
        wasm_bytes.clone(),
        &authority_key,
    );

    let cache = celnet_license::LocalArtifactCache::new();
    cache.store(artifact_v1);

    let mut registry = ModelRegistry::new();
    let model_id = ModelId(component_id);
    let descriptor = ModelDescriptor::new(model_id, ModelKind::Pricing, GreekSupport::PRICE_ONLY);

    // 1. Initial hydration from local cache
    let id = registry
        .hydrate_from_cache(descriptor, &cache, "1.0.0", &authority_key, FuelBudget::DEFAULT)
        .expect("hydrate from cache v1");
    assert_eq!(id, model_id);
    assert!(registry.has_model(model_id));
    assert_eq!(registry.len(), 1);

    // 2. Publish and hot-swap to v2
    let artifact_v2 = ComponentArtifact::package_and_sign(
        component_id,
        "2.0.0",
        LicenseTier::CorePricing,
        Some("FX".to_string()),
        wasm_bytes,
        &authority_key,
    );
    cache.store(artifact_v2);

    let swapped_id = registry
        .hydrate_from_cache(descriptor, &cache, "2.0.0", &authority_key, FuelBudget::DEFAULT)
        .expect("hot swap from cache v2");
    assert_eq!(swapped_id, model_id);
    // Count remains 1 because it replaced in place
    assert_eq!(registry.len(), 1);

    let model = registry.model(model_id).expect("model must exist");
    let price = model.price(OptionType::Call, &sample_inputs()).unwrap();
    assert!(price > 0.0);
}

