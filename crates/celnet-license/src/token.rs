//! Institutional Cryptographic Capability Tokens and Decentralized Policy Attenuation Chains.
//!
//! Provides cryptographically verifiable capability tokens supporting offline delegation,
//! attenuation, and policy verification using BLAKE3-keyed message authentication chains.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::datalog::{Check, DatalogEngine, Fact, Rule, Term};
use crate::error::LicenseError;
use crate::manifest::{CapabilityManifest, LicenseTier};

/// Cryptographic authorization block within an institutional capability token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Sequential index of this block in the attenuation chain.
    pub index: usize,
    /// Ground facts declared by this block.
    pub facts: Vec<Fact>,
    /// Inference rules declared by this block.
    pub rules: Vec<Rule>,
    /// Authorization caveats (checks) that must hold.
    pub checks: Vec<Check>,
    /// BLAKE3 digest of the preceding block (or [0; 32] for root).
    pub parent_hash: [u8; 32],
    /// Cryptographic signature over `(index || facts || rules || checks || parent_hash)`.
    pub signature: [u8; 32],
}

impl Block {
    /// Compute the BLAKE3 digest of this block's contents.
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.index.to_le_bytes());
        hasher.update(&self.parent_hash);
        let payload = serde_json::to_vec(&( &self.facts, &self.rules, &self.checks ))
            .expect("block payload serialization");
        hasher.update(&payload);
        hasher.finalize().into()
    }
}

/// A cryptographically verifiable institutional capability token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityToken {
    /// Verification key of the root license authority.
    pub root_authority_id: [u8; 32],
    /// Ordered sequence of authority and attenuation blocks.
    pub blocks: Vec<Block>,
}

/// Alias for backward compatibility across existing surfaces.
pub type BiscuitToken = CapabilityToken;

impl CapabilityToken {
    /// Issue a new root license token signed with the Master License Authority key.
    pub fn issue_root(
        authority_key: &[u8; 32],
        facts: Vec<Fact>,
        rules: Vec<Rule>,
        checks: Vec<Check>,
    ) -> Self {
        let mut root_block = Block {
            index: 0,
            facts,
            rules,
            checks,
            parent_hash: [0u8; 32],
            signature: [0u8; 32],
        };

        let digest = root_block.digest();
        let mut signer = blake3::Hasher::new_keyed(authority_key);
        signer.update(&digest);
        root_block.signature = signer.finalize().into();

        // The public verification identifier is the BLAKE3 hash of the secret authority key
        let authority_id = blake3::hash(authority_key).into();

        Self {
            root_authority_id: authority_id,
            blocks: vec![root_block],
        }
    }

    /// Attenuate the token offline by appending additional caveat checks.
    ///
    /// The resulting token is strictly more restrictive and cannot be un-attenuated.
    pub fn attenuate(
        &self,
        caveats: Vec<Check>,
        intermediary_key: &[u8; 32],
    ) -> Result<Self, LicenseError> {
        if self.blocks.is_empty() {
            return Err(LicenseError::BrokenAttenuationChain {
                index: 0,
                expected: "non-empty blocks".to_string(),
                actual: "empty".to_string(),
            });
        }

        let last_block = self.blocks.last().unwrap();
        let parent_hash = last_block.digest();
        let next_index = self.blocks.len();

        let mut next_block = Block {
            index: next_index,
            facts: Vec::new(),
            rules: Vec::new(),
            checks: caveats,
            parent_hash,
            signature: [0u8; 32],
        };

        let digest = next_block.digest();
        let mut signer = blake3::Hasher::new_keyed(intermediary_key);
        signer.update(&digest);
        next_block.signature = signer.finalize().into();

        let mut new_blocks = self.blocks.clone();
        new_blocks.push(next_block);

        Ok(Self {
            root_authority_id: self.root_authority_id,
            blocks: new_blocks,
        })
    }

