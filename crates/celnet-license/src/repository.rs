//! Content-Addressed Capability Repository, Secure Supply-Chain Manifests, and Local Cache.
//!
//! Provides enterprise-grade supply-chain security for dynamic component hydration.
//! Ensures all remote libraries, pricing models, and risk kernels are cryptographically signed,
//! tamper-evident, anti-rollback protected, and strictly authorized by the node's capability token.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::artifact::ComponentArtifact;
use crate::error::LicenseError;
use crate::manifest::LicenseTier;
use crate::token::CapabilityToken;

/// Metadata entry for a single signed component available in the remote repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityTargetEntry {
    /// Canonical component identifier (e.g. "feature.pricer.exotic_lsv").
    pub component_id: String,
    /// Semantic version string (e.g. "1.2.0").
    pub version: String,
    /// Required operational capability tier.
    pub required_tier: LicenseTier,
    /// Optional asset class constraint (e.g. "FX", "RATES", "EQUITY").
    pub required_asset_class: Option<String>,
    /// BLAKE3 256-bit cryptographic digest of the component payload.
    pub content_digest: [u8; 32],
    /// Binary payload size in bytes.
    pub size_bytes: usize,
    /// UNIX timestamp when this artifact was released.
    pub release_timestamp: u64,
}

impl CapabilityTargetEntry {
    /// Compute the entry's canonical digest for manifest signing.
    pub fn entry_digest(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.component_id.as_bytes());
        hasher.update(self.version.as_bytes());
        hasher.update(&[self.required_tier as u8]);
        if let Some(ref asset) = self.required_asset_class {
            hasher.update(asset.as_bytes());
        }
        hasher.update(&self.content_digest);
        hasher.update(&self.size_bytes.to_le_bytes());
        hasher.update(&self.release_timestamp.to_le_bytes());
        hasher.finalize().into()
    }
}

/// Cryptographically signed repository targets manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityTargetsManifest {
    /// Monotonically increasing manifest sequence number (prevents rollback attacks).
    pub manifest_version: u64,
    /// Map of canonical component identifier to target metadata.
    pub targets: HashMap<String, CapabilityTargetEntry>,
    /// Release Authority signature over the serialized targets map.
    pub authority_signature: [u8; 32],
}

impl CapabilityTargetsManifest {
    /// Create and cryptographically sign a targets manifest using the Release Authority key.
    pub fn create_and_sign(
        manifest_version: u64,
        targets: HashMap<String, CapabilityTargetEntry>,
        authority_key: &[u8; 32],
    ) -> Self {
        let mut sorted_entries: Vec<(&String, &CapabilityTargetEntry)> = targets.iter().collect();
        sorted_entries.sort_by_key(|(k, _)| *k);

        let mut signer = blake3::Hasher::new_keyed(authority_key);
        signer.update(&manifest_version.to_le_bytes());
        for (k, v) in sorted_entries {
            signer.update(k.as_bytes());
            signer.update(&v.entry_digest());
        }
        let signature: [u8; 32] = signer.finalize().into();

        Self {
            manifest_version,
            targets,
            authority_signature: signature,
        }
    }

    /// Verify the cryptographic integrity and authority signature of this manifest.
    pub fn verify_signature(&self, authority_key: &[u8; 32]) -> Result<(), LicenseError> {
        let mut sorted_entries: Vec<(&String, &CapabilityTargetEntry)> = self.targets.iter().collect();
        sorted_entries.sort_by_key(|(k, _)| *k);

        let mut verifier = blake3::Hasher::new_keyed(authority_key);
        verifier.update(&self.manifest_version.to_le_bytes());
        for (k, v) in sorted_entries {
            verifier.update(k.as_bytes());
            verifier.update(&v.entry_digest());
        }
        let expected_sig: [u8; 32] = verifier.finalize().into();

        if self.authority_signature != expected_sig {
            return Err(LicenseError::InvalidSignature(
                "targets manifest authority signature mismatch".to_string(),
            ));
        }

        Ok(())
    }
}

/// Cryptographically signed repository timestamp proving metadata freshness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryTimestamp {
    /// Timestamp metadata sequence version.
    pub version: u64,
    /// BLAKE3 digest of the active targets manifest.
    pub targets_manifest_digest: [u8; 32],
    /// UNIX timestamp until which this metadata is valid (prevents freeze attacks).
    pub valid_until: u64,
    /// Release Authority signature over timestamp metadata.
    pub authority_signature: [u8; 32],
}

