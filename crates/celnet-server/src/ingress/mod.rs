//! High-throughput ingress queueing tier (SOTA Scalability Phase 5).
//!
//! Provides strict head-drop eviction and Controlled Delay (CoDel) queue management
//! to neutralize bufferbloat and Coordinated Omission.

pub mod codel;
pub mod head_drop;

pub use codel::{CoDelConfig, CoDelQueue, SojournLatencyStats};
pub use head_drop::{HeadDropQueue, HeadDropStats, HeadDropStatsSnapshot};
