//! **Distributed risk federation** — the [`FleetTopology::Distributed`] branch of
//! [`RiskEdge`](super::RiskEdge): the edge becomes a *client* of the same
//! `RiskService` it serves, fanning every RPC out across N backend
//! `celnet-server` processes and fanning the partials back into the **identical**
//! answer the single-node edge would give over the union book
//! (`docs/RISK-HIERARCHY.md` §3.4, `docs/SCALE-OUT.md` §2; Phase 3).
//!
//! # Why this is exactly the single-node answer (the load-bearing reason)
//!
//! Each backend owns a **disjoint** HRW slice of the firm's facts
//! (`celnet_risk_fleet::natural_owner_of` over the `(entity, pair)` partition key,
//! so every tenor/strike of a pair is co-resident on one shard). The reduction
//! splits exactly along the additive / non-additive line `celnet-risk-fleet`
//! already proved in-process:
//!
//! * **Additive** measures (per-ccy net delta vector, gamma and the higher Greeks,
//!   vega in the numeraire, the premium, and the `(tenor × delta)` vega ladder) are
//!   a **linear functional** of the facts. Summing the wire [`AdditiveRisk`] across
//!   backends per `(dimension, group)` key is associative + commutative, so the
//!   federated sum equals the single-node collapse over the union — exact to
//!   floating-point summation order (pinned here by visiting backends in endpoint
//!   order). No book ever crosses the wire for the additive path: each backend
//!   ships only its small per-node roll-up.
//! * **Non-additive** measures (VaR / ES, FRTB-SbM spot curvature) are a **non-linear
//!   functional of the joint P&L distribution** — the VaR of a union is *not* the
//!   sum of shard VaRs (offsetting tails diversify; curvature is a `max`). The only
//!   correct distributed answer is to **gather the union of constituent positions**
//!   (each backend's `DrillRisk{include_positions}` for the node, entitlement-pruned
//!   identically at the backend so the gathered set is consistent),
//!   [`position_to_fact`](super::convert::position_to_fact) each back into a cube
//!   fact, build one [`Cube`](celnet_risk_cube::Cube), and **re-derive the measure
//!   once** over the whole set — the same `Cube::node_var_es` / `node_curvature_spot`
//!   algebra the single-node edge runs, so the result is exact by construction, not
//!   approximate. This runs **off the hot path** and only when a non-additive
//!   measure is actually requested.
//!
//! # Ownership vs reach (health-aware, never a silently-smaller firm number)
//!
//! A backend maps to a [`celnet_router::Replica`]; the federated edge holds a
//! [`celnet_router::ReplicaSet`] (one replica per endpoint, in endpoint order). The
//! `natural_owner` of an `(entity, pair)` slice is its stable home. The federation
//! fans the request to **every `Up`, connected backend** and verifies coverage
//! before trusting the union: an `Up` replica self-serves its slice; a `Down`
//! replica's slice is covered **only** by a declared healthy **hot standby** that is
//! itself an `Up` connected backend genuinely holding the re-homed slice
//! (`docs/SCALE-OUT.md` §3). This is deliberately *stricter* than raw HRW-fallback
//! routing: the router's `route` would re-home a down owner's *keys* to the next
//! healthy replica, but that fallback node does not actually hold the missing facts
//! unless they were replicated/re-homed to it — so for a *completeness* guarantee
//! only a hot standby counts. A `Down` replica with **no** such standby ⇒ the slice
//! is uncovered ⇒ [`Status::unavailable`]: the federation never returns a firm
//! number computed over a strict subset of the book (a silently-wrong, too-small
//! answer).
//!
//! # No proto / schema change
//!
//! The federated edge speaks the **same** one `celnet-proto` contract on both sides
//! (it is a `celnet_client::Client` to the backends and a `RiskService` to its own
//! callers). There is no `schema_version`, no N/N-1 negotiation, no federation-only
//! message — the wire is unchanged (`CLAUDE.md` rule 9).

use std::collections::BTreeMap;
use std::sync::Arc;

use celnet_client::Client;
use celnet_proto::{
    AdditiveRisk, AggregateRiskRequest, AggregateRiskResponse, CcyExposureLeg, DrillRiskRequest,
    DrillRiskResponse, LimitStatusRequest, LimitStatusResponse, ListPositionsRequest,
    ListPositionsResponse, RiskNode, RiskPosition, VegaLadderBucket,
};
use celnet_router::{Replica, ReplicaId, ReplicaSet};
use tonic::Status;