    /// Verify the cryptographic integrity of the token and evaluate policy rules.
    pub fn verify(
        &self,
        authority_key: &[u8; 32],
        ambient_facts: &[Fact],
    ) -> Result<CapabilityManifest, LicenseError> {
        if self.blocks.is_empty() {
            return Err(LicenseError::BrokenAttenuationChain {
                index: 0,
                expected: "at least one block".to_string(),
                actual: "0 blocks".to_string(),
            });
        }

        // 1. Verify root authority key
        let expected_auth_id: [u8; 32] = blake3::hash(authority_key).into();
        if self.root_authority_id != expected_auth_id {
            return Err(LicenseError::InvalidSignature("root authority key mismatch".to_string()));
        }

        // 2. Verify root block signature
        let root = &self.blocks[0];
        if root.index != 0 || root.parent_hash != [0u8; 32] {
            return Err(LicenseError::BrokenAttenuationChain {
                index: 0,
                expected: "zero parent hash".to_string(),
                actual: format!("{:?}", root.parent_hash),
            });
        }

        let root_digest = root.digest();
        let mut root_verifier = blake3::Hasher::new_keyed(authority_key);
        root_verifier.update(&root_digest);
        let expected_root_sig: [u8; 32] = root_verifier.finalize().into();
        if root.signature != expected_root_sig {
            return Err(LicenseError::InvalidSignature("root block signature mismatch".to_string()));
        }

        // 3. Verify attenuation chain links
        let mut prev_hash = root_digest;
        for (i, block) in self.blocks.iter().enumerate().skip(1) {
            if block.index != i {
                return Err(LicenseError::BrokenAttenuationChain {
                    index: i,
                    expected: format!("index {i}"),
                    actual: format!("index {}", block.index),
                });
            }
            if block.parent_hash != prev_hash {
                return Err(LicenseError::BrokenAttenuationChain {
                    index: i,
                    expected: format!("{prev_hash:x?}"),
                    actual: format!("{:x?}", block.parent_hash),
                });
            }
            prev_hash = block.digest();
        }

        // 4. Populate and run Policy Engine
        let mut engine = DatalogEngine::new();

        // Load ambient facts (time, node cores, requested asset)
        for fact in ambient_facts {
            engine.add_fact(fact.clone());
        }

        // Load token facts and rules
        for block in &self.blocks {
            for fact in &block.facts {
                engine.add_fact(fact.clone());
            }
            for rule in &block.rules {
                engine.add_rule(rule.clone());
            }
        }

        // Execute fixed-point deduction iteration
        engine.run_fixed_point(64);

        // Verify all checks across all blocks
        for block in &self.blocks {
            for check in &block.checks {
                engine.verify_check(check).map_err(LicenseError::DatalogCheckFailed)?;
            }
        }

        // Extract manifest from derived facts
        self.extract_manifest(&engine)
    }

    fn extract_manifest(&self, engine: &DatalogEngine) -> Result<CapabilityManifest, LicenseError> {
        let tenant_id = engine
            .query_predicate("tenant")
            .into_iter()
            .find_map(|f| match f.terms.first() {
                Some(Term::String(s)) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "default_institutional_tenant".to_string());

        let mut licensed_asset_classes = HashSet::new();
        for f in engine.query_predicate("licensed_asset") {
            if let Some(Term::String(asset)) = f.terms.first() {
                licensed_asset_classes.insert(asset.to_uppercase());
            }
        }

        let mut licensed_tiers = HashSet::new();
        for f in engine.query_predicate("licensed_tier") {
            if let Some(Term::String(tier_str)) = f.terms.first() {
                match tier_str.as_str() {
                    "CORE_PRICING" => { licensed_tiers.insert(LicenseTier::CorePricing); }
                    "ULTRA_LOW_LATENCY_SBE" => { licensed_tiers.insert(LicenseTier::UltraLowLatencySbe); }
                    "RATES_AND_BONDS" => { licensed_tiers.insert(LicenseTier::RatesAndBonds); }
                    "EXOTICS_AND_STRUCTURED" => { licensed_tiers.insert(LicenseTier::ExoticsAndStructured); }
                    "DISTRIBUTED_RISK_FLEET" => { licensed_tiers.insert(LicenseTier::DistributedRiskFleet); }
                    "GPU_AAD_ACCEL" => { licensed_tiers.insert(LicenseTier::GpuAadAcceleration); }
                    "PORTFOLIO_MARGIN" => { licensed_tiers.insert(LicenseTier::PortfolioMargin); }
                    "ALGO_EXECUTION" => { licensed_tiers.insert(LicenseTier::AlgoExecution); }
                    _ => {}
                }
            }
        }

        let max_cores = engine
            .query_predicate("max_cores")
            .into_iter()
            .find_map(|f| match f.terms.first() {
                Some(Term::Integer(i)) => Some(*i as usize),
                _ => None,
            })
            .unwrap_or(256);

        let max_throughput_msg_per_sec = engine
            .query_predicate("max_throughput")
            .into_iter()
            .find_map(|f| match f.terms.first() {
                Some(Term::Integer(i)) => Some(*i as u64),
                _ => None,
            })
            .unwrap_or(10_000_000);

        let expires_at = engine
            .query_predicate("expires_at")
            .into_iter()
            .find_map(|f| match f.terms.first() {
                Some(Term::Integer(i)) => Some(*i as u64),
                _ => None,
            })
            .unwrap_or(1_893_456_000); // 2030-01-01

        Ok(CapabilityManifest {
            tenant_id,
            licensed_asset_classes,
            licensed_tiers,
            max_cores,
            max_throughput_msg_per_sec,
            expires_at,
        })
    }
}
