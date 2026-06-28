//! The `RiskService` edge: firm-scale hierarchical risk computed **server-side**
//! over the live position book, behind the one `celnet-proto` contract.
//!
//! A client (GUI / SDK / Excel) never loops positions and sums — it lists
//! positions, asks for a rolled-up node tree over an org dimension, drills a node to
//! its constituents, and reads limit utilization. Aggregation, common-numeraire
//! conversion, non-additive re-derivation, entitlement pruning and limit RAG are all
//! the server's job here (`docs/RISK-HIERARCHY.md`; `docs/INTERFACES.md` §"Phase-2
//! contract: RiskService"). This closes the client-side-aggregation parity
//! violation (the GUI Book view looping `transport.scenario` per position).
//!
//! # Composition
//!
//! * [`store`] — the shared live position book (`celnet-risk-cube` facts + the org
//!   hierarchy + the attribution interner + the limit tree), populated by the
//!   click-to-trade booking path and read by every RPC here.
//! * [`convert`] — wire ↔ domain mapping (one place, used by both gRPC and WS).
//! * [`aggregate`] — the pure aggregation engine (prune → group-by → numeraire
//!   collapse → non-additive re-derivation).
//!
//! Every RPC enters the readiness gate (bumping the drain counter) and the
//! aggregation runs off a lock-free snapshot, strictly off the pinned pricing hot
//! path (RH §3.5).

#![allow(clippy::result_large_err)]

pub mod aggregate;
pub mod convert;
pub mod federate;
pub mod store;

use std::sync::Arc;

use celnet_limits::{
    Enforcement, LimitScope, NonAdditiveExposure, RagStatus, Utilization, check_scope,
};
use celnet_proto::risk_service_server::RiskService;
use celnet_proto::{
    AggregateRatesRiskRequest, AggregateRatesRiskResponse, AggregateRiskRequest,
    AggregateRiskResponse, BookRatesPositionRequest, BookRatesPositionResponse, DrillRiskRequest,
    DrillRiskResponse, LimitStatusRequest, LimitStatusResponse, LimitUtilization,
    ListPositionsRequest, ListPositionsResponse, ListRatesPositionsRequest,
    ListRatesPositionsResponse, RiskScope,
};
use celnet_risk_cube::{
    BookId, DeskId, DimensionId, EntityId, LocationId, NetGreeks, NodeAggregate, TraderId,
    VegaLadder,
};
use celnet_risk_fleet::FleetTopology;
use celnet_risk_normalize::SpotResolver as _;
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::readiness::ReadinessGate;
use crate::services::access::{
    DeskScope, RequiredAuthority, ResolvedCaller, authorize_caller, resolve_caller,
};
use crate::services::sessions::SessionRegistry;
use aggregate::{
    NonAdditiveConfig, aggregate_nodes, cube_from_facts, entitled_cube, entitled_facts_for_scope,
};
use celnet_entitlements::{Action, AssetClass};
use convert::WireResolver;
use store::PositionStore;

/// The `RiskService` edge over the shared live position book + readiness gate.
///
/// # Fleet topology (deploy-time seam)
///
/// The edge carries the resolved [`FleetTopology`] (`celnet-risk-fleet`) it was
/// booted under — the deploy-time choice of whether firm risk aggregates **in this
/// process** (every logical shard co-resident; the validated single-node algebra) or
/// would be **fanned out across physical shards** reached via backend endpoints. This
/// mirrors the [`crate::deployment`-style](celnet_integration) edge-adapter seam:
/// the topology is a runtime tag chosen at boot, not a contract concern (no proto
/// change, no `schema_version` — `CLAUDE.md` rule 9).
///
/// For [`FleetTopology::InProcess`] (the default, and the only path Phase 2 serves)
/// the edge behaves **exactly** as the single-node aggregation always has: every RPC
/// runs the direct prune → group-by → numeraire-collapse → non-additive-re-derive
/// path over the live store snapshot, byte-for-byte unchanged. The seam's in-process
/// reducer would re-sum the same constituents in HRW-shard order, which differs from
/// the store's fact-insertion order in the last floating-point bit; rather than
/// perturb a single byte of the established output, the in-process edge keeps the
/// direct path and the fleet seam stays the validated reconciliation oracle
/// (`celnet-risk-fleet` proves the two agree to summation-order tolerance).
///
/// [`FleetTopology::Distributed`] (Phase 3) makes the edge a **client of the same
/// `RiskService` it serves**: at construction it connects a [`federate::Fleet`] of
/// `celnet_client::Client`s (one per backend endpoint) and every RPC then **fans out**
/// to the backends and **fans in** the partials into the identical answer the
/// single-node edge would give over the union book — additive measures by linear
/// wire-sum, non-additive by re-gathering constituents and re-deriving once
/// (`docs/RISK-HIERARCHY.md` §3.4; see [`federate`]). A distributed edge constructed
/// **without** a connected fleet (the sync [`RiskEdge::with_topology`], used where the
/// topology is only inspected) fails loudly with `unavailable` rather than silently
/// degrading — it never serves a single-node answer it cannot vouch for. No proto /
/// `schema_version` change: the wire is the same on both sides (`CLAUDE.md` rule 9).
#[derive(Debug)]
pub struct RiskEdge {
    store: Arc<PositionStore>,
    gate: Arc<ReadinessGate>,
    topology: FleetTopology,
    /// The connected backend fleet for a [`FleetTopology::Distributed`] edge (built at
    /// construction via [`RiskEdge::with_topology_connected`]); `None` for in-process
    /// or a not-yet-connected distributed edge.
    fleet: Option<Arc<federate::Fleet>>,
    /// The live session registry the entitlement boundary validates `session_token`
    /// against. The constructors default to a fresh empty registry (consulted only
    /// when a request actually presents a token, so the principal-only tests never
    /// touch it); the boot path overrides it with the edge-wide registry via
    /// [`RiskEdge::with_sessions`] so every front shares one authentication state.
    sessions: Arc<SessionRegistry>,
    /// The shared **linear-rates position book** the `BookRatesPosition` /
    /// `ListRatesPositions` RPCs write/read (and the dealer-quoting
    /// [`RfqDeskService`](crate::services::desk) books accepted deals into). The
    /// constructors default to a fresh empty store; the boot path overrides it via
    /// [`RiskEdge::with_rates_store`] so the desk edge and the Book workspace read one
    /// coherent rates book.
    rates: Arc<crate::services::rates_book::RatesPositionStore>,
}

/// A fresh, empty [`SessionRegistry`] the constructors default to. It is only ever
/// consulted when a request presents a `session_token`, so a principal-only edge
/// (and every principal-only test) never touches it; the boot path replaces it with
/// the edge-wide registry via [`RiskEdge::with_sessions`].
fn default_sessions() -> Arc<SessionRegistry> {
    Arc::new(SessionRegistry::new(Clock::system()))
}

/// A fresh, empty rates position book the constructors default to (overridden at boot
/// via [`RiskEdge::with_rates_store`] so the desk edge shares the same instance).
fn default_rates_store() -> Arc<crate::services::rates_book::RatesPositionStore> {
    Arc::new(crate::services::rates_book::RatesPositionStore::new())
}

impl RiskEdge {
    /// Construct the risk edge over the shared position book and readiness gate, in
    /// the **in-process** fleet topology (the default single-node aggregation).
    #[must_use]
    pub fn new(store: Arc<PositionStore>, gate: Arc<ReadinessGate>) -> Self {
        Self::with_topology(store, gate, FleetTopology::InProcess)
    }

    /// Construct the risk edge under an explicit [`FleetTopology`] (the deploy-time
    /// fleet seam), **without** connecting any backend fleet. [`RiskEdge::new`] is the
    /// [`FleetTopology::InProcess`] case. A [`FleetTopology::Distributed`] edge built
    /// this way has no fleet and will fail every RPC with `unavailable` until it is
    /// connected; use [`RiskEdge::with_topology_connected`] at boot to dial the
    /// backends.
    #[must_use]
    pub fn with_topology(
        store: Arc<PositionStore>,
        gate: Arc<ReadinessGate>,
        topology: FleetTopology,
    ) -> Self {
        Self {
            store,
            gate,
            topology,
            fleet: None,
            sessions: default_sessions(),
            rates: default_rates_store(),
        }
    }

