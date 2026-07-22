//! `LiquidityFeedService` — the backend LP-quote ingest (D3).
//!
//! An inbound liquidity provider (the `lp-sim` binary today, any FIX/API adapter
//! tomorrow) opens a client-streaming `LpFeed`, pushes its live per-instrument
//! two-ways as [`LpQuote`]s, and half-closes to receive an [`LpFeedAck`]. Each push
//! is routed by the shared [`AggregationHub`] into every enabled aggregated book
//! that lists the pushing LP as a member and whose scope admits the instrument;
//! the consolidated composite then surfaces to GUI subscribers over
//! `StreamService.StreamSession`.
//!
//! This is a **machine-to-machine** service — an LP speaks gRPC directly — so it
//! has no WS mirror. Like every edge service it passes the [`ReadinessGate`], so a
//! starting / draining instance refuses new feed sessions with `UNAVAILABLE`
//! instead of accepting quotes it may drop mid-cutover.

use std::sync::Arc;

use celnet_proto::liquidity_feed_service_server::LiquidityFeedService;
use celnet_proto::{LpFeedAck, LpQuote};
use futures_util::StreamExt;
use tonic::{Request, Response, Status, Streaming};

use crate::readiness::ReadinessGate;
use crate::services::aggregation::AggregationHub;

/// The `LiquidityFeedService` edge: routes pushed LP quotes into the shared
/// aggregation hub.
pub struct LiquidityFeedEdge {
    hub: Arc<AggregationHub>,
    gate: Arc<ReadinessGate>,
}

impl LiquidityFeedEdge {
    /// Construct the ingest edge over the shared aggregation hub + readiness gate.
    #[must_use]
    pub fn new(hub: Arc<AggregationHub>, gate: Arc<ReadinessGate>) -> Self {
        Self { hub, gate }
    }
}

#[tonic::async_trait]
impl LiquidityFeedService for LiquidityFeedEdge {
    async fn lp_feed(
        &self,
        request: Request<Streaming<LpQuote>>,
    ) -> Result<Response<LpFeedAck>, Status> {
        // Refuse a new feed session while starting / draining (bumps the in-flight
        // drain counter for the lifetime of the guard).
        let _guard = self.gate.enter();
        if !self.gate.is_ready() {
            return Err(Status::unavailable("edge is starting or draining"));
        }

        let mut inbound = request.into_inner();
        let mut accepted: u64 = 0;
        while let Some(quote) = inbound.next().await {
            let quote: LpQuote = quote?;
            if self.hub.ingest(&quote) {
                accepted += 1;
            }
        }
        Ok(Response::new(LpFeedAck { accepted }))
    }
}
