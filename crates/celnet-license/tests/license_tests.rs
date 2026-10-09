//! Comprehensive unit and integration test suite for celnet-license.
//!
//! Validates:
//! 1. Hardware probing (NHED) and TPM attestation quotes.
//! 2. Declarative policy inference rules and authorization checks.
//! 3. Cryptographic capability token issuance, offline attenuation, and tamper resistance.
//! 4. Hardware and temporal quota enforcement.
//! 5. Content-addressed capability repository, TUF manifests, and automated synchronization.

use celnet_license::{
    CapabilityManifest, CapabilityTargetEntry, CapabilityTargetsManifest, CapabilityToken, Check,
    ComponentArtifact, Constraint, DatalogEngine, Fact, LicenseError, LicenseTier,
    NodeHardwareDescriptor, NodeIdentityQuote, Op, RemoteCapabilityClient,
    RepositoryTimestamp, Rule, Term, VectorIsa, probe_local_hardware,
};
use std::collections::HashMap;

#[test]
fn test_hardware_probing_and_digest_determinism() {
    let hw = probe_local_hardware();
    assert!(hw.logical_cores >= 1);
    assert!(hw.physical_cores >= 1);

    let digest1 = hw.digest();
    let digest2 = hw.digest();
    assert_eq!(digest1, digest2, "NHED digest must be strictly deterministic");
}

#[test]
fn test_tpm_attestation_quote_verification() {
    let hw = NodeHardwareDescriptor {
        node_id: "prod-ldn-01".to_string(),
        physical_cores: 64,
        logical_cores: 128,
        numa_nodes: 2,
        l3_cache_bytes: 128 * 1024 * 1024,
        vector_isa: VectorIsa::Avx512,
        has_gpu: true,
        has_cxl_pmem: true,
        nic_driver: celnet_license::NicDriver::AfXdp,
    };

    let tpm_root_key = [0x42u8; 32];
    let ephemeral_pubkey = [0x99u8; 32];

    let quote = NodeIdentityQuote::sign_with_tpm(ephemeral_pubkey, &hw, &tpm_root_key);
    assert!(quote.verify(&hw, &tpm_root_key));

    // Tampered hardware descriptor should fail verification
    let mut tampered_hw = hw.clone();
    tampered_hw.physical_cores = 32;
    assert!(!quote.verify(&tampered_hw, &tpm_root_key));

    // Wrong TPM root key should fail verification
    let wrong_tpm_key = [0x11u8; 32];
    assert!(!quote.verify(&hw, &wrong_tpm_key));
}

#[test]
fn test_datalog_engine_deduction_and_checks() {
    let mut engine = DatalogEngine::new();

    // Base facts
    engine.add_fact(Fact::new("licensed_asset", vec![Term::String("FX".to_string())]));
    engine.add_fact(Fact::new("max_cores", vec![Term::Integer(128)]));

    // Rule: allow_trading($asset) <- licensed_asset($asset)
    engine.add_rule(Rule {
        head: Fact::new("allow_trading", vec![Term::Variable("a".to_string())]),
        body: vec![Fact::new("licensed_asset", vec![Term::Variable("a".to_string())])],
        constraints: vec![],
    });

    engine.run_fixed_point(10);

    // Assert deduction occurred
    assert!(engine.contains_fact(&Fact::new("allow_trading", vec![Term::String("FX".to_string())])));

    // Check caveat: check if max_cores($c), $c >= 64
    let valid_check = Check {
        queries: vec![Rule {
            head: Fact::new("query", vec![]),
            body: vec![Fact::new("max_cores", vec![Term::Variable("c".to_string())])],
            constraints: vec![Constraint {
                variable: "c".to_string(),
                op: Op::Ge,
                target: Term::Integer(64),
            }],
        }],
    };
    assert!(engine.verify_check(&valid_check).is_ok());

    // Check caveat that fails: check if max_cores($c), $c > 256
    let failing_check = Check {
        queries: vec![Rule {
            head: Fact::new("query", vec![]),
            body: vec![Fact::new("max_cores", vec![Term::Variable("c".to_string())])],
            constraints: vec![Constraint {
                variable: "c".to_string(),
                op: Op::Gt,
                target: Term::Integer(256),
            }],
        }],
    };
    assert!(engine.verify_check(&failing_check).is_err());
}

