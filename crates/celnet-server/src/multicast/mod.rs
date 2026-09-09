//! Jasper Fair Multicast Proxy Tree (SOTA Scalability Phase 5).
//!
//! Provides synchronized microsecond playout release deadlines and tree hedging ($H=2$)
//! to eliminate last-look latency arbitrage across streaming counterparties.

pub mod jasper;

pub use jasper::{
    DeliveryRecord, EdgeProxy, FairnessReport, JasperConfig, JasperFrame, JasperMulticastTree,
    SubscriberId,
};
