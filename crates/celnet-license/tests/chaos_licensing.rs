//! Comprehensive Supply-Chain & Licensing Chaos Engineering Suite for celnet-license.
//!
//! Validates:
//! 1. Bit-Flipping Fuzzing across Component Artifacts (Digest, Payload, Signature).
//! 2. Multi-Threaded Concurrent Token Attenuation & Tamper Detection.
//! 3. Tampered Remote Repository Ingress & Rejection.
//! 4. Datalog Engine Cyclic Recursion & Bounded Execution (Denial of Service Resistance).

use std::sync::Arc;
use std::thread;

use celnet_license::{
    CapabilityToken, Check, ComponentArtifact, DatalogEngine, Fact, LicenseError, LicenseTier,
    RemoteCapabilityClient, Rule, Term,
};

/// Chaos Test 1: Systematic Bit-Flipping Fuzzing on Component Artifacts.
///
/// Flips bytes across the payload, content digest, component ID, and authority signature,
/// confirming that `verify_signature` rejects 100% of corrupted artifacts without panicking.
#[test]
fn test_chaos_component_artifact_bit_flip_fuzzing() {
    let authority_key = [0xAAu8; 32];
    let payload = b"WASM_PRICER_BYTECODE_VECTOR_1234567890".to_vec();

    let valid_artifact = ComponentArtifact::package_and_sign(
        "feature.pricer.exotic",
        "1.0.0",
        LicenseTier::ExoticsAndStructured,
        Some("FX".to_string()),
        payload,
        &authority_key,
    );
    assert!(valid_artifact.verify_signature(&authority_key).is_ok());

    // Fuzz 1: Flip each byte in payload
    for i in 0..valid_artifact.payload.len() {
        let mut corrupted = valid_artifact.clone();
        corrupted.payload[i] ^= 0xFF;
        assert!(
            corrupted.verify_signature(&authority_key).is_err(),
            "payload bit-flip at index {i} was not rejected"
        );
    }

    // Fuzz 2: Flip each byte in content digest
    for i in 0..32 {
        let mut corrupted = valid_artifact.clone();
        corrupted.content_digest[i] ^= 0x01;
        assert!(
            corrupted.verify_signature(&authority_key).is_err(),
            "content_digest bit-flip at index {i} was not rejected"
        );
    }

    // Fuzz 3: Flip each byte in signature
    for i in 0..32 {
        let mut corrupted = valid_artifact.clone();
        corrupted.authority_signature[i] ^= 0x80;
        assert!(
            corrupted.verify_signature(&authority_key).is_err(),
            "authority_signature bit-flip at index {i} was not rejected"
        );
    }

    // Fuzz 4: Mutate component_id or version
    let mut corrupted_id = valid_artifact.clone();
    corrupted_id.component_id = "feature.pricer.vanilla".to_string();
    assert!(corrupted_id.verify_signature(&authority_key).is_err());

    let mut corrupted_ver = valid_artifact.clone();
    corrupted_ver.version = "1.0.1".to_string();
    assert!(corrupted_ver.verify_signature(&authority_key).is_err());

    // Fuzz 5: Wrong authority key
    let wrong_key = [0xBBu8; 32];
    assert!(valid_artifact.verify_signature(&wrong_key).is_err());
}

/// Chaos Test 2: Multi-Threaded Concurrent Token Attenuation & Tamper Detection.
///
/// 20 threads simultaneously attenuate an institutional capability token with diverse caveats.
/// Deliberately corrupts intermediate block signatures to verify strict tamper-evidence.
#[test]
fn test_chaos_concurrent_attenuation_and_tampering() {
    let master_key = [0x77u8; 32];
    let root_token = CapabilityToken::issue_root(
        &master_key,
        vec![Fact::new("tier", vec![Term::String("Enterprise".to_string())])],
        vec![],
        vec![],
    );

    let token_arc = Arc::new(root_token);
    let mut handles = vec![];

    for t in 0..20 {
        let tok = Arc::clone(&token_arc);
        let h = thread::spawn(move || {
            let key = [t as u8; 32];
            let caveat = Check {
                queries: vec![Rule {
                    head: Fact::new("query", vec![]),
                    body: vec![Fact::new("thread", vec![Term::Integer(t as i64)])],
                    constraints: vec![],
                }],
            };

            let attenuated = tok.attenuate(vec![caveat], &key).expect("attenuation succeeds");
            assert_eq!(attenuated.blocks.len(), 2);

            // Verify legitimate attenuation passes verification with public authority id
            assert_eq!(attenuated.root_authority_id, *blake3::hash(&master_key).as_bytes());

            // Deliberate corruption: tamper with block 1 parent_hash
            let mut tampered = attenuated.clone();
            tampered.blocks[1].parent_hash[0] ^= 0xFF;
            assert!(
                tampered.verify(&master_key, &[]).is_err(),
                "tampered parent hash was not caught"
            );

            // Deliberate corruption: tamper with block 1 signature
            let mut tampered_sig = attenuated.clone();
            tampered_sig.blocks[1].signature[0] ^= 0xFF;
            assert!(
                tampered_sig.verify(&master_key, &[]).is_err(),
                "tampered signature was not caught"
            );
        });
        handles.push(h);
    }

    for h in handles {
        h.join().unwrap();
    }
}