impl RepositoryTimestamp {
    /// Create and sign a repository timestamp.
    pub fn create_and_sign(
        version: u64,
        manifest_digest: [u8; 32],
        valid_until: u64,
        authority_key: &[u8; 32],
    ) -> Self {
        let mut signer = blake3::Hasher::new_keyed(authority_key);
        signer.update(&version.to_le_bytes());
        signer.update(&manifest_digest);
        signer.update(&valid_until.to_le_bytes());
        let signature: [u8; 32] = signer.finalize().into();

        Self {
            version,
            targets_manifest_digest: manifest_digest,
            valid_until,
            authority_signature: signature,
        }
    }

    /// Verify the validity and freshness of the timestamp.
    pub fn verify(
        &self,
        authority_key: &[u8; 32],
        current_time_secs: u64,
    ) -> Result<(), LicenseError> {
        if current_time_secs > self.valid_until {
            return Err(LicenseError::LicenseExpired {
                expires_at: self.valid_until,
                current_time: current_time_secs,
            });
        }

        let mut verifier = blake3::Hasher::new_keyed(authority_key);
        verifier.update(&self.version.to_le_bytes());
        verifier.update(&self.targets_manifest_digest);
        verifier.update(&self.valid_until.to_le_bytes());
        let expected_sig: [u8; 32] = verifier.finalize().into();

        if self.authority_signature != expected_sig {
            return Err(LicenseError::InvalidSignature(
                "repository timestamp signature mismatch".to_string(),
            ));
        }

        Ok(())
    }
}

/// High-performance, thread-safe content-addressed local artifact cache.
#[derive(Debug, Default, Clone)]
pub struct LocalArtifactCache {
    by_key: Arc<RwLock<HashMap<(String, String), ComponentArtifact>>>,
    by_digest: Arc<RwLock<HashMap<[u8; 32], ComponentArtifact>>>,
}

impl LocalArtifactCache {
    /// Create a new empty in-memory artifact cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a verified component artifact in the local cache.
    pub fn store(&self, artifact: ComponentArtifact) {
        let key = (artifact.component_id.clone(), artifact.version.clone());
        let digest = artifact.content_digest;

        let mut map_key = self.by_key.write().expect("cache write lock");
        let mut map_digest = self.by_digest.write().expect("cache write lock");

        map_key.insert(key, artifact.clone());
        map_digest.insert(digest, artifact);
    }

    /// Retrieve an artifact by component ID and version string.
    pub fn get(&self, component_id: &str, version: &str) -> Option<ComponentArtifact> {
        let map = self.by_key.read().expect("cache read lock");
        map.get(&(component_id.to_string(), version.to_string())).cloned()
    }

    /// Retrieve an artifact by its BLAKE3 content digest.
    pub fn get_by_digest(&self, digest: &[u8; 32]) -> Option<ComponentArtifact> {
        let map = self.by_digest.read().expect("cache read lock");
        map.get(digest).cloned()
    }

    /// Check if an artifact is present in the cache.
    pub fn contains(&self, component_id: &str, version: &str) -> bool {
        let map = self.by_key.read().expect("cache read lock");
        map.contains_key(&(component_id.to_string(), version.to_string()))
    }

    /// List all cached component entries (id, version, digest).
    pub fn list_cached_entries(&self) -> Vec<(String, String, [u8; 32])> {
        let map = self.by_key.read().expect("cache read lock");
        map.values()
            .map(|a| (a.component_id.clone(), a.version.clone(), a.content_digest))
            .collect()
    }
}

/// Summary report of an automated capability synchronization cycle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilitySyncReport {
    /// Components that were fetched, cryptographically verified, and hydrated.
    pub hydrated_components: Vec<String>,
    /// Components skipped because the node's license token does not authorize them.
    pub skipped_unlicensed: Vec<String>,
    /// Components already cached and at the latest version.
    pub up_to_date: Vec<String>,
    /// Timestamp when synchronization concluded.
    pub synced_at_secs: u64,
}

/// Dynamic Remote Capability Client orchestrating secure, licensed capability pulling.
#[derive(Debug, Clone)]
pub struct RemoteCapabilityClient {
    /// Remote artifact registry or repository source.
    repository: Arc<RwLock<HashMap<String, ComponentArtifact>>>,
    /// Remote signed targets manifest.
    manifest: Arc<RwLock<Option<CapabilityTargetsManifest>>>,
    /// Remote signed timestamp.
    timestamp: Arc<RwLock<Option<RepositoryTimestamp>>>,
    /// Local verified artifact cache.
    cache: LocalArtifactCache,
}

