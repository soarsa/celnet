//! Content-Addressed Component Artifact Distribution and Cryptographic Supply-Chain Verification.
//!
//! Ensures dynamic components are cryptographically signed, tamper-evident, and authorized
//! by the node's institutional capability token before execution.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::LicenseError;
use crate::manifest::LicenseTier;
use crate::token::CapabilityToken;

/// Signed, content-addressed component artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentArtifact {
    /// Canonical component identifier (e.g. "feature.pricer.exotic_lsv").
    pub component_id: String,
    /// Semantic version string (e.g. "1.0.0").
    pub version: String,
    /// The functional capability tier required to execute this component.
    pub required_tier: LicenseTier,
    /// Optional asset class restriction (e.g. "FX", "RATES").
    pub required_asset_class: Option<String>,
    /// The binary bytecode or relocatable module bytes.
    pub payload: Vec<u8>,
    /// BLAKE3 256-bit cryptographic digest over the payload.
    pub content_digest: [u8; 32],
    /// Release Authority cryptographic signature over `(component_id || version || content_digest)`.
    pub authority_signature: [u8; 32],
}

impl ComponentArtifact {
    /// Package and cryptographically sign a component artifact using the Release Authority key.
    pub fn package_and_sign(
        component_id: impl Into<String>,
        version: impl Into<String>,
        required_tier: LicenseTier,
        required_asset_class: Option<String>,
        payload: Vec<u8>,
        authority_key: &[u8; 32],
    ) -> Self {
        let cid = component_id.into();
        let ver = version.into();
        let digest: [u8; 32] = blake3::hash(&payload).into();

        let mut signer = blake3::Hasher::new_keyed(authority_key);
        signer.update(cid.as_bytes());
        signer.update(ver.as_bytes());
        signer.update(&digest);
        let signature: [u8; 32] = signer.finalize().into();

        Self {
            component_id: cid,
            version: ver,
            required_tier,
            required_asset_class,
            payload,
            content_digest: digest,
            authority_signature: signature,
        }
    }

    /// Verify the cryptographic integrity and authenticity of this artifact.
    pub fn verify_signature(&self, authority_key: &[u8; 32]) -> Result<(), LicenseError> {
        // 1. Verify payload digest matches actual content
        let actual_digest: [u8; 32] = blake3::hash(&self.payload).into();
        if actual_digest != self.content_digest {
            return Err(LicenseError::Serialization(
                "payload digest mismatch: content has been altered".to_string(),
            ));
        }

        // 2. Verify signature
        let mut verifier = blake3::Hasher::new_keyed(authority_key);
        verifier.update(self.component_id.as_bytes());
        verifier.update(self.version.as_bytes());
        verifier.update(&self.content_digest);
        let expected_sig: [u8; 32] = verifier.finalize().into();

        if self.authority_signature != expected_sig {
            return Err(LicenseError::InvalidSignature(
                "component artifact signature mismatch".to_string(),
            ));
        }

        Ok(())
    }
}

/// Content-addressed repository of signed component artifacts.
#[derive(Debug, Default, Clone)]
pub struct ArtifactRegistry {
    artifacts: Arc<RwLock<HashMap<String, ComponentArtifact>>>,
}

impl ArtifactRegistry {
    /// Create a new empty artifact repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a signed component artifact to the registry.
    pub fn publish(&self, artifact: ComponentArtifact) {
        let mut map = self.artifacts.write().expect("artifact write lock");
        map.insert(artifact.component_id.clone(), artifact);
    }

    /// Fetch, cryptographically verify, and authorize a component artifact using a node's capability token.
    pub fn fetch_and_authorize(
        &self,
        component_id: &str,
        node_token: &CapabilityToken,
        authority_key: &[u8; 32],
    ) -> Result<ComponentArtifact, LicenseError> {
        let map = self.artifacts.read().expect("artifact read lock");
        let artifact = map
            .get(component_id)
            .cloned()
            .ok_or_else(|| LicenseError::CapabilityDenied(format!("artifact '{component_id}' not found")))?;

        // 1. Verify artifact cryptographic signature
        artifact.verify_signature(authority_key)?;

        // 2. Verify node's license token and evaluate capability manifest
        let manifest = node_token.verify(authority_key, &[])?;

        // 3. Verify node is licensed for the required tier and asset class
        if !manifest.is_feature_authorized(
            artifact.required_asset_class.as_deref(),
            artifact.required_tier,
        ) {
            return Err(LicenseError::CapabilityDenied(format!(
                "node token does not authorize tier {:?} for asset {:?}",
                artifact.required_tier, artifact.required_asset_class
            )));
        }

        Ok(artifact)
    }
}
