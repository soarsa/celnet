//! Celnet Zero-Loss Upgrade Protocol (ZLUP) & Bit-Exact Twin Comparator.
//!
//! Provides the 5-Stage Zero-Loss Upgrade Orchestrator:
//! 1. Stage 1: Pre-Flight Shadow Boot (non-voting replication follower)
//! 2. Stage 2: State Catch-Up & Bit-Exact IEEE-754 Twin Comparator
//! 3. Stage 3: Raft §6 Dynamic Joint Consensus (C_old,new promotion)
//! 4. Stage 4: Atomic Ingress Socket Redirection (zero packet loss / 0 TCP drops)
//! 5. Stage 5: Clean Drain & Decommission (graceful teardown of retired instance)

#![forbid(unsafe_code)]

pub mod error;
pub mod ingress;
pub mod orchestrator;
pub mod scale;
pub mod twin;

pub use error::UpgradeError;
pub use ingress::{AtomicIngressRouter, IngressRequestGuard};
pub use orchestrator::{UpgradeOrchestrator, UpgradeStage};
pub use scale::{ChaosEngine, ClusterScaleManager, ScaleReport};
pub use twin::{BitExactTwinComparator, TwinVerificationReport};