use super::store::{BookedPosition, PositionStore};
use super::{RiskEdge, convert};

/// The connected backend fleet behind a [`FleetTopology::Distributed`] edge: one
/// `celnet_client::Client` per endpoint (sharing its HTTP/2 channel), plus the
/// router membership that maps endpoints to replicas for health-aware reach.
///
/// Built once at edge construction ([`Fleet::connect`]); cheap to share behind an
/// [`Arc`]. Each [`Client`] is itself cheap to clone (a shared channel), so the
/// per-RPC fan-out clones handles, never re-dials.
///
/// [`FleetTopology::Distributed`]: celnet_risk_fleet::FleetTopology::Distributed
/// [`Arc`]: std::sync::Arc
#[derive(Debug)]
pub struct Fleet {
    /// One backend per declared endpoint, in endpoint order (== replica order).
    backends: Vec<Backend>,
    /// The router membership (one [`Replica`] per backend, ids `1..=N` in endpoint
    /// order). Drives the health-aware reach/coverage guard in `reachable_serving`.
    replicas: ReplicaSet,
}

/// One backend node: its endpoint, its router replica id, and the connected client.
#[derive(Debug, Clone)]
struct Backend {
    replica: ReplicaId,
    client: Client,
}

impl Fleet {
    /// Connect to every backend `endpoint` (in order) as an all-`Up` membership
    /// (replica id `i+1` for endpoint `i`). The HTTP/2 channels are established
    /// eagerly so a misconfigured fleet fails at boot, not on the first RPC.
    ///
    /// This is the boot path (`CELNET_FLEET_BACKENDS` is a flat endpoint list with no
    /// health/standby annotation, so every declared backend is `Up`). Use
    /// [`Fleet::connect_with_membership`] to express failover topologies (a `Down`
    /// replica backed by a declared hot standby) — e.g. in tests.
    ///
    /// # Errors
    /// [`Status::unavailable`] if any backend cannot be dialled, or
    /// [`Status::internal`] if the assembled membership is invalid.
    pub async fn connect(endpoints: &[String]) -> Result<Self, Status> {
        let members: Vec<(String, Replica)> = endpoints
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let id = ReplicaId(u64::try_from(i).unwrap_or(u64::MAX) + 1);
                (e.clone(), Replica::up(id))
            })
            .collect();
        Self::connect_with_membership(&members).await
    }

    /// Connect a fleet from explicit `(endpoint, [`Replica`])` members, so a caller can
    /// express a failover topology: a `Down` replica (its endpoint is **not** dialled)
    /// backed by a declared healthy standby that **is** a connected backend
    /// (`Replica::up(id).with_standby(sb)` / `.down()`). Only `Up` members' endpoints
    /// are dialled; a `Down` member contributes its replica (with its standby pin) to
    /// the membership for the reach guard, but no channel.
    ///
    /// # Errors
    /// [`Status::unavailable`] if an `Up` backend cannot be dialled, or
    /// [`Status::internal`] if the membership is invalid (duplicate id / dangling
    /// standby).
    pub async fn connect_with_membership(members: &[(String, Replica)]) -> Result<Self, Status> {
        let mut backends = Vec::new();
        let mut replicas = Vec::with_capacity(members.len());
        for (endpoint, replica) in members {
            replicas.push(*replica);
            if matches!(replica.health, celnet_router::Health::Up) {
                let url = if endpoint.contains("://") {
                    endpoint.clone()
                } else {
                    format!("http://{endpoint}")
                };
                let client = Client::connect(url).await.map_err(|e| {
                    Status::unavailable(format!("fleet backend `{endpoint}` unreachable: {e}"))
                })?;
                backends.push(Backend {
                    replica: replica.id,
                    client,
                });
            }
        }
        let replicas = ReplicaSet::new(replicas)
            .map_err(|e| Status::internal(format!("invalid fleet membership: {e}")))?;
        Ok(Self { backends, replicas })
    }

    /// The replica membership (for the reach guard and observability).
    #[must_use]
    pub fn replicas(&self) -> &ReplicaSet {
        &self.replicas
    }

    /// The number of declared backends.
    #[must_use]
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Whether the fleet has no backends.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// The set of backends to **actually fan a request to**, with a completeness guard
    /// that every declared replica's slice is covered by a reachable serving backend
    /// — so the union of fanned-to backends is the **whole** book, never a strict
    /// subset (which would be a silently-smaller, wrong firm number).
    ///
    /// The fan-out set is the de-duplicated set of **`Up`, connected backends**. The
    /// completeness guard is *stricter than raw HRW routing on purpose*: a `Down`
    /// replica's slice is considered covered **only** if it has a **declared healthy
    /// standby that is itself a connected `Up` backend** (a hot standby genuinely
    /// holding the re-homed slice). A `Down` replica with **no** such standby is
    /// **uncovered** ⇒ [`Status::unavailable`]. (Raw HRW-fallback would *route* a down
    /// owner's keys to the next healthy replica, but that fallback replica does not
    /// actually hold the missing facts unless they were re-homed/replicated to it —
    /// so for a *completeness* guarantee, only a declared hot standby that holds the
    /// slice counts. This is the documented `docs/SCALE-OUT.md` §3 hot-standby
    /// failover; HRW-fallback is a routing convenience the standby model supersedes
    /// here.) An all-down fleet, or any uncovered slice ⇒ `unavailable`.
    fn reachable_serving(&self) -> Result<Vec<&Backend>, Status> {
        let is_up_backend = |id: ReplicaId| -> bool {
            self.backends.iter().any(|b| b.replica == id)
                && self
                    .replicas
                    .get(id)
                    .is_some_and(|r| matches!(r.health, celnet_router::Health::Up))
        };

        for declared in self.replicas.replicas() {
            match declared.health {
                celnet_router::Health::Up => {
                    // Self-served (must be a connected backend — every Up member is).
                    if !self.backends.iter().any(|b| b.replica == declared.id) {
                        return Err(Status::unavailable(format!(
                            "fleet replica {} is Up but not a connected backend",
                            declared.id.0
                        )));
                    }
                }
                celnet_router::Health::Down => {
                    // Covered only by a declared healthy standby that is a connected
                    // backend genuinely holding the re-homed slice.
                    let covered = declared.standby.is_some_and(is_up_backend);
                    if !covered {
                        return Err(Status::unavailable(format!(
                            "fleet replica {} is down with no reachable hot standby \
                             holding its slice; refusing a firm number over a partial book",
                            declared.id.0
                        )));
                    }
                }
            }
        }

        // Fan-out set: every Up, connected backend (each slice's facts live on the Up
        // survivor — the primary, or the standby that holds a down replica's re-home).
        let out: Vec<&Backend> = self
            .backends
            .iter()
            .filter(|b| {
                self.replicas
                    .get(b.replica)
                    .is_some_and(|r| matches!(r.health, celnet_router::Health::Up))
            })
            .collect();
        if out.is_empty() {
            return Err(Status::unavailable(
                "no reachable (Up) fleet backend to fan out to",
            ));
        }
        Ok(out)
    }
}