    /// Construct the risk edge under an explicit [`FleetTopology`], connecting the
    /// backend [`federate::Fleet`] for a [`FleetTopology::Distributed`] edge so the
    /// federation can fan out. For [`FleetTopology::InProcess`] this is exactly
    /// [`RiskEdge::with_topology`] (no fleet). The HTTP/2 channels are dialled eagerly,
    /// so a misconfigured fleet fails at boot, not on the first RPC.
    ///
    /// # Errors
    /// [`Status::unavailable`] if a backend endpoint cannot be dialled.
    pub async fn with_topology_connected(
        store: Arc<PositionStore>,
        gate: Arc<ReadinessGate>,
        topology: FleetTopology,
    ) -> Result<Self, Status> {
        let fleet = match &topology {
            FleetTopology::InProcess => None,
            FleetTopology::Distributed { endpoints } => {
                Some(Arc::new(federate::Fleet::connect(endpoints).await?))
            }
        };
        Ok(Self {
            store,
            gate,
            topology,
            fleet,
            sessions: default_sessions(),
            rates: default_rates_store(),
        })
    }

    /// Construct a **distributed** risk edge over an already-connected
    /// [`federate::Fleet`] (and the firm hierarchy/limit config carried by `store`).
    /// This is the explicit-membership entry point the boot path's
    /// [`RiskEdge::with_topology_connected`] funnels into, and the seam a failover
    /// topology test uses (build the fleet with [`federate::Fleet::connect_with_membership`]
    /// to express a down replica backed by a hot standby). The topology is recorded as
    /// [`FleetTopology::Distributed`] over the fleet's backend count.
    #[must_use]
    pub fn with_fleet(
        store: Arc<PositionStore>,
        gate: Arc<ReadinessGate>,
        fleet: Arc<federate::Fleet>,
    ) -> Self {
        let endpoints = (0..fleet.len()).map(|i| format!("backend-{i}")).collect();
        Self {
            store,
            gate,
            topology: FleetTopology::Distributed { endpoints },
            fleet: Some(fleet),
            sessions: default_sessions(),
            rates: default_rates_store(),
        }
    }

    /// Install the edge-wide [`SessionRegistry`] so this edge validates session
    /// tokens against the SAME authentication state every other front shares (the
    /// boot path calls this; the constructors otherwise default to an empty
    /// registry). Builder-style so a federated edge clones the parent's registry
    /// onto its staged sub-edges.
    #[must_use]
    pub fn with_sessions(mut self, sessions: Arc<SessionRegistry>) -> Self {
        self.sessions = sessions;
        self
    }

    /// Install the shared [`RatesPositionStore`](crate::services::rates_book::RatesPositionStore)
    /// so this edge's `BookRatesPosition` / `ListRatesPositions` RPCs read and write the
    /// SAME rates book the dealer-quoting [`RfqDeskService`](crate::services::desk) books
    /// accepted deals into (the boot path shares one instance across both edges).
    #[must_use]
    pub fn with_rates_store(
        mut self,
        rates: Arc<crate::services::rates_book::RatesPositionStore>,
    ) -> Self {
        self.rates = rates;
        self
    }

    /// The shared rates position book (so the boot path / a test can inspect it).
    #[must_use]
    pub fn rates_store(&self) -> &Arc<crate::services::rates_book::RatesPositionStore> {
        &self.rates
    }

    /// The fleet topology this edge was booted under (the resolved deploy-time seam).
    #[must_use]
    pub fn topology(&self) -> &FleetTopology {
        &self.topology
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

    /// Resolve how an RPC should be served under the edge's topology:
    /// [`Serve::Direct`] for the in-process single-node path, or
    /// [`Serve::Federate`] (carrying the connected backend [`federate::Fleet`]) for a
    /// distributed edge. A distributed edge **without** a connected fleet (built via
    /// the sync [`RiskEdge::with_topology`]) fails loudly with `unavailable` — it never
    /// silently serves a single-node answer it cannot vouch for.
    ///
    /// # Errors
    /// [`Status::unavailable`] for a distributed edge whose fleet was never connected.
    fn serve_mode(&self) -> Result<Serve<'_>, Status> {
        match &self.topology {
            FleetTopology::InProcess => Ok(Serve::Direct),
            FleetTopology::Distributed { endpoints } => match &self.fleet {
                Some(fleet) => Ok(Serve::Federate(fleet)),
                None => Err(Status::unavailable(format!(
                    "distributed fleet risk edge over {} backend(s) is not connected; \
                     construct it with `RiskEdge::with_topology_connected`",
                    endpoints.len()
                ))),
            },
        }
    }

    /// The shared position book (so the booking path can record live trades into it).
    #[must_use]
    pub fn store(&self) -> &Arc<PositionStore> {
        &self.store
    }
}

/// How an RPC is served under the edge's resolved [`FleetTopology`].
enum Serve<'a> {
    /// In-process: run the single-node aggregation directly over the local store.
    Direct,
    /// Distributed: fan out across the connected backend [`federate::Fleet`].
    Federate(&'a federate::Fleet),
}

/// Resolve a wire [`RiskScope`] (dimension + value) onto a cube `(dimension, value)`
/// scope filter, or `None` for the firm apex (covers everything).
///
/// # Errors
/// `invalid_argument` for an unknown dimension enum value.
fn scope_filter(scope: Option<&RiskScope>) -> Result<Option<(DimensionId, u64)>, Status> {
    match scope {
        None => Ok(None),
        Some(s) => Ok(convert::dimension_of(s.dimension)?.map(|d| (d, s.value))),
    }
}

/// Map a wire [`RiskScope`] onto a [`LimitScope`] (the limit-tree addressing). A
/// `FIRM` scope (or a `FIRM`-dimension scope) is the firm apex.
///
/// # Errors
/// `invalid_argument` for an unknown dimension enum value.
fn limit_scope_of(scope: &RiskScope) -> Result<LimitScope, Status> {
    let value = scope.value;
    Ok(match convert::dimension_of(scope.dimension)? {
        None => LimitScope::Firm,
        Some(DimensionId::Trader) => {
            LimitScope::Trader(TraderId(u32::try_from(value).unwrap_or(0)))
        }
        Some(DimensionId::Book) => LimitScope::Book(BookId(u32::try_from(value).unwrap_or(0))),
        Some(DimensionId::Desk) => LimitScope::Desk(DeskId(u32::try_from(value).unwrap_or(0))),
        Some(DimensionId::Location) => {
            LimitScope::Location(LocationId(u32::try_from(value).unwrap_or(0)))
        }
        Some(DimensionId::Entity) => {
            LimitScope::Entity(EntityId(u32::try_from(value).unwrap_or(0)))
        }
        Some(DimensionId::Underlying) => {
            // An underlying limit scope is addressed by the packed underlying
            // discriminant; the limit tree's `LimitScope::CcyPair` packs the same way
            // for FX/metals, but the wire only carries the `u64` value, so we cannot
            // reconstruct the underlying. An underlying limit scope is therefore not
            // addressable by value alone — reject loudly rather than guess.
            return Err(Status::invalid_argument(
                "limit status for a CCY_PAIR scope is addressed by the pair, not a bare value",
            ));
        }
    })
}

impl RiskEdge {
    /// **The effective pruning principal** for a read (item B §3): the caller's
    /// asserted (or grant-all-defaulted) body principal, then **narrowed to the
    /// caller's session-derived desk** so a non-admin desk-bound session cannot
    /// widen to a firm-wide view via an omitted/grant-all body principal.
    ///
    /// * an **admin** session, or **no** session (the legacy/demo/federation path) ⇒
    ///   [`DeskScope::All`] ⇒ no narrowing — byte-identical to the prior
    ///   `convert::principal_of(req.principal)` behaviour;
    /// * a **trader** session bound to desk `slug` ⇒ narrow to `Desk = intern(slug)`
    ///   (the canonical numeric desk id the boot-time `configure_desk` populated);
    /// * a **trader** session with no desk ⇒ narrow to `Desk = 0` (the house/unowned
    ///   `DeskId(0)` facts only).
    ///
    /// The numeric desk id is resolved through THIS store's interner
    /// ([`PositionStore::intern`]), which is idempotent and agrees with the live
    /// attribution path's lazy book interning within a run (item B §3 design).
    ///
    /// # Errors
    /// `invalid_argument` if the asserted principal carries an unknown dimension.
    fn effective_principal(
        &self,
        asserted: Option<&celnet_proto::EntitlementPrincipal>,
        caller: &ResolvedCaller,
    ) -> Result<celnet_entitlements::Principal, Status> {
        let base = convert::principal_of(asserted)?;
        Ok(match caller.desk_scope() {
            DeskScope::All => base,
            DeskScope::Desk(slug) => {
                convert::narrow_to_desk(base, u64::from(self.store.intern(&slug)))
            }
            // House/unowned facts (DeskId 0) — a deskless trader sees only those.
            DeskScope::Deskless => convert::narrow_to_desk(base, 0),
        })
    }

