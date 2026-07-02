//! The one-shot pricing service: `Price(PriceRequest) → PriceResponse`.
//!
//! The simplest edge RPC: price a single [`celnet_proto::Instrument`] against a
//! caller-supplied [`celnet_proto::MarketContext`] under a
//! [`celnet_proto::Conventions`], returning the full 13-Greek set and the resolved
//! strike. Unlike RFQ, the caller supplies the market context explicitly (this is
//! a calculation RPC, not a tradable quote), so no live-market read is needed.
//!
//! All math routes through [`crate::pricer`], so a `Price` result is identical to
//! the mid of a [`crate::services::quote`] RFQ on the same instrument and market.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use celnet_proto::pricing_service_server::PricingService;
use celnet_proto::{
    PriceRequest, PriceResponse, PriceXvaRequest, PriceXvaResponse, RatesPriceRequest,
    RatesPriceResponse,
};
use tonic::{Request, Response, Status};

use crate::pricer::{ConventionSet, PricingEngine};
use crate::rates_pricing::RatesPriceError;
use crate::readiness::ReadinessGate;
use crate::services::forward::{Serve, route_underlying, serve_mode};
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::services::risk::federate::Fleet;
use crate::surface_book::SurfaceBook;
use crate::xva_pricing::{XvaPriceError, price_xva};

/// The one-shot pricing service over the readiness gate.
///
/// In [`FleetTopology::InProcess`](celnet_risk_fleet::FleetTopology) mode (`fleet ==
/// None`) the edge prices locally over its own [`SurfaceBook`] — unchanged. In
/// [`FleetTopology::Distributed`](celnet_risk_fleet::FleetTopology) mode the edge
/// holds the connected backend [`Fleet`] and **forwards** each `Price` to the backend
/// that owns the instrument's pair, returning that backend's reply verbatim
/// (`docs/SCALE-OUT.md` §3 owned-pair forwarding).
#[derive(Debug)]
pub struct PricingEdge {
    gate: Arc<ReadinessGate>,
    surface_book: Arc<SurfaceBook>,
    /// The connected backend fleet for owned-pair forwarding; `None` ⇒ in-process
    /// (price locally). Reuses the SAME `Fleet` the risk federation connects.
    fleet: Option<Arc<Fleet>>,
}

impl PricingEdge {
    /// Construct the pricing service in the in-process topology (prices locally).
    #[must_use]
    pub fn new(gate: Arc<ReadinessGate>, surface_book: Arc<SurfaceBook>) -> Self {
        Self {
            gate,
            surface_book,
            fleet: None,
        }
    }

    /// Construct the pricing service with an optional connected backend [`Fleet`]:
    /// `Some(fleet)` ⇒ distributed (forward `Price` by the instrument's pair to its
    /// owning backend); `None` ⇒ in-process (price locally), exactly [`PricingEdge::new`].
    #[must_use]
    pub fn with_fleet(
        gate: Arc<ReadinessGate>,
        surface_book: Arc<SurfaceBook>,
        fleet: Option<Arc<Fleet>>,
    ) -> Self {
        Self {
            gate,
            surface_book,
            fleet,
        }
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

        // Distributed: forward to the backend that owns the instrument's pair and
        // return its reply verbatim. In-process: fall through to local pricing.
        if let Serve::Forward(fleet) = serve_mode(self.fleet.as_ref()) {
            let instrument = req
                .instrument
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("missing `instrument`"))?;
            let pair = route_underlying(instrument.underlying.as_ref())?;
            let (_replica, client) = fleet.owner_of_pair(pair)?;
            let mut svc =
                celnet_proto::pricing_service_client::PricingServiceClient::new(client.channel());
            return Ok(Response::new(svc.price(req).await?.into_inner()));
        }

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

        // Resolve the optional pinned `surface_version`: an honoured pin prices
        // against the marked surface's vol and is echoed on the reply; an unknown
        // pinned version is refused (the pin cannot be honoured).
        let PinnedVol {
            market: effective_market,
            echo_version,
        } = resolve_pinned_vol(
            &self.surface_book,
            req.surface_version,
            &instrument,
            &market,
        )?;

        let priced = PricingEngine::price(&instrument, &effective_market, &conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        Ok(Response::new(PriceResponse {
            request_id: req.request_id,
            greeks: Some(priced.greeks.into()),
            resolved_strike: priced.resolved_strike,
            conventions: Some(wire_conv),
            correlation_id: req.correlation_id,
            surface_version: echo_version,
            price_std_error: priced.std_error,
        }))
    }

    async fn price_rates(
        &self,
        request: Request<RatesPriceRequest>,
    ) -> Result<Response<RatesPriceResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        // Linear-rates pricing is a pure calculation against the caller-supplied
        // `CurveSet` — there is no per-pair market read to route — so every
        // replica computes the identical result; no fleet forwarding is needed.
        let result = PricingEngine::price_rates(&req).map_err(|e| match e {
            // A bootstrap failure on otherwise-valid input is an internal numeric
            // fault; every other variant is a malformed request.
            RatesPriceError::Bootstrap(_) => Status::internal(e.to_string()),
            _ => Status::invalid_argument(e.to_string()),
        })?;

        Ok(Response::new(RatesPriceResponse {
            request_id: req.request_id,
            result: Some(result),
            correlation_id: req.correlation_id,
        }))
    }

    async fn price_xva(
        &self,
        request: Request<PriceXvaRequest>,
    ) -> Result<Response<PriceXvaResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        // XVA is a pure calculation against the caller-supplied netting set and
        // survival curves — there is no per-pair market read to route — so every
        // replica computes the identical result; no fleet forwarding is needed.
        let result = price_xva(&req).map_err(|e| match e {
            // A non-finite adjustment on otherwise-valid input is an internal
            // numeric fault; every other variant is a malformed request.
            XvaPriceError::NonFiniteResult => Status::internal(e.to_string()),
            _ => Status::invalid_argument(e.to_string()),
        })?;

        Ok(Response::new(PriceXvaResponse {
            request_id: req.request_id,
            result: Some(result),
            correlation_id: req.correlation_id,
        }))
    }
}