impl RiskEdge {
    /// Federate [`list_positions`](RiskEdge::list_positions_impl) across the fleet:
    /// fan the SAME request to every reachable backend (entitlement-pruned
    /// identically at each backend), then **union** the entitled positions
    /// (de-duplicated by `position_id`, in id order — a position lives on exactly one
    /// shard so a duplicate id can only be the same fact).
    ///
    /// # Errors
    /// [`Status::unavailable`] if a slice is uncovered; propagates a backend status.
    pub(super) async fn federated_list_positions(
        &self,
        fleet: &Fleet,
        req: &ListPositionsRequest,
    ) -> Result<ListPositionsResponse, Status> {
        let backends = fleet.reachable_serving()?;
        let mut by_id: BTreeMap<u64, RiskPosition> = BTreeMap::new();
        for b in backends {
            let resp = list_positions_via(&b.client, req).await?;
            for p in resp.positions {
                by_id.insert(p.position_id, p);
            }
        }
        Ok(ListPositionsResponse {
            positions: by_id.into_values().collect(),
            correlation_id: req.correlation_id,
        })
    }

    /// Federate [`aggregate_risk`](RiskEdge::aggregate_risk_impl): fan the SAME
    /// request to all reachable backends, **sum** the wire [`AdditiveRisk`] per
    /// `(dimension, group)` node (linear ⇒ == single-node), and, when VaR/ES or
    /// curvature is requested, **re-gather** each node's constituents and re-derive
    /// the non-additive measures once over the union (non-linear ⇒ must not sum).
    ///
    /// # Errors
    /// [`Status::unavailable`] if a slice is uncovered; propagates a backend status;
    /// `failed_precondition` for a numeraire-rate failure during re-derivation.
    pub(super) async fn federated_aggregate_risk(
        &self,
        fleet: &Fleet,
        req: &AggregateRiskRequest,
    ) -> Result<AggregateRiskResponse, Status> {
        let backends = fleet.reachable_serving()?;

        // 1. Additive fan-in: sum the wire AdditiveRisk per (dimension, group),
        //    preserving first-seen node order so the response order is deterministic.
        let mut order: Vec<u64> = Vec::new();
        let mut summed: BTreeMap<u64, RiskNode> = BTreeMap::new();
        let mut numeraire_code = String::new();
        for b in &backends {
            let resp = aggregate_via(&b.client, req).await?;
            numeraire_code = resp.numeraire;
            for node in resp.nodes {
                match summed.get_mut(&node.group) {
                    Some(acc) => merge_node_additive(acc, &node),
                    None => {
                        order.push(node.group);
                        summed.insert(node.group, node);
                    }
                }
            }
        }

        // 2. Non-additive re-gather (only when requested): gather each node's
        //    constituents from all backends, rebuild a cube, re-derive once.
        let evaluates_nonadditive =
            !req.var_spot_shocks.is_empty() || req.curvature_risk_weight > 0.0;
        if evaluates_nonadditive {
            for group in &order {
                let node = summed.get_mut(group).expect("group inserted in order");
                let nonadditive = self
                    .regather_node_nonadditive(fleet, &backends, req, node.dimension, *group)
                    .await?;
                node.nonadditive = Some(nonadditive);
            }
        }

        let nodes = order
            .into_iter()
            .map(|g| summed.remove(&g).expect("group inserted in order"))
            .collect();
        Ok(AggregateRiskResponse {
            dimension: req.dimension,
            numeraire: numeraire_code,
            nodes,
            correlation_id: req.correlation_id,
        })
    }