    /// The **wire** desk-narrowed principal to forward to federation backends (item
    /// B §3). For an admin / no-session caller ([`DeskScope::All`]) this is the
    /// asserted principal verbatim — `None` stays `None` — so the forwarded request
    /// is byte-identical to before. For a desk-bound caller it is the narrowed
    /// rule-set ([`Self::effective_principal`]) re-serialized to wire form, so each
    /// backend re-prunes by the same narrowing the aggregating edge applies locally.
    ///
    /// NOTE (flagged design): the narrowed rule pins `Desk = intern(slug)` where the
    /// handle is THIS edge's interner value. In a distributed deployment the backends'
    /// facts carry their `OrgKey.desk` handle; the narrowing is correct iff that
    /// handle equals this edge's slug interning (the firm-uniform identity config +
    /// deterministic boot interning make this hold). A handle disagreement can only
    /// *under*-count (show fewer facts than entitled) — never widen — which is the
    /// security-conservative failure direction. The default/only-Phase-2 topology
    /// (`InProcess`) narrows locally and is unaffected.
    ///
    /// # Errors
    /// `invalid_argument` if the asserted principal carries an unknown dimension.
    fn narrowed_wire_principal(
        &self,
        asserted: Option<&celnet_proto::EntitlementPrincipal>,
        caller: &ResolvedCaller,
    ) -> Result<Option<celnet_proto::EntitlementPrincipal>, Status> {
        if caller.desk_scope().is_all() {
            return Ok(asserted.cloned());
        }
        Ok(Some(convert::principal_to_wire(
            &self.effective_principal(asserted, caller)?,
        )))
    }

    /// The shared list-positions implementation (gRPC + WS both call this).
    ///
    /// # Errors
    /// `invalid_argument` for a malformed scope/principal.
    pub fn list_positions_impl(
        &self,
        req: &ListPositionsRequest,
        caller: &ResolvedCaller,
    ) -> Result<ListPositionsResponse, Status> {
        let snapshot = self.store.snapshot();
        let principal = self.effective_principal(req.principal.as_ref(), caller)?;
        let scope = scope_filter(req.scope.as_ref())?;
        let facts =
            entitled_facts_for_scope(&snapshot.facts, &principal, &snapshot.hierarchy, scope);
        let positions = facts
            .iter()
            .map(|f| {
                convert::fact_to_position(
                    f,
                    snapshot.wire_id_of(f.position_id.0),
                    snapshot.attribution_of(f.position_id.0).cloned(),
                )
            })
            .collect();
        Ok(ListPositionsResponse {
            positions,
            correlation_id: req.correlation_id,
        })
    }

    /// The shared aggregate-risk implementation (gRPC + WS both call this).
    ///
    /// # Errors
    /// `invalid_argument` for a malformed request; `failed_precondition` for a
    /// missing/invalid reporting-numeraire rate.
    pub fn aggregate_risk_impl(
        &self,
        req: &AggregateRiskRequest,
        caller: &ResolvedCaller,
    ) -> Result<AggregateRiskResponse, Status> {
        let numeraire = req
            .numeraire
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("AggregateRisk missing `numeraire`"))?;
        let resolver = WireResolver::new(numeraire)?;
        let principal = self.effective_principal(req.principal.as_ref(), caller)?;
        let dim = convert::dimension_of(req.dimension)?;
        let scope = scope_filter(req.scope.as_ref())?;

        let snapshot = self.store.snapshot();
        // Prune by principal BEFORE roll-up; narrow by scope (a sub-tree) by pruning
        // the entitled facts down to the scope, then aggregate the resulting cube.
        let entitled = entitled_cube(&snapshot.facts, &principal, &snapshot.hierarchy);
        let cube = match scope {
            None => entitled,
            Some((sdim, value)) => {
                // Restrict to the scope sub-tree: keep only facts whose resolved
                // group value on `sdim` matches, then re-build the cube.
                let scoped: Vec<_> = snapshot
                    .facts
                    .iter()
                    .filter(|f| {
                        celnet_entitlements::EntitlementFilter::new(&principal, &snapshot.hierarchy)
                            .admits(f)
                            && aggregate::resolved_group_value(&snapshot.hierarchy, f, sdim)
                                == value
                    })
                    .cloned()
                    .collect();
                cube_from_facts(&scoped, snapshot.hierarchy.clone())
            }
        };

        let cfg = NonAdditiveConfig {
            spot_shocks: req.var_spot_shocks.clone(),
            var_alpha: req.var_alpha,
            curvature_risk_weight: req.curvature_risk_weight,
        };
        let nodes = aggregate_nodes(&cube, dim, &resolver, &req.vega_pillars, &cfg)?;

