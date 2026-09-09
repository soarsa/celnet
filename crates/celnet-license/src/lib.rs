//! Celnet Institutional Capability Licensing, Policy Evaluation Engine, and Hardware Attestation.
//!
//! Provides cryptographically verifiable authorization tokens, hardware-rooted security descriptors,
//! content-addressed artifact distribution, and automated capability hydration.

#![forbid(unsafe_code)]

pub mod artifact;
pub mod datalog;
pub mod error;
pub mod manifest;
pub mod nhed;
pub mod repository;
pub mod token;

pub use artifact::{ArtifactRegistry, ComponentArtifact};
pub use datalog::{
    Check, Constraint, DatalogEngine, Fact, Op, PolicyCheck, PolicyConstraint, PolicyEngine,
    PolicyFact, PolicyOp, PolicyRule, PolicyTerm, Rule, Term,
};
pub use error::LicenseError;
pub use manifest::{CapabilityManifest, LicenseTier};
pub use nhed::{
    NicDriver, NodeHardwareDescriptor, NodeIdentityQuote, VectorIsa, probe_local_hardware,
};
pub use repository::{
    CapabilitySyncReport, CapabilityTargetEntry, CapabilityTargetsManifest, LocalArtifactCache,
    RemoteCapabilityClient, RepositoryTimestamp,
};
pub use token::{BiscuitToken, Block, CapabilityToken};
