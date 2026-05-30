//! The `tonic` gRPC service implementation for the Celnet pricing edge.
//!
//! Implements the generated [`crate::proto::pricing_edge_server::PricingEdge`]
//! trait. Each RPC:
//!
//! 1. enters the [`ReadinessGate`] (bumping the in-flight counter for the
//!    graceful-drain barrier; the guard is released on return, even on error);
//! 2. rejects the call with `UNAVAILABLE` if the gate is not ready (so a draining
//!    or still-starting instance never serves new work — the `/readyz` contract);
//! 3. decodes the request into the domain vocabulary, routes it through the
//!    [`CoreLink`] (vanilla over the hot ring; surface / exotic over the control
//!    plane), and encodes the result back onto the wire.
//!
//! The service holds only `Arc` handles, so the generated server is cheaply
//! cloneable per connection.
//!
//! Every RPC returns `Result<Response<…>, tonic::Status>` — the signature the
//! generated `tonic` service trait mandates. `tonic::Status` is a large error
//! type by construction (it carries gRPC metadata, a message, and details), so
//! `clippy::result_large_err` fires on every gRPC handler. The shape is fixed by
//! the framework's trait and cannot be boxed without violating it, so the lint is
//! allowed for this module only, with this rationale.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use celnet_types::{DeltaConvention, OptionType, VanillaInputs};
use tonic::{Request, Response, Status};

use crate::core_link::{BarrierTopology, CoreLink, ExoticQuery, SurfaceQuery};
use crate::proto;
use crate::readiness::ReadinessGate;

/// The gRPC pricing-edge service.
///
/// Wraps the shared [`CoreLink`] (the async⇄core bridge) and the
/// [`ReadinessGate`] (the `/readyz` + drain state). Constructed by
/// [`crate::Edge::start`] and registered on the `tonic` server.
#[derive(Debug)]
pub struct PricingEdgeService {
    link: Arc<CoreLink>,
    gate: Arc<ReadinessGate>,
}

impl PricingEdgeService {
    /// Construct the service over a core bridge and a readiness gate.
    #[must_use]
    pub fn new(link: Arc<CoreLink>, gate: Arc<ReadinessGate>) -> Self {
        Self { link, gate }
    }

    /// Reject the call if the edge is not accepting traffic (starting/draining).
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

// ---- enum / message decoders ------------------------------------------------

/// Decode a proto `OptionType` tag into the domain enum.
fn decode_option_type(tag: i32) -> Result<OptionType, Status> {
    match proto::OptionType::try_from(tag) {
        Ok(proto::OptionType::Call) => Ok(OptionType::Call),
        Ok(proto::OptionType::Put) => Ok(OptionType::Put),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown option_type tag {tag}"
        ))),
    }
}

/// Decode a proto `DeltaConvention` tag into the domain enum.
fn decode_delta_convention(tag: i32) -> Result<DeltaConvention, Status> {
    match proto::DeltaConvention::try_from(tag) {
        Ok(proto::DeltaConvention::SpotUnadjusted) => Ok(DeltaConvention::SpotUnadjusted),
        Ok(proto::DeltaConvention::ForwardUnadjusted) => Ok(DeltaConvention::ForwardUnadjusted),
        Ok(proto::DeltaConvention::SpotPremiumAdjusted) => Ok(DeltaConvention::SpotPremiumAdjusted),
        Ok(proto::DeltaConvention::ForwardPremiumAdjusted) => {
            Ok(DeltaConvention::ForwardPremiumAdjusted)
        }
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown delta_convention tag {tag}"
        ))),
    }
}

/// Decode a proto `BarrierKind` tag into the server's [`BarrierTopology`].
fn decode_barrier_topology(tag: i32) -> Result<BarrierTopology, Status> {
    match proto::BarrierKind::try_from(tag) {
        Ok(proto::BarrierKind::DownAndOut) => Ok(BarrierTopology::DownAndOut),
        Ok(proto::BarrierKind::UpAndOut) => Ok(BarrierTopology::UpAndOut),
        Ok(proto::BarrierKind::DownAndIn) => Ok(BarrierTopology::DownAndIn),
        Ok(proto::BarrierKind::UpAndIn) => Ok(BarrierTopology::UpAndIn),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown barrier_kind tag {tag}"
        ))),
    }
}