    /// Federate [`drill_risk`](RiskEdge::drill_risk_impl): fan the SAME drill to all
    /// reachable backends, then **merge** the per-child additive measures (summed per
    /// child group, exactly as the aggregate), **re-gather** each child's
    /// non-additive measures over the union (when shocks/curvature ride on the
    /// request — drill children carry the request's `vega_pillars`; non-additive is
    /// re-derived only if the original aggregate path requested it, which the drill
    /// mirrors via the parent request fields), and **union** the contributing
    /// positions.
    ///
    /// # Errors
    /// [`Status::unavailable`] if a slice is uncovered; propagates a backend status.
    pub(super) async fn federated_drill_risk(
        &self,
        fleet: &Fleet,
        req: &DrillRiskRequest,
    ) -> Result<DrillRiskResponse, Status> {
        let backends = fleet.reachable_serving()?;
        let mut child_order: Vec<u64> = Vec::new();
        let mut children: BTreeMap<u64, RiskNode> = BTreeMap::new();
        let mut positions: BTreeMap<u64, RiskPosition> = BTreeMap::new();
        let node_scope = req.node;
        for b in &backends {
            let resp = drill_via(&b.client, req).await?;
            for child in resp.children {
                match children.get_mut(&child.group) {
                    Some(acc) => merge_node_additive(acc, &child),
                    None => {
                        child_order.push(child.group);
                        children.insert(child.group, child);
                    }
                }
            }
            for p in resp.positions {
                positions.insert(p.position_id, p);
            }
        }
        let children = child_order
            .into_iter()
            .map(|g| children.remove(&g).expect("child inserted in order"))
            .collect();
        Ok(DrillRiskResponse {
            node: node_scope,
            children,
            positions: positions.into_values().collect(),
            correlation_id: req.correlation_id,
        })
    }

