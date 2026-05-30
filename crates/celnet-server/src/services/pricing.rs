//! The one-shot pricing service: `Price(PriceRequest) → PriceResponse`.
//!
//! The simplest edge RPC: price a single [`celnet_proto::Instrument`] against a
//! caller-supplied [`celnet_proto::MarketContext`] under a
//! [`celnet_proto::Conventions`], returning the full 14-Greek set and the resolved
//! strike. Unlike RFQ, the caller supplies the market context explicitly (this is
//! a calculation RPC, not a tradable quote), so no live-market read is needed.
//!
//! All math routes through [`crate::pricer`], so a `Price` result is identical to
//! the mid of a [`crate::services::quote`] RFQ on the same instrument and market.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use celnet_proto::pricing_service_server::PricingService;
use celnet_proto::{PriceRequest, PriceResponse};
use tonic::{Request, Response, Status};

use crate::pricer::{ConventionSet, price_instrument};
use crate::readiness::ReadinessGate;

/// The one-shot pricing service over the readiness gate.
#[derive(Debug)]
pub struct PricingEdge {
    gate: Arc<ReadinessGate>,
}

impl PricingEdge {
    /// Construct the pricing service.
    #[must_use]
    pub fn new(gate: Arc<ReadinessGate>) -> Self {
        Self { gate }
    }

    fn require_ready(&self) -> Result<(), Status> {
        if self.gate.is_ready() {
            Ok(())
        } else {
            Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ))
        }
    }
}

#[tonic::async_trait]
impl PricingService for PricingEdge {
    async fn price(
        &self,
        request: Request<PriceRequest>,
    ) -> Result<Response<PriceResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        let instrument = req
            .instrument
            .ok_or_else(|| Status::invalid_argument("missing `instrument`"))?;
        let market = req
            .market
            .ok_or_else(|| Status::invalid_argument("missing `market`"))?;
        let wire_conv = req
            .conventions
            .ok_or_else(|| Status::invalid_argument("missing `conventions`"))?;
        let conv = ConventionSet::decode(&wire_conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let priced = price_instrument(&instrument, &market, &conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        Ok(Response::new(PriceResponse {
            request_id: req.request_id,
            greeks: Some(priced.greeks.into()),
            resolved_strike: priced.resolved_strike,
            conventions: Some(wire_conv),
        }))
    }
}