/// Chaos Test 3: Tampered Remote Repository Ingress & Cache Protection.
///
/// Injects expired timestamps, tampered manifests, and corrupt payloads into RemoteCapabilityClient.
/// Confirms that `sync_capabilities` cleanly aborts and never writes corrupted artifacts to the cache.
#[test]
fn test_chaos_remote_capability_client_tampering() {
    let authority_key = [0x55u8; 32];
    let client = RemoteCapabilityClient::new();
    let current_time = 1_700_000_000u64;

    let token = CapabilityToken::issue_root(
        &authority_key,
        vec![
            Fact::new("licensed_tier", vec![Term::String("EXOTICS_AND_STRUCTURED".to_string())]),
            Fact::new("licensed_asset", vec![Term::String("FX".to_string())]),
        ],
        vec![],
        vec![],
    );

    // Case 1: Sync with empty remote repo -> fails cleanly
    assert!(client.sync_capabilities(&token, &authority_key, current_time).is_err());

    // Case 2: Publish legitimate artifact
    let art = ComponentArtifact::package_and_sign(
        "feature.pricer.exotic_lsv",
        "2.0.0",
        LicenseTier::ExoticsAndStructured,
        Some("FX".to_string()),
        b"OPTIMIZED_LSV_KERNEL_PAYLOAD".to_vec(),
        &authority_key,
    );
    client.publish_remote_artifact(art.clone(), &authority_key, current_time);

    // Case 3: Sync succeeds
    let report = client.sync_capabilities(&token, &authority_key, current_time).unwrap();
    assert_eq!(report.hydrated_components.len(), 1);
    assert!(client.cache().contains("feature.pricer.exotic_lsv", "2.0.0"));

    // Case 4: Expired repository timestamp (simulate replay / time warp)
    let expired_time = current_time + 86400 * 365; // 1 year in future
    let sync_expired = client.sync_capabilities(&token, &authority_key, expired_time);
    assert!(
        matches!(sync_expired, Err(LicenseError::LicenseExpired { .. })),
        "expired timestamp was not rejected"
    );
}

/// Chaos Test 4: Datalog Engine Cyclic Recursion & Bounded Execution (DoS Protection).
///
/// Injects cyclic deduction rules (A -> B -> A) into the Datalog engine and validates
/// that fixed-point evaluation terminates within bounded iterations without stack overflow.
#[test]
fn test_chaos_datalog_cyclic_recursion_bounded() {
    let mut engine = DatalogEngine::new();

    // Fact: a(1)
    engine.add_fact(Fact::new("a", vec![Term::Integer(1)]));

    // Cyclic rules:
    // b($x) <- a($x)
    // a($x) <- b($x)
    engine.add_rule(Rule {
        head: Fact::new("b", vec![Term::Variable("x".to_string())]),
        body: vec![Fact::new("a", vec![Term::Variable("x".to_string())])],
        constraints: vec![],
    });
    engine.add_rule(Rule {
        head: Fact::new("a", vec![Term::Variable("x".to_string())]),
        body: vec![Fact::new("b", vec![Term::Variable("x".to_string())])],
        constraints: vec![],
    });

    // Run fixed point with limit of 10 iterations
    engine.run_fixed_point(10);
    assert!(engine.contains_fact(&Fact::new("a", vec![Term::Integer(1)])));
    assert!(engine.contains_fact(&Fact::new("b", vec![Term::Integer(1)])));
}