    /// Federate [`limit_status`](RiskEdge::limit_status_impl): the limit tree is
    /// **firm-level policy configured at the aggregating (this) edge**, not per-shard,
    /// so the federation re-derives utilization over the **gathered union** of the
    /// scope's constituents against this edge's own limit tree — the single-node
    /// algebra, exactly. It gathers the scope's entitled positions from every reachable
    /// backend (`ListPositions` scoped to the limit scope), rebuilds them into this
    /// edge's [`PositionStore`] (which carries the firm hierarchy + limit tree), and
    /// delegates to the unchanged single-node [`limit_status_impl`](RiskEdge::limit_status_impl).
    ///
    /// # Errors
    /// [`Status::unavailable`] if a slice is uncovered; `invalid_argument` for a
    /// malformed scope; `failed_precondition` for a numeraire-rate failure.
    pub(super) async fn federated_limit_status(
        &self,
        fleet: &Fleet,
        req: &LimitStatusRequest,
    ) -> Result<LimitStatusResponse, Status> {
        let backends = fleet.reachable_serving()?;
        // Gather the scope's entitled positions across the fleet (the limit scope is
        // the listing scope: ListPositions prunes by principal AND scope at the backend).
        let list_req = ListPositionsRequest {
            scope: req.scope,
            principal: req.principal.clone(),
            correlation_id: req.correlation_id,
        };
        let union = self.gather_union(&backends, &list_req).await?;
        // Rebuild the union into a transient store that carries THIS edge's firm
        // hierarchy + limit tree, then run the unchanged single-node limit algebra.
        let staged = self.stage_union(&union)?;
        let staged_edge = RiskEdge::new(staged, Arc::clone(&self.gate));
        staged_edge.limit_status_impl(req)
    }

    /// Re-derive one node's non-additive measures over the **gathered union** of its
    /// constituents: drill the node on every backend with `include_positions`,
    /// `position_to_fact` each wire position into this edge's hierarchy, stage them in
    /// a transient store, and run the unchanged single-node aggregate over just that
    /// node's scope — returning the re-derived [`NonAdditiveRisk`](celnet_proto::NonAdditiveRisk).
    async fn regather_node_nonadditive(
        &self,
        _fleet: &Fleet,
        backends: &[&Backend],
        req: &AggregateRiskRequest,
        dimension: i32,
        group: u64,
    ) -> Result<celnet_proto::NonAdditiveRisk, Status> {
        // Gather this node's constituents from every backend, entitlement-pruned
        // identically at each backend (same principal), via ListPositions scoped to
        // the node. (DrillRisk{include_positions} would equally work; ListPositions is
        // the leaner gather and prunes by the same principal + scope.)
        let scope = celnet_proto::RiskScope {
            dimension,
            value: group,
        };
        let list_req = ListPositionsRequest {
            scope: Some(scope),
            principal: req.principal.clone(),
            correlation_id: req.correlation_id,
        };
        let union = self.gather_union(backends, &list_req).await?;
        let staged = self.stage_union(&union)?;
        let staged_edge = RiskEdge::new(staged, Arc::clone(&self.gate));
        // Run the single-node aggregate over just this node's scope (firm apex over
        // the staged constituents == this one node), with the SAME shocks/curvature,
        // and lift the re-derived non-additive measures off the (single) result node.
        let scoped_req = AggregateRiskRequest {
            dimension: celnet_proto::RiskDimension::Firm as i32,
            numeraire: req.numeraire.clone(),
            principal: None, // already pruned at the backends; the staged set is final
            scope: None,
            vega_pillars: req.vega_pillars.clone(),
            var_spot_shocks: req.var_spot_shocks.clone(),
            var_alpha: req.var_alpha,
            curvature_risk_weight: req.curvature_risk_weight,
            correlation_id: req.correlation_id,
        };
        let resp = staged_edge.aggregate_risk_impl(&scoped_req)?;
        let node = resp
            .nodes
            .into_iter()
            .next()
            .ok_or_else(|| Status::internal("re-gather produced no firm node"))?;
        node.nonadditive
            .ok_or_else(|| Status::internal("re-gathered firm node missing non-additive measures"))
    }

    /// Gather the union of entitled positions for a listing request across a set of
    /// backends, de-duplicated by `position_id` in id order.
    async fn gather_union(
        &self,
        backends: &[&Backend],
        req: &ListPositionsRequest,
    ) -> Result<Vec<RiskPosition>, Status> {
        let mut by_id: BTreeMap<u64, RiskPosition> = BTreeMap::new();
        for b in backends {
            let resp = list_positions_via(&b.client, req).await?;
            for p in resp.positions {
                by_id.insert(p.position_id, p);
            }
        }
        Ok(by_id.into_values().collect())
    }

