//! **Owned-pair forwarding** — the stateless router tier of `docs/SCALE-OUT.md` §3
//! for the *unary* wire APIs (`PricingService` / `QuoteService` / `SurfaceService`).
//!
//! When the edge boots [`FleetTopology::Distributed`](celnet_risk_fleet::FleetTopology)
//! the backend fleet (the same [`Fleet`] the `RiskService` federation already connects
//! — reused, never a second pool of channels) partitions the firm's pricing work by
//! **currency-pair** (`docs/SCALE-OUT.md` §2: the pair is the primary partition key,
//! so every tenor/strike of a pair is co-resident on one backend). A unary request is
//! then **forwarded** to the single backend that *owns* its pair, and that backend's
//! response is returned **verbatim**:
//!
//! * `Price` → the owner of `instrument.pair`;
//! * `RequestQuote` / `AcceptQuote` / `RejectQuote` → the owner of the quote's pair
//!   (accept/reject route to the *same* owner that issued the quote — see
//!   [`super::quote`]);
//! * `GetSmile` / `MarkSurface` / `Scenario` → the owner of the request's pair, so a
//!   `MarkSurface` deposits on the owner's `surface_book` and a later `Price` for that
//!   pair routes to the same owner and sees it.
//!
//! [`FleetTopology::InProcess`] is the default and is **unchanged** — the edge prices
//! locally over its own [`CoreLink`](crate::core_link) / [`SurfaceBook`](crate::surface_book).
//!
//! # No proto / schema change
//!
//! Forwarding speaks the **same** one `celnet-proto` contract on both legs (the edge is
//! a `celnet_client::Client` to the backends and the generated service to its callers).
//! There is no federation-only message, no `schema_version` — the wire is unchanged
//! (`CLAUDE.md` rule 9). Routing reuses `celnet_router`'s HRW [`PartitionMap`] via
//! [`Fleet::owner_of_pair`], so the owned-pair forwarding and the risk fan-out agree
//! on ownership with no extra state.
//!
//! [`PartitionMap`]: celnet_router::PartitionMap

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use celnet_types::CcyPair;
use tonic::Status;

use super::risk::federate::Fleet;

/// How a unary RPC is served under the edge's resolved topology: serve **locally**
/// (the in-process default, byte-identical to the single-node edge) or **forward** to
/// the connected backend [`Fleet`] that owns the request's pair.
///
/// Mirrors the risk edge's `Serve` (Direct vs Federate) discriminator: the same
/// deploy-time seam, applied to the unary pricing/quote/surface APIs.
pub(crate) enum Serve<'a> {
    /// In-process: price locally over the edge's own core / surface book.
    Local,
    /// Distributed: forward to the backend fleet (route by the request's pair).
    Forward(&'a Fleet),
}

/// Resolve how to serve, given the edge's optional connected fleet handle. A
/// distributed edge threads `Some(fleet)`; an in-process edge holds `None`.
///
/// `None` ⇒ [`Serve::Local`] (the unchanged single-node path); `Some(fleet)` ⇒
/// [`Serve::Forward`]. There is no "distributed but unconnected" unary state: a
/// distributed edge is only ever built *with* the connected fleet at boot (the boot
/// path dials the backends eagerly so a misconfigured fleet fails at boot), so a
/// present fleet always forwards and an absent one always serves locally.
pub(crate) fn serve_mode(fleet: Option<&Arc<Fleet>>) -> Serve<'_> {
    match fleet {
        Some(fleet) => Serve::Forward(fleet),
        None => Serve::Local,
    }
}

/// Decode a wire [`celnet_proto::CcyPair`] (the routing key) into the typed
/// [`CcyPair`] owned-pair forwarding routes on. A malformed/absent pair is a client
/// error (`invalid_argument`) — a request with no decodable pair cannot be routed.
///
/// # Errors
/// [`Status::invalid_argument`] if the pair is absent or carries an invalid currency
/// code.
pub(crate) fn route_pair(pair: Option<&celnet_proto::CcyPair>) -> Result<CcyPair, Status> {
    let wire =
        pair.ok_or_else(|| Status::invalid_argument("request carries no `pair` to route on"))?;
    CcyPair::try_from(wire.clone())
        .map_err(|e| Status::invalid_argument(format!("invalid routing pair: {e}")))
}