#[test]
fn test_biscuit_token_issuance_attenuation_and_verification() {
    let authority_key = [0x77u8; 32];

    // 1. Issue root enterprise license
    let root_facts = vec![
        Fact::new("tenant", vec![Term::String("GoldmanCelnetCorp".to_string())]),
        Fact::new("licensed_asset", vec![Term::String("FX".to_string())]),
        Fact::new("licensed_asset", vec![Term::String("RATES".to_string())]),
        Fact::new("licensed_tier", vec![Term::String("CORE_PRICING".to_string())]),
        Fact::new("licensed_tier", vec![Term::String("ULTRA_LOW_LATENCY_SBE".to_string())]),
        Fact::new("licensed_tier", vec![Term::String("DISTRIBUTED_RISK_FLEET".to_string())]),
        Fact::new("max_cores", vec![Term::Integer(256)]),
        Fact::new("expires_at", vec![Term::Integer(1_800_000_000)]),
    ];

    let root_token = CapabilityToken::issue_root(&authority_key, root_facts, vec![], vec![]);

    // 2. Verify root token
    let manifest = root_token.verify(&authority_key, &[]).expect("root token verify");
    assert_eq!(manifest.tenant_id, "GoldmanCelnetCorp");
    assert!(manifest.is_feature_authorized(Some("FX"), LicenseTier::CorePricing));
    assert!(manifest.is_feature_authorized(Some("RATES"), LicenseTier::UltraLowLatencySbe));
    assert!(!manifest.is_feature_authorized(Some("EQUITY"), LicenseTier::CorePricing));

    // 3. Offline Attenuation: Delegate creates child token restricting cores to 64
    let delegate_key = [0x88u8; 32];
    let caveat = Check {
        queries: vec![Rule {
            head: Fact::new("query", vec![]),
            body: vec![Fact::new("current_cores", vec![Term::Variable("c".to_string())])],
            constraints: vec![Constraint {
                variable: "c".to_string(),
                op: Op::Le,
                target: Term::Integer(64),
            }],
        }],
    };

    let child_token = root_token
        .attenuate(vec![caveat], &delegate_key)
        .expect("attenuate");
    assert_eq!(child_token.blocks.len(), 2);

    // 4. Verification with ambient fact current_cores = 32 (should PASS)
    let ambient_pass = vec![Fact::new("current_cores", vec![Term::Integer(32)])];
    assert!(child_token.verify(&authority_key, &ambient_pass).is_ok());

    // 5. Verification with ambient fact current_cores = 96 (should FAIL due to caveat)
    let ambient_fail = vec![Fact::new("current_cores", vec![Term::Integer(96)])];
    let err = child_token.verify(&authority_key, &ambient_fail).unwrap_err();
    assert!(matches!(err, LicenseError::DatalogCheckFailed(_)));
}

#[test]
fn test_manifest_validation_against_hardware_and_expiry() {
    let mut tiers = std::collections::HashSet::new();
    tiers.insert(LicenseTier::CorePricing);

    let manifest = CapabilityManifest {
        tenant_id: "Desk1".to_string(),
        licensed_asset_classes: std::collections::HashSet::new(),
        licensed_tiers: tiers,
        max_cores: 32,
        max_throughput_msg_per_sec: 1_000_000,
        expires_at: 1_750_000_000,
    };

    let hw = NodeHardwareDescriptor {
        node_id: "node-x".to_string(),
        physical_cores: 16,
        logical_cores: 32,
        numa_nodes: 1,
        l3_cache_bytes: 32 * 1024 * 1024,
        vector_isa: VectorIsa::Standard,
        has_gpu: false,
        has_cxl_pmem: false,
        nic_driver: celnet_license::NicDriver::StandardKernel,
    };

    // Before expiry, within cores
    assert!(manifest.validate_node(&hw, 1_700_000_000).is_ok());

    // After expiry
    let expired_err = manifest.validate_node(&hw, 1_800_000_000).unwrap_err();
    assert!(matches!(expired_err, LicenseError::LicenseExpired { .. }));

    // Over core limit
    let mut heavy_hw = hw.clone();
    heavy_hw.physical_cores = 64;
    let core_err = manifest.validate_node(&heavy_hw, 1_700_000_000).unwrap_err();
    assert!(matches!(core_err, LicenseError::CoreQuotaExceeded { .. }));
}

#[test]
fn test_artifact_packaging_signature_and_authorization() {
    let authority_key = [0x55u8; 32];
    let registry = celnet_license::ArtifactRegistry::new();

    // 1. Package a signed exotic pricing pricer artifact
    let pricer_wasm = b"\x00asm\x01\x00\x00\x00_celnet_lsv_particle_bytecode".to_vec();
    let artifact = ComponentArtifact::package_and_sign(
        "feature.pricer.exotic_lsv",
        "2.4.0",
        LicenseTier::ExoticsAndStructured,
        Some("FX".to_string()),
        pricer_wasm.clone(),
        &authority_key,
    );
    assert!(artifact.verify_signature(&authority_key).is_ok());

    // 2. Publish artifact to repository
    registry.publish(artifact);

    // 3. Issue node token with ExoticsAndStructured tier
    let root_facts = vec![
        Fact::new("tenant", vec![Term::String("HedgeFundAlpha".to_string())]),
        Fact::new("licensed_asset", vec![Term::String("FX".to_string())]),
        Fact::new("licensed_tier", vec![Term::String("EXOTICS_AND_STRUCTURED".to_string())]),
    ];
    let node_token = CapabilityToken::issue_root(&authority_key, root_facts, vec![], vec![]);

    // 4. Fetch and authorize artifact
    let fetched = registry
        .fetch_and_authorize("feature.pricer.exotic_lsv", &node_token, &authority_key)
        .expect("authorized pull");
    assert_eq!(fetched.payload, pricer_wasm);

    // 5. Node lacking ExoticsAndStructured tier should be DENIED
    let vanilla_facts = vec![
        Fact::new("tenant", vec![Term::String("RetailBroker".to_string())]),
        Fact::new("licensed_asset", vec![Term::String("FX".to_string())]),
        Fact::new("licensed_tier", vec![Term::String("CORE_PRICING".to_string())]),
    ];
    let vanilla_token = CapabilityToken::issue_root(&authority_key, vanilla_facts, vec![], vec![]);
    let denied = registry
        .fetch_and_authorize("feature.pricer.exotic_lsv", &vanilla_token, &authority_key)
        .unwrap_err();
    assert!(matches!(denied, LicenseError::CapabilityDenied(_)));

    // 6. Tampered artifact payload should be REJECTED
    let mut tampered_artifact = ComponentArtifact::package_and_sign(
        "feature.pricer.tampered",
        "1.0.0",
        LicenseTier::CorePricing,
        None,
        b"legitimate_bytes".to_vec(),
        &authority_key,
    );
    tampered_artifact.payload = b"malicious_tampered_bytes".to_vec();
    assert!(tampered_artifact.verify_signature(&authority_key).is_err());
}