        Ok(AggregateRiskResponse {
            dimension: req.dimension,
            numeraire: resolver.numeraire_code().to_owned(),
            nodes,
            correlation_id: req.correlation_id,
        })
    }

    /// The shared drill-risk implementation (gRPC + WS both call this).
    ///
    /// # Errors
    /// `invalid_argument` for a malformed request; `failed_precondition` for a
    /// numeraire rate failure.
    pub fn drill_risk_impl(
        &self,
        req: &DrillRiskRequest,
        caller: &ResolvedCaller,
    ) -> Result<DrillRiskResponse, Status> {
        let node_scope = req
            .node
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("DrillRisk missing `node`"))?;
        let numeraire = req
            .numeraire
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("DrillRisk missing `numeraire`"))?;
        let resolver = WireResolver::new(numeraire)?;
        let principal = self.effective_principal(req.principal.as_ref(), caller)?;
        let scope = scope_filter(Some(node_scope))?;

        let snapshot = self.store.snapshot();
        // The node's constituent facts (entitled + within the drilled scope).
        let scope_facts =
            entitled_facts_for_scope(&snapshot.facts, &principal, &snapshot.hierarchy, scope);

        // Child sub-nodes broken out by `child_dimension` over the scope's facts.
        let children = if req.include_children {
            let child_dim = convert::dimension_of(req.child_dimension)?;
            let scope_cube = cube_from_facts(&scope_facts, snapshot.hierarchy.clone());
            let cfg = NonAdditiveConfig::default();
            aggregate_nodes(&scope_cube, child_dim, &resolver, &req.vega_pillars, &cfg)?
        } else {
            Vec::new()
        };

        // The node's contributing positions (the drill-down leaves).
        let positions = if req.include_positions {
            scope_facts
                .iter()
                .map(|f| {
                    convert::fact_to_position(
                        f,
                        snapshot.wire_id_of(f.position_id.0),
                        snapshot.attribution_of(f.position_id.0).cloned(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(DrillRiskResponse {
            node: Some(*node_scope),
            children,
            positions,
            correlation_id: req.correlation_id,
        })
    }

    /// The shared limit-status implementation (gRPC + WS both call this).
    ///
    /// # Errors
    /// `invalid_argument` for a malformed scope; `failed_precondition` for a
    /// numeraire rate failure.
    pub fn limit_status_impl(
        &self,
        req: &LimitStatusRequest,
        caller: &ResolvedCaller,
    ) -> Result<LimitStatusResponse, Status> {
        let wire_scope = req
            .scope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("LimitStatus missing `scope`"))?;
        let numeraire = req
            .numeraire
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("LimitStatus missing `numeraire`"))?;
        let resolver = WireResolver::new(numeraire)?;
        let principal = self.effective_principal(req.principal.as_ref(), caller)?;
        let limit_scope = limit_scope_of(wire_scope)?;
        let scope = scope_filter(Some(wire_scope))?;

        let snapshot = self.store.snapshot();
        // Aggregate the scope node (entitled + scoped), in canonical units.
        let scope_facts =
            entitled_facts_for_scope(&snapshot.facts, &principal, &snapshot.hierarchy, scope);
        let scope_cube = cube_from_facts(&scope_facts, snapshot.hierarchy.clone());
        let node = match limit_scope.dimension() {
            None => scope_cube.firm_aggregate(&aggregate::default_grid()),
            Some(d) => scope_cube
                .group_by(d, &aggregate::default_grid())
                .into_iter()
                .find(|n| Some(n.group) == limit_scope.group_value())
                .unwrap_or_else(|| empty_node(limit_scope.group_value().unwrap_or(0))),
        };

        // Express the node in the reporting numeraire so the additive limit
        // exposures (delta / vega / premium / vega-ladder) are numeraire-consistent.
        let numeraire_node = numeraire_expressed_node(&node, &resolver)?;

        // Non-additive (VaR/ES/stop-loss) limit exposures, re-derived over
        // numeraire-scaled positions when shocks are supplied.
        let nonadditive = if req.var_spot_shocks.is_empty() {
            NonAdditiveExposure::default()
        } else {
            let alpha = if req.var_alpha > 0.0 {
                req.var_alpha
            } else {
                0.99
            };
            let scaled = scale_node_positions(&node, &resolver)?;
            let scaled_node = NodeAggregate {
                positions: scaled,
                ..node.clone()
            };
            let scenarios: Vec<_> = req
                .var_spot_shocks
                .iter()
                .map(|s| celnet_risk_cube::Scenario::spot(*s))
                .collect();
            NonAdditiveExposure::from_scenarios(
                &celnet_risk_normalize::AssetPricer,
                &scaled_node,
                &scenarios,
                alpha,
            )
        };

        let checks = check_scope(&snapshot.limits, limit_scope, &numeraire_node, &nonadditive);
        let mut worst = RagStatus::Green;
        let mut hard_breach = false;
        let limits: Vec<LimitUtilization> = checks
            .iter()
            .map(|c| {
                worst = worst.max(c.utilization.status);
                if c.limit.enforcement == Enforcement::Hard && c.utilization.status.is_breach() {
                    hard_breach = true;
                }
                utilization_to_wire(&c.utilization)
            })
            .collect();

        Ok(LimitStatusResponse {
            scope: Some(*wire_scope),
            limits,
            worst: convert::rag_to_wire(worst),
            hard_breach,
            correlation_id: req.correlation_id,
        })
    }

    /// The shared `BookRatesPosition` implementation (gRPC + WS both call this).
    /// Upserts the supplied [`RatesPosition`] into the live rates book (a `0`
    /// `position_id` ⇒ the server assigns a fresh one) and echoes the stored fact.
    ///
    /// # Errors
    /// `invalid_argument` for a missing position or instrument.
    pub fn book_rates_position_impl(
        &self,
        req: &BookRatesPositionRequest,
    ) -> Result<BookRatesPositionResponse, Status> {
        let position = req
            .position
            .ok_or_else(|| Status::invalid_argument("BookRatesPosition missing `position`"))?;
        if position.instrument.is_none() {
            return Err(Status::invalid_argument(
                "BookRatesPosition: position carries no instrument",
            ));
        }
        let booked = self.rates.book(position);
        Ok(BookRatesPositionResponse {
            position: Some(booked),
        })
    }

    /// The shared `ListRatesPositions` implementation (gRPC + WS both call this).
    /// Returns the live rates book, narrowed by the optional `(entity, book)` scope
    /// and **entitlement-pruned** to the cells the asserted principal admits (the
    /// deny-by-default semantics, mirrored over the flat rates `(entity, book)` space
    /// — see [`crate::services::rates_book::admits_rates_cell`]).
    ///
    /// # Errors
    /// Infallible today; returns `Result` for parity with the other `_impl` reads.
    pub fn list_rates_positions_impl(
        &self,
        req: &ListRatesPositionsRequest,
    ) -> Result<ListRatesPositionsResponse, Status> {
        let positions = self
            .rates
            .snapshot()
            .into_iter()
            .filter(|p| {
                // Optional scope narrowing: a present `entity`/`book` must match. The
                // `ccy` axis is a curve-currency narrowing that a stored position (no
                // currency of its own) cannot answer, so it does not constrain here.
                req.scope.as_ref().is_none_or(|s| {
                    s.entity.is_none_or(|e| e == p.entity) && s.book.is_none_or(|b| b == p.book)
                })
            })
            .filter(|p| {
                crate::services::rates_book::admits_rates_cell(
                    req.principal.as_ref(),
                    p.entity,
                    p.book,
                )
            })
            .collect();
        Ok(ListRatesPositionsResponse { positions })
    }
}

/// An empty node aggregate at a group (a scope with no current risk).
fn empty_node(group: u64) -> NodeAggregate {
    NodeAggregate {
        group,
        net_greeks: NetGreeks::zero(),
        vega_ladder: VegaLadder::new(),
        positions: Vec::new(),
        exotic_legs: Vec::new(),
        leaves: Vec::new(),
    }
}

/// Express a node's additive measures in the reporting numeraire so the limit layer
/// (which reads `NetGreeks` / `VegaLadder` directly) sees numeraire-consistent
/// exposures: `delta_base ← delta_numeraire`, `vega ← vega_numeraire`,
/// `premium_quote ← premium_numeraire`, and the vega ladder converted per pillar
/// through the premium currency. Gamma / vanna / volga / charm / speed / zomma /
/// color are pure sensitivities (the proto contract reports them raw), so they pass
/// through unchanged. The leaves are scaled so concentration metrics
/// (gross delta / vega) are numeraire-consistent too.
///
/// # Errors
/// `failed_precondition` for a missing/invalid numeraire rate.
fn numeraire_expressed_node(
    node: &NodeAggregate,
    resolver: &WireResolver,
) -> Result<NodeAggregate, Status> {
    let numeraire = node
        .numeraire_view(resolver)
        .map_err(convert::numeraire_status)?;
    let mut net = node.net_greeks;
    net.delta_base = numeraire.delta_numeraire;
    net.vega = numeraire.vega_numeraire;
    net.premium_quote = numeraire.premium_numeraire;

    // Numeraire-convert the vega ladder per pillar (through the premium currency),
    // bucketing by each position's tenor via the default grid — the SAME bucketing
    // the aggregator's ladder uses, so a bucketed-vega limit reads the same number.
    let mut ladder = VegaLadder::new();
    for (pillar, vega) in
        aggregate::numeraire_pillar_vega(node, resolver, &aggregate::default_grid())?
    {
        ladder.add(pillar, vega);
    }

    // Scale the leaves' delta_base / vega for the gross concentration metrics.
    let mut leaves = node.leaves.clone();
    for leaf in &mut leaves {
        // The leaf's base-leg currency (the leg `delta_base` nets at). FX/metals
        // project to their pair base; a cross-asset base leg is an asset unit with no
        // `Ccy`, so its gross-concentration scaling falls back to the numeraire leg.
        let base_ccy = leaf
            .underlying
            .as_ccy_pair()
            .map_or(leaf.vega_premium_ccy, |p| p.base);
        let dr = resolver.rate_into_numeraire(base_ccy).ok_or_else(|| {
            convert::numeraire_status(celnet_risk_normalize::NumeraireError::MissingRate(base_ccy))
        })?;
        let vr = resolver
            .rate_into_numeraire(leaf.vega_premium_ccy)
            .ok_or_else(|| {
                convert::numeraire_status(celnet_risk_normalize::NumeraireError::MissingRate(
                    leaf.vega_premium_ccy,
                ))
            })?;
        leaf.greeks.delta_base *= dr;
        leaf.greeks.vega *= vr;
    }

    Ok(NodeAggregate {
        group: node.group,
        net_greeks: net,
        vega_ladder: ladder,
        positions: node.positions.clone(),
        // Carried through unchanged: this additive numeraire view feeds the limit
        // layer (which reads the additive measures); the exotic legs travel with the
        // node so it stays complete/reconcilable. Numeraire-correct exotic
        // re-pricing for VaR/ES is done in `aggregate` via scaled legs.
        exotic_legs: node.exotic_legs.clone(),
        leaves,
    })
}

/// Scale a node's positions so their repriced P&L is in the reporting numeraire
/// (notional × quote→numeraire rate) for the non-additive limit exposures.
///
/// # Errors
/// `failed_precondition` for a missing/invalid quote-currency rate.
fn scale_node_positions(
    node: &NodeAggregate,
    resolver: &WireResolver,
) -> Result<Vec<celnet_risk_normalize::PositionRisk>, Status> {
    let mut out = Vec::with_capacity(node.positions.len());
    for p in &node.positions {
        // The position's quote/premium (numeraire) currency — the leg the spot rate
        // converts into the reporting numeraire (FX/metal pair quote, or the cross-
        // asset arm's own numeraire currency).
        let quote = p.numeraire_ccy().ok_or_else(|| {
            convert::numeraire_status(celnet_risk_normalize::NumeraireError::MissingRate(
                celnet_types::Ccy::USD,
            ))
        })?;
        let rate = resolver.rate_into_numeraire(quote).ok_or_else(|| {
            convert::numeraire_status(celnet_risk_normalize::NumeraireError::MissingRate(quote))
        })?;
        if !rate.is_finite() || rate <= 0.0 {
            return Err(convert::numeraire_status(
                celnet_risk_normalize::NumeraireError::InvalidRate(quote),
            ));
        }
        // Clone-and-scale: preserve the carry-tagged inputs/underlying + FX-provenance
        // conventions exactly, scaling only the notional (an API-faithful migration).
        let mut scaled = p.clone();
        scaled.notional_base = p.notional_base * rate;
        out.push(scaled);
    }
    Ok(out)
}

/// Map a domain [`Utilization`] onto its wire [`LimitUtilization`].
fn utilization_to_wire(u: &Utilization) -> LimitUtilization {
    let (kind, vega_pillar, tenor_days) = convert::limit_metric_to_wire(u.metric);
    LimitUtilization {
        metric: kind as i32,
        vega_pillar,
        tenor_days,
        cap: u.cap,
        exposure: u.exposure,
        ratio: u.ratio,
        status: convert::rag_to_wire(u.status),
        enforcement: convert::enforcement_to_wire(u.enforcement),
        headroom: u.headroom(),
    }
}

// The four entitlement-gated RPCs of the contract — the **complete** set: no
// other service's request carries an `EntitlementPrincipal` (verified against
// `celnet.proto`; pricing/quote/stream/surface serve market data and lifecycle,
// not entitlement-scoped book reads). Each passes the deny-by-default
// authorization boundary ([`crate::services::access::authorize`]) — which
// audits every allow AND deny — before any serving mode is resolved. The WS
// mirror dispatches onto these same trait methods, so one boundary covers both
// encodings; the federated frontend authorizes here once and forwards the
// asserted principal to the backends, which re-authorize it at their own
// trait entry.
#[tonic::async_trait]
impl RiskService for RiskEdge {
    async fn list_positions(
        &self,
        request: Request<ListPositionsRequest>,
    ) -> Result<Response<ListPositionsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "RiskService/ListPositions",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        let resp = match self.serve_mode()? {
            Serve::Direct => self.list_positions_impl(&req, &caller)?,
            Serve::Federate(fleet) => self.federated_list_positions(fleet, &req, &caller).await?,
        };
        Ok(Response::new(resp))
    }

    async fn aggregate_risk(
        &self,
        request: Request<AggregateRiskRequest>,
    ) -> Result<Response<AggregateRiskResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "RiskService/AggregateRisk",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        let resp = match self.serve_mode()? {
            Serve::Direct => self.aggregate_risk_impl(&req, &caller)?,
            Serve::Federate(fleet) => self.federated_aggregate_risk(fleet, &req, &caller).await?,
        };
        Ok(Response::new(resp))
    }

    async fn drill_risk(
        &self,
        request: Request<DrillRiskRequest>,
    ) -> Result<Response<DrillRiskResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "RiskService/DrillRisk",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        let resp = match self.serve_mode()? {
            Serve::Direct => self.drill_risk_impl(&req, &caller)?,
            Serve::Federate(fleet) => self.federated_drill_risk(fleet, &req, &caller).await?,
        };
        Ok(Response::new(resp))
    }

    async fn limit_status(
        &self,
        request: Request<LimitStatusRequest>,
    ) -> Result<Response<LimitStatusResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "RiskService/LimitStatus",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        let resp = match self.serve_mode()? {
            Serve::Direct => self.limit_status_impl(&req, &caller)?,
            Serve::Federate(fleet) => self.federated_limit_status(fleet, &req, &caller).await?,
        };
        Ok(Response::new(resp))
    }

    async fn aggregate_rates_risk(
        &self,
        request: Request<AggregateRatesRiskRequest>,
    ) -> Result<Response<AggregateRatesRiskResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "RiskService/AggregateRatesRisk",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        // Rates portfolio risk is a pure calculation over the request-supplied
        // `CurveSet` + positions (no live-market read, no per-pair fleet route to
        // forward), so the additive fan-in runs in-process under the edge topology
        // on every replica — no federation forwarding, unlike the store-backed RPCs.
        let resp = crate::services::rates_risk::aggregate_rates_risk(&req, self.topology())?;
        Ok(Response::new(resp))
    }

    async fn book_rates_position(
        &self,
        request: Request<BookRatesPositionRequest>,
    ) -> Result<Response<BookRatesPositionResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        // Booking is a write into the rates book — the strongest action on a rates
        // position. It carries the dedicated `Book·FixedIncome` capability: a caller
        // may see (`ReadAny`) and even respond/deal on rates yet still not be entitled
        // to commit a booked line. The capability gate requires an authenticated
        // session (a body principal cannot self-grant `Book`); an absent principal
        // under enforce is denied at the same boundary as the rates reads.
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "RiskService/BookRatesPosition",
            RequiredAuthority::Capability(Action::Book, AssetClass::FixedIncome),
            None,
        )?;
        let resp = self.book_rates_position_impl(&req)?;
        Ok(Response::new(resp))
    }

    async fn list_rates_positions(
        &self,
        request: Request<ListRatesPositionsRequest>,
    ) -> Result<Response<ListRatesPositionsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "RiskService/ListRatesPositions",
            RequiredAuthority::ReadAny,
            None,
        )?;
        let resp = self.list_rates_positions_impl(&req)?;
        Ok(Response::new(resp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_limits::{LimitMetric, LimitScope, LimitSpec};
    use celnet_proto::{
        AggregateRiskRequest, BookId as WireBookId, EntitlementPrincipal, EntitlementRule,
        ListPositionsRequest, NumeraireRate, Owner, ReportingNumeraire, RiskDimension, RiskScope,
        owner,
    };
    use celnet_types::{Ccy, CcyPair, OptionType, PremiumStyle, VanillaInputs};
    use store::BookedPosition;

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn usd_numeraire() -> ReportingNumeraire {
        ReportingNumeraire {
            numeraire: "USD".to_owned(),
            rates: vec![NumeraireRate {
                ccy: "EUR".to_owned(),
                rate: 1.10,
            }],
        }
    }

    fn attribution(book: &str, trader: &str) -> celnet_proto::AttributionRecord {
        celnet_proto::AttributionRecord {
            quoted_by: Some(WireBookId {
                book: "AUTO-MM".to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::AutoPricer("celnet-auto-pricer".to_owned())),
                }),
            }),
            held_by: Some(WireBookId {
                book: book.to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::Trader(trader.to_owned())),
                }),
            }),
            won: Some(true),
            lp_count: Some(2),
        }
    }

    fn booked(id: u64, notional: f64) -> BookedPosition {
        BookedPosition {
            position_id: id,
            pair: eurusd(),
            option: OptionType::Call,
            notional_base: notional,
            inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            quoted_delta: celnet_types::DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        }
    }

    /// A test edge over a fresh store, with the gate marked ready.
    fn edge() -> RiskEdge {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        RiskEdge::new(Arc::new(PositionStore::new()), gate)
    }

    /// A ready test edge under an explicit fleet [`FleetTopology`].
    fn edge_with(topology: FleetTopology) -> RiskEdge {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        RiskEdge::with_topology(Arc::new(PositionStore::new()), gate, topology)
    }

    /// Booking two lines on two desks, then listing positions, round-trips the
    /// attribution chain and the org placement (the live book the GUI Book reads).
    #[test]
    fn positions_round_trip_through_the_edge() {
        let edge = edge();
        edge.store
            .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .unwrap();
        edge.store
            .book_from_attribution(booked(2, 5_000_000.0), &attribution("G10-1", "asmith"))
            .unwrap();

        let resp = edge
            .list_positions_impl(
                &ListPositionsRequest {
                    scope: None,
                    principal: None, // grant-all default
                    correlation_id: Some(42),
                    session_token: None,
                },
                &ResolvedCaller::anonymous(),
            )
            .unwrap();
        assert_eq!(resp.positions.len(), 2);
        assert_eq!(resp.correlation_id, Some(42));
        // The wire position id round-trips (the cube handle is internal).
        let ids: Vec<u64> = resp.positions.iter().map(|p| p.position_id).collect();
        assert!(ids.contains(&1) && ids.contains(&2));
        // The attribution chain is reported (real holder books).
        let books: std::collections::HashSet<String> = resp
            .positions
            .iter()
            .filter_map(|p| p.attribution.as_ref())
            .filter_map(|a| a.held_by.as_ref())
            .map(|b| b.book.clone())
            .collect();
        assert!(books.contains("EM-VOL-1") && books.contains("G10-1"));
    }

    /// **Entitlement pruning before aggregation, no leakage.** Two desks; a
    /// grant-all principal sees the firm total over both, a desk-scoped principal
    /// sees ONLY its desk's total — proving the other desk's magnitude never leaks
    /// into the scoped aggregate (it is pruned before roll-up).
    #[test]
    fn entitlement_pruning_before_aggregation_no_leakage() {
        let edge = edge();
        // Book two desks: EM-VOL (book 1) and G10 (book 2), mapped to two desks.
        edge.store
            .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "a"))
            .unwrap();
        edge.store
            .book_from_attribution(booked(2, 7_000_000.0), &attribution("G10-1", "b"))
            .unwrap();
        let em_book = edge.store.intern("EM-VOL-1");
        let g10_book = edge.store.intern("G10-1");
        let em_desk = edge.store.intern("EM-VOL-DESK");
        let g10_desk = edge.store.intern("G10-DESK");
        edge.store.set_book_desk(em_book, em_desk);
        edge.store.set_book_desk(g10_book, g10_desk);

        let req = |principal: Option<EntitlementPrincipal>| AggregateRiskRequest {
            dimension: RiskDimension::Firm as i32,
            numeraire: Some(usd_numeraire()),
            principal,
            scope: None,
            vega_pillars: vec![],
            var_spot_shocks: vec![],
            var_alpha: 0.0,
            curvature_risk_weight: 0.0,
            correlation_id: None,
            session_token: None,
        };

        // Grant-all (omitted principal) sees the whole firm. We compare on the
        // premium (a long call's PV is positive and additive) — the single-pair
        // `delta_numeraire` self-funds to ~0 in the quote numeraire (the EUR hedge
        // and its USD funding leg net), which is correct but not a useful magnitude.
        let firm = edge
            .aggregate_risk_impl(&req(None), &ResolvedCaller::anonymous())
            .unwrap();
        let firm_prem = firm.nodes[0].additive.as_ref().unwrap().premium_numeraire;
        assert_eq!(firm.nodes[0].position_count, 2);
        assert!(firm_prem > 0.0, "long calls → positive firm premium");

        // A principal scoped to the EM-VOL desk sees ONLY that desk's firm total.
        let scoped = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Desk as i32,
                    value: u64::from(em_desk),
                }],
            }],
            denies: vec![],
        };
        let scoped_resp = edge
            .aggregate_risk_impl(&req(Some(scoped)), &ResolvedCaller::anonymous())
            .unwrap();
        assert_eq!(
            scoped_resp.nodes[0].position_count, 1,
            "only the EM-VOL leg"
        );
        let scoped_prem = scoped_resp.nodes[0]
            .additive
            .as_ref()
            .unwrap()
            .premium_numeraire;

        // The scoped total must DIFFER from the firm total (no leakage of G10): the
        // scoped premium is strictly less than the firm premium (it omits G10's).
        assert!(scoped_prem > 0.0);
        assert!(
            scoped_prem < firm_prem - 1.0,
            "scoped premium {scoped_prem} must be below firm {firm_prem} (no leakage)"
        );

        // A deny-wins barrier on the firm-wide view cuts EM-VOL: the remaining firm
        // total then excludes EM-VOL entirely (information barrier).
        let walled = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Desk as i32,
                    value: u64::from(em_desk),
                }],
            }],
        };
        let walled_resp = edge
            .aggregate_risk_impl(&req(Some(walled)), &ResolvedCaller::anonymous())
            .unwrap();
        assert_eq!(walled_resp.nodes[0].position_count, 1, "EM-VOL walled off");
    }

    /// **A hard limit breach is reported.** A desk delta limit set well below the
    /// node's numeraire delta breaches RED/BREACH and sets `hard_breach`.
    #[test]
    fn limit_status_reports_a_hard_breach() {
        let edge = edge();
        edge.store
            .book_from_attribution(booked(1, 50_000_000.0), &attribution("EM-VOL-1", "a"))
            .unwrap();
        let book = edge.store.intern("EM-VOL-1");

        // First read the node's delta exposure with a generous limit so we know the
        // magnitude, then set a hard cap well below it.
        let scope = RiskScope {
            dimension: RiskDimension::Book as i32,
            value: u64::from(book),
        };
        // A hard vega cap of 1.0 (tiny) vs a ~50mm-notional book's USD vega → breach.
        // (Vega is non-zero in the numeraire; the single-pair delta self-funds to ~0,
        // so a vega cap is the clean breach exercise here.)
        edge.store.set_limit(
            LimitScope::Book(BookId(book)),
            LimitSpec::hard(LimitMetric::Vega, 1.0),
        );

        let resp = edge
            .limit_status_impl(
                &LimitStatusRequest {
                    scope: Some(scope),
                    numeraire: Some(usd_numeraire()),
                    principal: None,
                    vega_pillars: vec![],
                    var_spot_shocks: vec![],
                    var_alpha: 0.0,
                    correlation_id: Some(7),
                    session_token: None,
                },
                &ResolvedCaller::anonymous(),
            )
            .unwrap();
        assert_eq!(resp.correlation_id, Some(7));
        assert_eq!(resp.limits.len(), 1);
        assert_eq!(resp.worst, celnet_proto::RagStatus::Breach as i32);
        assert!(resp.hard_breach, "a hard delta breach must set hard_breach");
        let lim = &resp.limits[0];
        assert_eq!(lim.metric, celnet_proto::LimitMetricKind::Vega as i32);
        assert!(lim.exposure.abs() > lim.cap, "exposure over cap");
        assert!(lim.headroom < 0.0, "negative headroom on a breach");

        // A generous cap → green, no hard breach.
        edge.store.set_limit(
            LimitScope::Book(BookId(book)),
            LimitSpec::hard(LimitMetric::Vega, 1.0e12),
        );
        let clear = edge
            .limit_status_impl(
                &LimitStatusRequest {
                    scope: Some(scope),
                    numeraire: Some(usd_numeraire()),
                    principal: None,
                    vega_pillars: vec![],
                    var_spot_shocks: vec![],
                    var_alpha: 0.0,
                    correlation_id: None,
                    session_token: None,
                },
                &ResolvedCaller::anonymous(),
            )
            .unwrap();
        assert_eq!(clear.worst, celnet_proto::RagStatus::Green as i32);
        assert!(!clear.hard_breach);
    }

    /// **Drill from a desk node into its books**, with the contributing positions,
    /// entitlement-pruned. A drill returns the child book sub-nodes and the leaves.
    #[test]
    fn drill_desk_into_books_and_positions() {
        let edge = edge();
        edge.store
            .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "a"))
            .unwrap();
        edge.store
            .book_from_attribution(booked(2, 6_000_000.0), &attribution("EM-VOL-2", "b"))
            .unwrap();
        let b1 = edge.store.intern("EM-VOL-1");
        let b2 = edge.store.intern("EM-VOL-2");
        let desk = edge.store.intern("EM-VOL-DESK");
        edge.store.set_book_desk(b1, desk);
        edge.store.set_book_desk(b2, desk);

        let resp = edge
            .drill_risk_impl(
                &celnet_proto::DrillRiskRequest {
                    node: Some(RiskScope {
                        dimension: RiskDimension::Desk as i32,
                        value: u64::from(desk),
                    }),
                    child_dimension: RiskDimension::Book as i32,
                    numeraire: Some(usd_numeraire()),
                    principal: None,
                    vega_pillars: vec![],
                    include_children: true,
                    include_positions: true,
                    correlation_id: Some(9),
                    session_token: None,
                },
                &ResolvedCaller::anonymous(),
            )
            .unwrap();
        assert_eq!(resp.correlation_id, Some(9));
        // Two child book sub-nodes, two contributing positions.
        assert_eq!(resp.children.len(), 2);
        assert_eq!(resp.positions.len(), 2);
        // Each child reports the Book dimension.
        assert!(
            resp.children
                .iter()
                .all(|c| c.dimension == RiskDimension::Book as i32)
        );
    }

    /// **The default edge is the in-process topology.** A `RiskEdge::new` edge (and
    /// thus the boot default when `CELNET_FLEET_MODE`/`CELNET_FLEET_BACKENDS` are
    /// absent) carries [`FleetTopology::InProcess`], so `serve_mode` resolves to the
    /// direct single-node path and the aggregation runs unchanged.
    #[test]
    fn default_edge_is_in_process_and_servable() {
        let edge = edge();
        assert_eq!(*edge.topology(), FleetTopology::InProcess);
        assert!(matches!(edge.serve_mode(), Ok(Serve::Direct)));
    }

    /// **The in-process default path is byte-identical to the topology-free edge.**
    /// The same booked book aggregated through a `RiskEdge::new` edge and through an
    /// explicit `with_topology(InProcess)` edge yields a bit-for-bit identical
    /// `AggregateRiskResponse` — proving threading the topology in changed nothing on
    /// the served path (Phase-2 zero-behavior-change invariant).
    #[test]
    fn in_process_aggregate_is_byte_identical_to_default() {
        let book = |edge: &RiskEdge| {
            edge.store
                .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "a"))
                .unwrap();
            edge.store
                .book_from_attribution(booked(2, 7_000_000.0), &attribution("G10-1", "b"))
                .unwrap();
        };
        let req = AggregateRiskRequest {
            dimension: RiskDimension::Firm as i32,
            numeraire: Some(usd_numeraire()),
            principal: None,
            scope: None,
            vega_pillars: vec![],
            var_spot_shocks: vec![-0.02, -0.01, 0.0, 0.01, 0.02],
            var_alpha: 0.99,
            curvature_risk_weight: 0.18,
            correlation_id: Some(123),
            session_token: None,
        };

        let default_edge = edge();
        book(&default_edge);
        let from_default = default_edge
            .aggregate_risk_impl(&req, &ResolvedCaller::anonymous())
            .unwrap();

        let inproc_edge = edge_with(FleetTopology::InProcess);
        book(&inproc_edge);
        let from_inproc = inproc_edge
            .aggregate_risk_impl(&req, &ResolvedCaller::anonymous())
            .unwrap();

        // Same dimension / numeraire / correlation, same node count.
        assert_eq!(from_default.dimension, from_inproc.dimension);
        assert_eq!(from_default.numeraire, from_inproc.numeraire);
        assert_eq!(from_default.correlation_id, from_inproc.correlation_id);
        assert_eq!(from_default.nodes.len(), from_inproc.nodes.len());
        // Every additive measure bit-identical, and the re-derived non-additive
        // VaR/ES/curvature bit-identical too (same direct path, same inputs).
        for (d, i) in from_default.nodes.iter().zip(from_inproc.nodes.iter()) {
            assert_eq!(d.position_count, i.position_count);
            let da = d.additive.as_ref().unwrap();
            let ia = i.additive.as_ref().unwrap();
            assert_eq!(
                da.delta_numeraire.to_bits(),
                ia.delta_numeraire.to_bits(),
                "delta_numeraire must be byte-identical"
            );
            assert_eq!(da.vega_numeraire.to_bits(), ia.vega_numeraire.to_bits());
            assert_eq!(
                da.premium_numeraire.to_bits(),
                ia.premium_numeraire.to_bits()
            );
            // Non-additive measures are presence-tracked Option<f64>; compare both
            // presence and (when present) the exact bits.
            let bits = |x: Option<f64>| x.map(f64::to_bits);
            let dn = d.nonadditive.as_ref().unwrap();
            let in_ = i.nonadditive.as_ref().unwrap();
            assert_eq!(bits(dn.var), bits(in_.var), "VaR byte-identical");
            assert_eq!(bits(dn.es), bits(in_.es), "ES byte-identical");
            assert_eq!(
                bits(dn.curvature_spot),
                bits(in_.curvature_spot),
                "curvature byte-identical"
            );
        }
    }

    /// **A distributed edge with no connected fleet fails loudly, never silently
    /// degrades.** A `Distributed` topology built via the *sync* `with_topology` (no
    /// dialled backends) cannot vouch for a firm number, so every RPC returns a typed
    /// `unavailable` — it does NOT quietly serve the local single-node book. The
    /// resolved topology is still stored and inspectable. (The connected federation
    /// path — `with_topology_connected` over real backends — is exercised end-to-end
    /// in `tests/risk_federation.rs`.)
    #[tokio::test]
    async fn distributed_edge_without_fleet_is_unavailable() {
        let edge = edge_with(FleetTopology::Distributed {
            endpoints: vec!["shard-a:7000".to_owned(), "shard-b:7000".to_owned()],
        });
        edge.store
            .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "a"))
            .unwrap();

        // The topology is resolved and stored; no fleet was connected.
        assert!(matches!(edge.topology(), FleetTopology::Distributed { .. }));
        // The serve-mode guard rejects loudly (unavailable, not a stub answer).
        assert_eq!(
            edge.serve_mode().err().map(|s| s.code()),
            Some(tonic::Code::Unavailable),
            "an unconnected distributed edge must fail with unavailable"
        );

        // Every served RPC (gRPC trait surface — WS dispatches through the same
        // methods) returns Unavailable BEFORE serving the local book. The
        // requests assert an explicit grant-all principal so they pass the
        // deny-by-default authorization boundary and reach the serve-mode guard
        // (the absent-principal deny itself is proven in
        // `tests/entitlements_boundary.rs`).
        let asserted = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![],
        };
        let agg = RiskService::aggregate_risk(
            &edge,
            Request::new(AggregateRiskRequest {
                dimension: RiskDimension::Firm as i32,
                numeraire: Some(usd_numeraire()),
                principal: Some(asserted.clone()),
                scope: None,
                vega_pillars: vec![],
                var_spot_shocks: vec![],
                var_alpha: 0.0,
                curvature_risk_weight: 0.0,
                correlation_id: None,
                session_token: None,
            }),
        )
        .await;
        assert_eq!(agg.unwrap_err().code(), tonic::Code::Unavailable);

        let list = RiskService::list_positions(
            &edge,
            Request::new(ListPositionsRequest {
                scope: None,
                principal: Some(asserted),
                correlation_id: None,
                session_token: None,
            }),
        )
        .await;
        assert_eq!(list.unwrap_err().code(), tonic::Code::Unavailable);
    }

    /// **The env knob resolves the way the boot path expects.** `FleetTopology::parse`
    /// (the exact call `fleet_topology_from_env` makes) maps the absent/empty case and
    /// non-distributed modes to the in-process default, and the explicit distributed
    /// mode with backends to a `Distributed` topology over the trimmed endpoints.
    #[test]
    fn env_knob_resolves_to_expected_topology() {
        // Absent ⇒ in-process (the byte-identical default).
        assert_eq!(FleetTopology::parse("", ""), FleetTopology::InProcess);
        // Any non-distributed mode ⇒ in-process.
        assert_eq!(
            FleetTopology::parse("standalone", "shard-a:7000"),
            FleetTopology::InProcess
        );
        // distributed mode but no usable backend ⇒ in-process (safe default).
        assert_eq!(
            FleetTopology::parse("distributed", "  , "),
            FleetTopology::InProcess
        );
        // distributed + backends ⇒ Distributed over the trimmed, non-empty endpoints.
        assert_eq!(
            FleetTopology::parse("distributed", " shard-a:7000 , shard-b:7000 ,"),
            FleetTopology::Distributed {
                endpoints: vec!["shard-a:7000".to_owned(), "shard-b:7000".to_owned()],
            }
        );
    }

    /// Booking a rates line carries the dedicated `Book·FixedIncome` capability:
    /// an authenticated trader (who holds it) books and the line lands in the book,
    /// while an unauthenticated caller — even one asserting a grant-all body
    /// principal — is denied under the default `Enforce` boundary, because a body
    /// principal cannot self-grant the `Book` capability (the finding-#3 guard).
    #[tokio::test]
    async fn book_rates_position_requires_book_capability() {
        use celnet_proto::{OisInstrument, RatesInstrument, RatesPosition, Side, rates_instrument};

        let registry = Arc::new(SessionRegistry::new(Clock::system()));
        let token = registry
            .issue(crate::services::sessions::AuthenticatedUser {
                user_id: "t-1".to_owned(),
                email: "trader@celnet.com".to_owned(),
                display_name: "Rates Trader".to_owned(),
                role: crate::config::identity::Role::Trader,
                desk_id: Some("g10".to_owned()),
                cap_grants: Vec::new(),
                cap_denies: Vec::new(),
            })
            .expect("issue trader session")
            .token;
        let edge = edge().with_sessions(registry);

        let line = RatesPosition {
            position_id: 0,
            entity: 1,
            book: 10,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.04,
                    notional: 10_000_000.0,
                    side: Side::Buy as i32,
                })),
            }),
        };

        // Authenticated trader holds Book·FixedIncome ⇒ the line is booked and the
        // server echoes a freshly-assigned id.
        let booked = edge
            .book_rates_position(Request::new(BookRatesPositionRequest {
                session_token: Some(token),
                position: Some(line.clone()),
                principal: None,
                correlation_id: None,
            }))
            .await
            .expect("authenticated trader books the line")
            .into_inner()
            .position
            .expect("booked position echoed");
        assert_eq!(booked.entity, 1);
        assert_eq!(booked.book, 10);
        assert!(booked.position_id > 0, "server assigns a fresh id");

        // No session, asserting a grant-all body principal, under Enforce ⇒ denied:
        // a body principal cannot self-grant the Book capability.
        let denied = edge
            .book_rates_position(Request::new(BookRatesPositionRequest {
                session_token: None,
                position: Some(line),
                principal: Some(EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
            }))
            .await;
        assert!(
            denied.is_err(),
            "an unauthenticated grant-all principal must not book under enforce"
        );
    }

    // --- §3 desk-identity bridge + risk narrowing -----------------------------

    use crate::config::identity::Role;
    use crate::services::sessions::{AuthenticatedUser, SessionRegistry};

    /// The firm-aggregate node's `position_count` an `aggregate_risk_impl` returns
    /// for a given caller + asserted principal — the oracle observable.
    fn firm_count(
        edge: &RiskEdge,
        caller: &ResolvedCaller,
        principal: Option<EntitlementPrincipal>,
    ) -> u32 {
        let req = AggregateRiskRequest {
            dimension: RiskDimension::Firm as i32,
            numeraire: Some(usd_numeraire()),
            principal,
            scope: None,
            vega_pillars: vec![],
            var_spot_shocks: vec![],
            var_alpha: 0.0,
            curvature_risk_weight: 0.0,
            correlation_id: None,
            session_token: None,
        };
        let resp = edge.aggregate_risk_impl(&req, caller).unwrap();
        resp.nodes.first().map_or(0, |n| n.position_count)
    }

    /// **§3 oracle (independent): session-derived desk narrowing == an explicit desk
    /// scope, and is STRICTLY narrower than admin.** Two desks are wired via the
    /// boot-time `configure_desk` bridge (not the test-only `set_book_desk`). A
    /// desk-EM trader session asserting **grant-all** sees EXACTLY the same firm
    /// total as the same trader asserting `scoped().grant(Desk = EM)` — proving the
    /// narrowing equals an explicit desk scope — and sees STRICTLY FEWER positions
    /// than an admin (whose grant-all is unrestricted). A deskless trader sees only
    /// house/DeskId(0) facts (here: none). The property — not a hardcoded count — is
    /// the oracle.
    #[test]
    fn desk_narrowing_equals_explicit_desk_scope_and_is_narrower_than_admin() {
        let edge = edge();
        // Two desks, two books each-ish: EM-VOL (book EM-VOL-1) and G10 (book G10-1).
        edge.store
            .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "a"))
            .unwrap();
        edge.store
            .book_from_attribution(booked(2, 7_000_000.0), &attribution("G10-1", "b"))
            .unwrap();
        // Wire the bridge from config-shaped DeskDefs (the production boot path).
        edge.store
            .configure_desk("em-vol-desk", &["EM-VOL-1".to_owned()]);
        edge.store.configure_desk("g10-desk", &["G10-1".to_owned()]);

        // Mint sessions over a shared registry the edge validates against.
        let sessions = Arc::new(SessionRegistry::new(Clock::manual(0)));
        let edge = edge.with_sessions(Arc::clone(&sessions));
        let mk = |id: &str, role: Role, desk: Option<&str>| AuthenticatedUser {
            user_id: id.to_owned(),
            email: format!("{id}@celnet.com"),
            display_name: id.to_owned(),
            role,
            desk_id: desk.map(str::to_owned),
            cap_grants: Vec::new(),
            cap_denies: Vec::new(),
        };
        let trader_tok = sessions
            .issue(mk("emtrader", Role::Trader, Some("em-vol-desk")))
            .unwrap()
            .token;
        let admin_tok = sessions.issue(mk("boss", Role::Admin, None)).unwrap().token;
        let deskless_tok = sessions
            .issue(mk("nomad", Role::Trader, None))
            .unwrap()
            .token;

        let trader = resolve_caller(&sessions, Some(&trader_tok), None).unwrap();
        let admin = resolve_caller(&sessions, Some(&admin_tok), None).unwrap();
        let deskless = resolve_caller(&sessions, Some(&deskless_tok), None).unwrap();

        // The canonical numeric desk id of the EM desk (idempotent interning).
        let em_desk = edge.store.intern("em-vol-desk");
        let explicit_em = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Desk as i32,
                    value: u64::from(em_desk),
                }],
            }],
            denies: vec![],
        };

        // ORACLE 1: the desk-EM trader asserting grant-all (an OMITTED body
        // principal) sees EXACTLY the same firm total as the SAME trader asserting an
        // explicit `Desk = EM` scope — the narrowing IS the explicit desk scope.
        let trader_grant_all = firm_count(&edge, &trader, None);
        let trader_explicit_scope = firm_count(&edge, &trader, Some(explicit_em.clone()));
        assert_eq!(
            trader_grant_all, trader_explicit_scope,
            "session desk-narrowing of a grant-all body == an explicit Desk scope"
        );
        assert_eq!(
            trader_grant_all, 1,
            "the EM trader sees only the EM-VOL leg"
        );

        // ORACLE 2: an ADMIN session asserting grant-all sees the WHOLE firm (both
        // legs) — STRICTLY MORE than the desk-bound trader (no narrowing for admin).
        let admin_grant_all = firm_count(&edge, &admin, None);
        assert_eq!(admin_grant_all, 2, "admin's grant-all is unrestricted");
        assert!(
            trader_grant_all < admin_grant_all,
            "the desk-bound trader ({trader_grant_all}) sees strictly fewer than admin ({admin_grant_all})"
        );

        // ORACLE 3: a DESKLESS trader narrows to DeskId(0) (house/unowned) — neither
        // booked leg is house-owned, so it sees NOTHING.
        let deskless_grant_all = firm_count(&edge, &deskless, None);
        assert_eq!(
            deskless_grant_all, 0,
            "a deskless trader sees only house/DeskId(0) facts (here: none)"
        );

        // Cross-check: the same explicit-EM-scope assertion under the ADMIN session
        // (no narrowing) ALSO yields exactly the EM leg — proving the trader's
        // grant-all narrowing reproduced the explicit scope's effect, not a wider or
        // emptier set.
        assert_eq!(
            firm_count(&edge, &admin, Some(explicit_em)),
            1,
            "an explicit Desk=EM scope (admin, no narrowing) sees exactly the EM leg"
        );
    }
}