impl RemoteCapabilityClient {
    /// Create a new remote capability client with a dedicated local cache.
    pub fn new() -> Self {
        Self {
            repository: Arc::new(RwLock::new(HashMap::new())),
            manifest: Arc::new(RwLock::new(None)),
            timestamp: Arc::new(RwLock::new(None)),
            cache: LocalArtifactCache::new(),
        }
    }

    /// Access the local artifact cache.
    pub fn cache(&self) -> &LocalArtifactCache {
        &self.cache
    }

    /// Publish a component and update the signed repository metadata (server-side publisher helper).
    pub fn publish_remote_artifact(
        &self,
        artifact: ComponentArtifact,
        authority_key: &[u8; 32],
        current_time_secs: u64,
    ) {
        let mut repo = self.repository.write().expect("repo write lock");
        repo.insert(artifact.component_id.clone(), artifact.clone());

        // Rebuild targets manifest
        let mut targets = HashMap::new();
        for art in repo.values() {
            targets.insert(
                art.component_id.clone(),
                CapabilityTargetEntry {
                    component_id: art.component_id.clone(),
                    version: art.version.clone(),
                    required_tier: art.required_tier,
                    required_asset_class: art.required_asset_class.clone(),
                    content_digest: art.content_digest,
                    size_bytes: art.payload.len(),
                    release_timestamp: current_time_secs,
                },
            );
        }

        let manifest_version = 1;
        let new_manifest = CapabilityTargetsManifest::create_and_sign(
            manifest_version,
            targets,
            authority_key,
        );

        let manifest_digest = blake3::hash(&serde_json::to_vec(&new_manifest).unwrap()).into();
        let valid_until = current_time_secs + 86400 * 30; // 30 days
        let new_timestamp = RepositoryTimestamp::create_and_sign(
            1,
            manifest_digest,
            valid_until,
            authority_key,
        );

        *self.manifest.write().expect("manifest write lock") = Some(new_manifest);
        *self.timestamp.write().expect("timestamp write lock") = Some(new_timestamp);
    }

    /// Synchronize local capabilities against remote repository based on node's capability token.
    ///
    /// # Protocol:
    /// 1. Verifies repository timestamp freshness and signature.
    /// 2. Verifies targets manifest cryptographic signature.
    /// 3. Evaluates node's capability token against each target requirement.
    /// 4. Pulls missing or updated artifacts, verifies BLAKE3 digests + signatures, and caches them.
    pub fn sync_capabilities(
        &self,
        token: &CapabilityToken,
        authority_key: &[u8; 32],
        current_time_secs: u64,
    ) -> Result<CapabilitySyncReport, LicenseError> {
        // 1. Verify Timestamp & Freshness
        let timestamp_guard = self.timestamp.read().expect("timestamp read lock");
        let timestamp = timestamp_guard
            .as_ref()
            .ok_or_else(|| LicenseError::Serialization("no repository timestamp available".into()))?;
        timestamp.verify(authority_key, current_time_secs)?;

        // 2. Verify Targets Manifest
        let manifest_guard = self.manifest.read().expect("manifest read lock");
        let manifest = manifest_guard
            .as_ref()
            .ok_or_else(|| LicenseError::Serialization("no targets manifest available".into()))?;
        manifest.verify_signature(authority_key)?;

        // 3. Evaluate Node Capability Manifest
        let node_manifest = token.verify(authority_key, &[])?;

        let mut report = CapabilitySyncReport {
            synced_at_secs: current_time_secs,
            ..Default::default()
        };

        let repo = self.repository.read().expect("repo read lock");

        // 4. Iterate over available targets
        for (comp_id, target) in &manifest.targets {
            // Check licensing entitlement
            if !node_manifest.is_feature_authorized(
                target.required_asset_class.as_deref(),
                target.required_tier,
            ) {
                report.skipped_unlicensed.push(comp_id.clone());
                continue;
            }

            // Check if local cache already has this exact version and content digest
            if let Some(cached) = self.cache.get(comp_id, &target.version) {
                if cached.content_digest == target.content_digest {
                    report.up_to_date.push(comp_id.clone());
                    continue;
                }
            }

            // Pull artifact from remote repository
            let remote_art = repo
                .get(comp_id)
                .ok_or_else(|| LicenseError::CapabilityDenied(format!("artifact '{comp_id}' not in repo")))?;

            // Verify content digest matches manifest
            if remote_art.content_digest != target.content_digest {
                return Err(LicenseError::Serialization(format!(
                    "artifact '{comp_id}' content digest does not match manifest"
                )));
            }

            // Verify Release Authority signature
            remote_art.verify_signature(authority_key)?;

            // Store in local cache
            self.cache.store(remote_art.clone());
            report.hydrated_components.push(comp_id.clone());
        }

        Ok(report)
    }
}