#[test]
fn test_remote_capability_client_sync_and_caching() {
    let authority_key = [0x99u8; 32];
    let client = RemoteCapabilityClient::new();
    let current_time = 1_725_000_000;

    // 1. Publish Rates Multi-Curve and Portfolio Margin artifacts to remote repo
    let rates_payload = b"\x00asm\x01\x00\x00\x00_rates_multicurve_bytecode".to_vec();
    let rates_art = ComponentArtifact::package_and_sign(
        "feature.rates.multicurve",
        "1.0.0",
        LicenseTier::RatesAndBonds,
        Some("RATES".to_string()),
        rates_payload.clone(),
        &authority_key,
    );

    let margin_payload = b"\x00asm\x01\x00\x00\x00_simm_margin_bytecode".to_vec();
    let margin_art = ComponentArtifact::package_and_sign(
        "feature.margin.isda_simm",
        "2.6.0",
        LicenseTier::PortfolioMargin,
        None,
        margin_payload.clone(),
        &authority_key,
    );

    client.publish_remote_artifact(rates_art, &authority_key, current_time);
    client.publish_remote_artifact(margin_art, &authority_key, current_time);

    // 2. Node token licensed ONLY for RATES (not Portfolio Margin)
    let node_facts = vec![
        Fact::new("tenant", vec![Term::String("BankTreasury".to_string())]),
        Fact::new("licensed_asset", vec![Term::String("RATES".to_string())]),
        Fact::new("licensed_tier", vec![Term::String("RATES_AND_BONDS".to_string())]),
    ];
    let token = CapabilityToken::issue_root(&authority_key, node_facts, vec![], vec![]);

    // 3. Sync cycle
    let report = client
        .sync_capabilities(&token, &authority_key, current_time)
        .expect("sync must succeed");

    assert_eq!(report.hydrated_components, vec!["feature.rates.multicurve".to_string()]);
    assert_eq!(report.skipped_unlicensed, vec!["feature.margin.isda_simm".to_string()]);

    // 4. Verify local cache now has the hydrated artifact
    let cached = client
        .cache()
        .get("feature.rates.multicurve", "1.0.0")
        .expect("cached entry must exist");
    assert_eq!(cached.payload, rates_payload);

    // 5. Run second sync cycle - should report up_to_date
    let report2 = client
        .sync_capabilities(&token, &authority_key, current_time)
        .expect("second sync must succeed");
    assert!(report2.hydrated_components.is_empty());
    assert_eq!(report2.up_to_date, vec!["feature.rates.multicurve".to_string()]);
}

#[test]
fn test_tuf_manifest_verification_and_tamper_rejection() {
    let authority_key = [0x33u8; 32];
    let attacker_key = [0x66u8; 32];

    let mut targets = HashMap::new();
    targets.insert(
        "feature.algo.twap".to_string(),
        CapabilityTargetEntry {
            component_id: "feature.algo.twap".to_string(),
            version: "1.1.0".to_string(),
            required_tier: LicenseTier::AlgoExecution,
            required_asset_class: Some("EQUITY".to_string()),
            content_digest: [0xAA; 32],
            size_bytes: 4096,
            release_timestamp: 1_700_000_000,
        },
    );

    let manifest = CapabilityTargetsManifest::create_and_sign(1, targets, &authority_key);
    assert!(manifest.verify_signature(&authority_key).is_ok());
    assert!(manifest.verify_signature(&attacker_key).is_err());

    // Timestamp freeze rejection
    let ts = RepositoryTimestamp::create_and_sign(1, [0x11; 32], 1_700_000_000, &authority_key);
    let expired_err = ts.verify(&authority_key, 1_800_000_000).unwrap_err();
    assert!(matches!(expired_err, LicenseError::LicenseExpired { .. }));
}