    /// Stage a gathered union of wire positions into a transient [`PositionStore`]
    /// that carries **this edge's firm hierarchy + limit tree** (so a re-derivation /
    /// limit check over the union sees the firm-consistent org placement and policy).
    /// Each wire position is reconstructed via
    /// [`position_to_fact`](super::convert::position_to_fact) and upserted under its
    /// own [`FactKey`].
    fn stage_union(&self, union: &[RiskPosition]) -> Result<Arc<PositionStore>, Status> {
        // Start from this edge's hierarchy + limit tree (the firm-consistent config
        // the federated edge was set up with), so group resolution and limit scopes
        // match the single-node oracle.
        let store = Arc::new(self.store.fork_config());
        for wire in union {
            let fact = convert::position_to_fact(wire)?;
            let booked = BookedPosition {
                position_id: wire.position_id,
                pair: fact.key.ccy_pair,
                option: fact.measure.position.option,
                notional_base: fact.measure.position.notional_base,
                inputs: fact.measure.position.inputs,
                quoted_delta: fact.measure.position.quoted_delta,
                premium_style: fact.measure.position.premium_style,
                surface_version: fact.surface_version,
            };
            store.upsert(booked, fact.key, wire.attribution.clone())?;
        }
        Ok(store)
    }
}

/// Sum one wire [`AdditiveRisk`] into another, in place: every scalar adds; the
/// per-ccy delta vector sums per currency; the vega ladder sums per pillar. This is
/// the wire-space mirror of `celnet_risk_cube::NodeAggregate::merge_additive` and is
/// exact because every additive measure is a linear functional of the facts.
fn merge_additive(acc: &mut AdditiveRisk, other: &AdditiveRisk) {
    acc.delta_numeraire += other.delta_numeraire;
    acc.gamma += other.gamma;
    acc.vega_numeraire += other.vega_numeraire;
    acc.theta += other.theta;
    acc.vanna += other.vanna;
    acc.volga += other.volga;
    acc.charm += other.charm;
    acc.speed += other.speed;
    acc.zomma += other.zomma;
    acc.color += other.color;
    acc.premium_numeraire += other.premium_numeraire;
    merge_delta_vector(&mut acc.delta_vector, &other.delta_vector);
    merge_vega_ladder(&mut acc.vega_ladder, &other.vega_ladder);
}

/// Sum the per-ccy delta-exposure legs of `other` into `acc`, matching by currency
/// (a new currency is appended, preserving first-seen order).
fn merge_delta_vector(acc: &mut Vec<CcyExposureLeg>, other: &[CcyExposureLeg]) {
    for leg in other {
        match acc.iter_mut().find(|l| l.ccy == leg.ccy) {
            Some(slot) => slot.amount += leg.amount,
            None => acc.push(leg.clone()),
        }
    }
}

/// Sum the vega-ladder buckets of `other` into `acc`, matching by pillar (a new
/// pillar is appended, preserving first-seen order).
fn merge_vega_ladder(acc: &mut Vec<VegaLadderBucket>, other: &[VegaLadderBucket]) {
    for bucket in other {
        match acc.iter_mut().find(|b| b.pillar == bucket.pillar) {
            Some(slot) => slot.vega += bucket.vega,
            None => acc.push(*bucket),
        }
    }
}

/// Merge one wire [`RiskNode`]'s additive measures into another (same `(dimension,
/// group)` key). Position counts add; the non-additive field is left to the
/// re-gather pass (summing it would be wrong).
fn merge_node_additive(acc: &mut RiskNode, other: &RiskNode) {
    acc.position_count += other.position_count;
    match (&mut acc.additive, &other.additive) {
        (Some(a), Some(o)) => merge_additive(a, o),
        (None, Some(o)) => acc.additive = Some(o.clone()),
        _ => {}
    }
}

// --- the four backend RPCs, dialled over a connected client's channel -----------
//
// The federated edge speaks the SAME contract to its backends; these thin wrappers
// build the generated tonic clients off the shared channel per call (cheap — a clone
// of the HTTP/2 channel) and return the raw wire response for the reducer to combine.

async fn list_positions_via(
    client: &Client,
    req: &ListPositionsRequest,
) -> Result<ListPositionsResponse, Status> {
    let mut svc = celnet_proto::risk_service_client::RiskServiceClient::new(client.channel());
    Ok(svc.list_positions(req.clone()).await?.into_inner())
}

async fn aggregate_via(
    client: &Client,
    req: &AggregateRiskRequest,
) -> Result<AggregateRiskResponse, Status> {
    let mut svc = celnet_proto::risk_service_client::RiskServiceClient::new(client.channel());
    Ok(svc.aggregate_risk(req.clone()).await?.into_inner())
}

async fn drill_via(client: &Client, req: &DrillRiskRequest) -> Result<DrillRiskResponse, Status> {
    let mut svc = celnet_proto::risk_service_client::RiskServiceClient::new(client.channel());
    Ok(svc.drill_risk(req.clone()).await?.into_inner())
}