/// Decode the proto Garman-Kohlhagen inputs into the domain DTO.
fn decode_inputs(inputs: Option<proto::VanillaInputs>) -> Result<VanillaInputs, Status> {
    let i = inputs.ok_or_else(|| Status::invalid_argument("missing `inputs`"))?;
    Ok(VanillaInputs::new(
        i.spot, i.strike, i.vol, i.t, i.r_dom, i.r_for,
    ))
}

/// Encode the domain Greek set onto the wire.
fn encode_greeks(g: &celnet_types::Greeks) -> proto::Greeks {
    proto::Greeks {
        price: g.price,
        delta_spot: g.delta_spot,
        delta_forward: g.delta_forward,
        gamma: g.gamma,
        vega: g.vega,
        theta: g.theta,
        rho_dom: g.rho_dom,
        rho_for: g.rho_for,
        vanna: g.vanna,
        volga: g.volga,
        charm: g.charm,
        speed: g.speed,
        zomma: g.zomma,
        color: g.color,
    }
}

#[tonic::async_trait]
impl proto::pricing_edge_server::PricingEdge for PricingEdgeService {
    async fn price_vanilla(
        &self,
        request: Request<proto::PriceVanillaRequest>,
    ) -> Result<Response<proto::PriceVanillaResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        let option_type = decode_option_type(req.option_type)?;
        let delta_conv = decode_delta_convention(req.delta_convention)?;
        let inputs = decode_inputs(req.inputs)?;

        // Hot path: price the requested strike on the pricing core. The core
        // forms its own GK inputs from the *published* market state and the smile
        // vol, so the response is the engine's price at the live smile. We also
        // resolve the convention-specific delta against the *request's* inputs
        // (the client-supplied vol), using the closed-form vanilla math.
        let resp = self
            .link
            .price(option_type, inputs.strike)
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;

        let delta_convention_value =
            celnet_vanilla::convention_delta(delta_conv, option_type, &inputs);

        Ok(Response::new(proto::PriceVanillaResponse {
            request_id: req.request_id,
            greeks: Some(encode_greeks(&resp.greeks)),
            delta_convention_value,
        }))
    }

    async fn surface_vol(
        &self,
        request: Request<proto::SurfaceVolRequest>,
    ) -> Result<Response<proto::SurfaceVolResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        let vol = self
            .link
            .surface_vol(SurfaceQuery {
                strike: req.strike,
                tenor_years: req.tenor_years,
            })
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;

        Ok(Response::new(proto::SurfaceVolResponse {
            request_id: req.request_id,
            vol: vol.vol,
            forward: vol.forward,
        }))
    }

    async fn price_barrier(
        &self,
        request: Request<proto::PriceBarrierRequest>,
    ) -> Result<Response<proto::PriceBarrierResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        let option_type = decode_option_type(req.option_type)?;
        let topology = decode_barrier_topology(req.barrier_kind)?;
        let inputs = decode_inputs(req.inputs)?;

        let price = self
            .link
            .price_barrier(ExoticQuery {
                option_type,
                inputs,
                topology,
                barrier: req.barrier,
                rebate: req.rebate,
            })
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;

        Ok(Response::new(proto::PriceBarrierResponse {
            request_id: req.request_id,
            price,
        }))
    }

    async fn readiness(
        &self,
        _request: Request<proto::ReadinessRequest>,
    ) -> Result<Response<proto::ReadinessResponse>, Status> {
        // The readiness probe is never gated on readiness — it *reports* it.
        let state = match self.gate.state() {
            crate::readiness::ServiceState::Starting => proto::ServiceState::Starting,
            crate::readiness::ServiceState::Ready => proto::ServiceState::Ready,
            crate::readiness::ServiceState::Draining => proto::ServiceState::Draining,
        };
        Ok(Response::new(proto::ReadinessResponse {
            state: state as i32,
            ready: self.gate.is_ready(),
            in_flight: self.gate.in_flight(),
        }))
    }
}
