//! Celnet **cross-shard risk fan-out** — the distributed risk-aggregation
//! *algebra* over the [`celnet_router`] HRW partition map
//! (`docs/RISK-HIERARCHY.md` §3.4, WS-R task #11; `docs/SCALE-OUT.md` §2/§11).
//!
//! # What this crate is (and is honestly NOT)
//!
//! This crate builds the **reduction algebra** that lets a firm-level risk number
//! be computed by **fanning out** across many logical shards and **fanning in**
//! the partial results — and proves that the fan-out result is **identical** to
//! the single-node [`celnet_risk_cube::Cube::firm_aggregate`] reference. It does
//! three things:
//!
//! 1. **Partition** ([`partition_key_of`], [`owner_of`], [`partition_facts`]) —
//!    map each [`RiskFact`] to a [`celnet_router::PartitionKey`] built from its
//!    `(legal-entity, ccy-pair)` dimension keys, then to an owning
//!    [`celnet_router::ReplicaId`] via the HRW [`celnet_router::PartitionMap`] over
//!    a supplied [`celnet_router::ReplicaSet`]. Per `docs/SCALE-OUT.md` §2 the
//!    **currency-pair is the primary partition key**, so *every tenor and strike of
//!    a pair is co-resident on one shard* — surface-shaped roll-up never crosses a
//!    shard. The legal entity is folded in as the sub-key so a pair's flow can be
//!    split per booking entity while still keeping each `(entity, pair)` cell whole.
//! 2. **Shard-local roll-up** ([`LogicalShard`]) — each logical shard owns its
//!    facts in its own [`Cube`] and produces a shard-local additive
//!    [`NodeAggregate`] **plus** retains every constituent position, so the firm
//!    reducer can re-derive non-additive measures.
//! 3. **Cross-shard reduce** ([`FleetReducer`], [`fan_out_aggregate`]) —
//!    - **Additive** (per-ccy net Greeks, the `(tenor × delta)` vega ladder) combine
//!      across shards via [`NodeAggregate::merge_additive`]. Summation is
//!      associative **and** commutative, so the fan-out sum is **exactly** the
//!      single-node `firm_aggregate` (bit-equal up to floating-point *summation
//!      order*, which the deterministic shard ordering pins — see the tests).
//!    - **Non-additive** (VaR / ES, FRTB-SbM curvature, correlation-weighted vega)
//!      **cannot be summed**. We implement the §3.4 spec literally: **gather the
//!      union of every shard's constituent positions at the firm reducer and
//!      re-derive the measure once over the whole set** (the off-hot-path IPV /
//!      firm tier of `docs/SCALE-OUT.md` §2). Because the firm reducer sees the
//!      identical constituent set the single-node cube would, the result is
//!      **exact by construction**, not approximate.
//!
//! ## Why summing shard VaRs would be WRONG (the load-bearing reason this exists)
//!
//! VaR / Expected-Shortfall is a non-linear functional of the **joint** P&L
//! distribution. The VaR of a sum is **not** the sum of VaRs: a long position on
//! one shard and an offsetting short on another *diversify* — their combined tail
//! loss is far smaller than the sum of standalone tail losses (sub-additivity).
//! FRTB-SbM curvature is a `max(CVR⁺, CVR⁺⁻, 0)` reduction, also non-linear.
//! Adding shard-local VaRs would therefore **double-count** independent tails and
//! ignore cross-shard diversification — a materially wrong, conservative-but-false
//! firm number. The only correct distributed answer is to re-gather constituents
//! and re-derive once; that is precisely the documented §3.4 "firm risk runs off
//! the hot path as a separate fan-in" caveat, and what this crate implements.
//!
//! # Honest scope (`docs/SCALE-OUT.md` §0 — no overclaim)
//!
//! What is **BUILT and validated in-process here**: the `(legal-entity, ccy-pair)`
//! → HRW-replica **partitioner**, the **disjoint-cover** shard assignment, the
//! **additive cross-shard merge** algebra, and the **non-additive firm re-gather**
//! — all proven to reconcile to the single-node cube to tight tolerance (additive)
//! and over the identical constituent multiset (non-additive: re-derived once at
//! the firm tier, equal to single-node up to floating-point summation order). The
//! partition reuses `celnet-router`'s real HRW map, so the minimal-reshuffle
//! property holds.
//!
//! What is **explicitly DESIGNED-ONLY and NOT built here** (per `docs/SCALE-OUT.md`
//! §0's "built vs designed" table): the **physical cross-node transport** (the
//! intra-DC RPC that ships partial aggregates between machines), the **replicated
//! event log** (Raft/Aeron-style), and **hot-standby/failover at the process
//! level**. A [`LogicalShard`] is an **in-process logical shard** — a `Cube` that
//! *would* live on a separate node in production. There are **no sockets, threads,
//! or RPC** in this crate pretending to be a live multi-node cluster; this is the
//! deterministic **aggregation algebra**, validated locally, that such transport
//! would carry. Calling [`fan_out_aggregate`] runs the whole fan-out/fan-in in one
//! process so the algebra can be unit-tested for exact reconciliation.
//!
//! # Determinism
//!
//! Partitioning is a pure function of `(fact, replica set)` via the router's frozen
//! HRW mixer; the reducer visits shards in a fixed (sorted-by-[`ReplicaId`]) order
//! and merges in that order, so the fan-out aggregate is **bit-reproducible** for a
//! fixed fact set and replica set (the tests assert byte-identity).
//!
//! # Provenance & naming
//!
//! Cube/star-schema framing and the additive-vs-non-additive split follow
//! `docs/RISK-HIERARCHY.md` §2.5/§3.4; HRW partitioning follows `docs/SCALE-OUT.md`
//! §2. Provenance is in doc comments only; no method/person/vendor name appears in
//! any public identifier (guardrail #8).

#![forbid(unsafe_code)]

use celnet_risk_cube::{
    Cube, DimensionId, NodeAggregate, RiskFact, Scenario, VarEs, VegaPillarMap,
};
use celnet_router::{
    BookId, PartitionKey, PartitionMap, ReplicaId, ReplicaSet, RouteError, TenantId,
};
use celnet_types::CcyPair;

/// Build the [`celnet_router::PartitionKey`] for a fact from its
/// `(legal-entity, ccy-pair)` dimension keys (`docs/RISK-HIERARCHY.md` §3.4,
/// `docs/SCALE-OUT.md` §2).
///
/// The **currency-pair is the primary partition key** — all tenors and strikes of
/// a pair fold to the same key, so a pair's whole surface-shaped flow is
/// co-resident on one shard and surface rebuild never crosses a node (§2). The
/// **legal entity** is folded in as the router's tenant sub-key, so a pair can be
/// split per booking entity while every `(entity, pair)` cell stays whole. The
/// book is **deliberately not** in the key here: putting it in would sub-shard a
/// pair's risk below the `(entity, pair)` cell and split a book's cross-tenor risk
/// — the partition granularity we want for risk aggregation is exactly
/// `(entity, pair)`.
///
/// ## Documented tension (§2, not hidden)
///
/// Partitioning by pair maximizes pricing/surface locality but **splits a tenant's
/// cross-pair risk across shards**: a desk long EURUSD and short GBPUSD has those
/// two legs on (potentially) different shards. That is exactly why joint/firm risk
/// runs **off the hot path** as a fan-in here ([`FleetReducer`]) rather than by
/// forcing everything onto one shard — the additive merge nets the legs at the
/// firm reducer, and the non-additive firm re-gather sees both legs jointly.
#[must_use]
pub fn partition_key_of(fact: &RiskFact) -> PartitionKey {
    PartitionKey::pair(fact.key.ccy_pair).with_tenant(TenantId(u64::from(fact.key.entity.0)))
}

/// Build the partition key from an explicit `(legal-entity, ccy-pair)` pair —
/// the same mapping as [`partition_key_of`], for callers routing without a full
/// [`RiskFact`] in hand.
#[must_use]
pub fn partition_key_for(entity: u32, pair: CcyPair) -> PartitionKey {
    PartitionKey::pair(pair).with_tenant(TenantId(u64::from(entity)))
}

/// A finer partition key that additionally sub-shards a hot `(entity, pair)` cell
/// by **book** (`docs/SCALE-OUT.md` §2 "sub-shard hot pairs by book/tenant"). This
/// is offered for capacity balancing of a saturating pair; it **breaks** the
/// `(entity, pair)` co-residency guarantee that [`partition_key_of`] gives, so use
/// it only where a single `(entity, pair)` cell exceeds a shard. The reducer
/// reconciles identically either way (the algebra is partition-shape-agnostic).
#[must_use]
pub fn partition_key_sub_book(fact: &RiskFact) -> PartitionKey {
    PartitionKey::pair(fact.key.ccy_pair)
        .with_tenant(TenantId(u64::from(fact.key.entity.0)))
        .with_book(BookId(u64::from(fact.key.book.0)))
}

/// The **owning replica** of a fact under the live membership: route its
/// `(entity, pair)` partition key through the HRW [`PartitionMap`], applying
/// hot-standby / HRW failover (`docs/SCALE-OUT.md` §2/§3).
///
/// # Errors
/// Propagates [`RouteError`] when the replica set is empty or every replica (and
/// every declared standby) is down.
pub fn owner_of(fact: &RiskFact, map: &PartitionMap<'_>) -> Result<ReplicaId, RouteError> {
    Ok(map.route(partition_key_of(fact))?.replica)
}

/// The **natural HRW owner** of a fact, ignoring health — the stable home a fact
/// belongs to. `None` only if the replica set is empty. Used to assign facts to
/// logical shards for aggregation (where every replica is, by definition, "up":
/// the aggregation tier reads from each shard's owned slice).
#[must_use]
pub fn natural_owner_of(fact: &RiskFact, map: &PartitionMap<'_>) -> Option<ReplicaId> {
    map.natural_owner(partition_key_of(fact))
}

/// A function mapping a fact to its partition key — the partition strategy seam.
/// Defaults to [`partition_key_of`] (`(entity, pair)` co-residency). Supply a
/// different strategy (e.g. [`partition_key_sub_book`]) to sub-shard hot pairs.
pub type PartitionStrategy = fn(&RiskFact) -> PartitionKey;

/// One **in-process logical shard**: the slice of the firm's facts the HRW map
/// assigned to a single [`ReplicaId`], held in its own [`Cube`].
///
/// In the target topology (`docs/SCALE-OUT.md` §2) this is a separate
/// `celnet-engine` node owning a disjoint partition slice. Here it is a local
/// `Cube` so the fan-out **algebra** can be exercised and reconciled deterministically;
/// the cross-node transport that would connect real shards is designed-only (§0).
#[derive(Debug, Clone)]
pub struct LogicalShard {
    replica: ReplicaId,
    cube: Cube,
}

impl LogicalShard {
    /// An empty shard owned by `replica`.
    #[must_use]
    pub fn new(replica: ReplicaId) -> Self {
        Self {
            replica,
            cube: Cube::new(),
        }
    }

    /// The replica this shard belongs to.
    #[must_use]
    pub const fn replica(&self) -> ReplicaId {
        self.replica
    }

    /// Read-only access to the shard's fact store.
    #[must_use]
    pub const fn cube(&self) -> &Cube {
        &self.cube
    }

    /// Add one fact this shard owns.
    pub fn upsert(&mut self, fact: RiskFact) {
        self.cube.upsert(fact);
    }

    /// The number of facts this shard owns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cube.len()
    }

    /// Whether this shard holds no facts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cube.is_empty()
    }

    /// This shard's **additive** firm-level roll-up, retaining its constituent
    /// positions for the firm reducer's non-additive re-derivation.
    pub fn local_aggregate<P: VegaPillarMap>(&self, pillars: &P) -> NodeAggregate {
        self.cube.firm_aggregate(pillars)
    }

    /// This shard's local roll-up grouped by an arbitrary dimension (a shard-local
    /// drill-down). The firm reducer can merge same-group aggregates across shards.
    pub fn local_group_by<P: VegaPillarMap>(
        &self,
        dim: DimensionId,
        pillars: &P,
    ) -> Vec<NodeAggregate> {
        self.cube.group_by(dim, pillars)
    }
}

/// The fan-out / fan-in **cross-shard reducer** (`docs/RISK-HIERARCHY.md` §3.4).
///
/// Holds the logical shards produced by [`partition_facts`] in **deterministic
/// owner order** (sorted by [`ReplicaId`]) so reductions are reproducible. It is
/// the in-process stand-in for the firm-risk fan-in tier that, in the target
/// topology, would gather partial results from real shards over the (designed-only)
/// cross-node transport.
#[derive(Debug, Clone)]
pub struct FleetReducer {
    /// Shards in ascending [`ReplicaId`] order (deterministic reduction order).
    shards: Vec<LogicalShard>,
}

impl FleetReducer {
    /// The shards, in ascending replica order.
    #[must_use]
    pub fn shards(&self) -> &[LogicalShard] {
        &self.shards
    }

    /// The number of non-empty logical shards.
    #[must_use]
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// The total fact count across all shards (== the input fact count, since the
    /// partition is a disjoint cover).
    #[must_use]
    pub fn total_facts(&self) -> usize {
        self.shards.iter().map(LogicalShard::len).sum()
    }

    /// **Additive fan-out**: each shard rolls up locally, then the firm aggregate
    /// is the [`NodeAggregate::merge_additive`] of every shard's roll-up, in
    /// deterministic owner order. Associative + commutative → **exactly** the
    /// single-node [`Cube::firm_aggregate`] over the union of facts (up to
    /// floating-point summation order, which the fixed order pins).
    pub fn fan_in_additive<P: VegaPillarMap>(&self, pillars: &P) -> NodeAggregate {
        let mut iter = self.shards.iter();
        let Some(first) = iter.next() else {
            // No shards (empty book): an empty firm aggregate.
            return Cube::new().firm_aggregate(pillars);
        };
        let mut acc = first.local_aggregate(pillars);
        for shard in iter {
            acc.merge_additive(&shard.local_aggregate(pillars));
        }
        acc
    }

    /// **Non-additive firm re-gather**: gather the union of every shard's
    /// constituent positions and re-derive a measure **once** over the whole set
    /// (`docs/RISK-HIERARCHY.md` §3.4; the off-hot-path firm tier of
    /// `docs/SCALE-OUT.md` §2). This is the **only** correct distributed answer for
    /// a non-additive measure — see the crate-level "why summing shard VaRs would
    /// be wrong" note. The gathered aggregate carries exactly the constituents the
    /// single-node cube would hold, so any non-additive reducer applied to it is
    /// **exact by construction**.
    ///
    /// The returned [`NodeAggregate`] is the firm node over the gathered union; pass
    /// it to [`FleetReducer::firm_var_es`], [`FleetReducer::firm_curvature_spot`],
    /// or directly to a [`Cube`] non-additive reducer.
    pub fn gather_firm_node<P: VegaPillarMap>(&self, pillars: &P) -> NodeAggregate {
        // The gathered node IS the additive fan-in: merge_additive concatenates the
        // constituent positions/leaves, so the additive numbers and the gathered
        // constituent set are produced in one pass — the same node the firm reducer
        // re-derives non-additive measures over.
        self.fan_in_additive(pillars)
    }

    /// Firm-level **VaR / ES** by the §3.4 re-gather: re-derive once (full
    /// bump-and-revalue oracle) over the union of all shards' constituents.
    /// Exact — never a sum of shard VaRs.
    pub fn firm_var_es<P: VegaPillarMap>(
        &self,
        pillars: &P,
        scenarios: &[Scenario],
        alpha: f64,
    ) -> VarEs {
        let firm = self.gather_firm_node(pillars);
        Cube::node_var_es(&firm, scenarios, alpha)
    }

    /// Firm-level **VaR / ES** by the AAD sensitivity scale path, re-gathered over
    /// the union of constituents (the fast lens; reconcile to [`Self::firm_var_es`]
    /// per the documented Taylor regime).
    pub fn firm_var_es_sensitivity<P: VegaPillarMap>(
        &self,
        pillars: &P,
        scenarios: &[Scenario],
        alpha: f64,
    ) -> VarEs {
        let firm = self.gather_firm_node(pillars);
        Cube::node_var_es_sensitivity(&firm, scenarios, alpha)
    }

    /// Firm-level **FRTB-SbM spot curvature** by the §3.4 re-gather: re-derive once
    /// over the union of all shards' constituents. Exact — never a sum of shard
    /// curvatures (curvature is a non-linear `max`).
    pub fn firm_curvature_spot<P: VegaPillarMap>(&self, pillars: &P, rw: f64) -> f64 {
        let firm = self.gather_firm_node(pillars);
        Cube::node_curvature_spot(&firm, rw)
    }
}

/// Partition a firm's facts across the HRW map into [`LogicalShard`]s, one per
/// owning [`ReplicaId`], using the default `(entity, pair)` strategy.
///
/// Each fact is routed by [`natural_owner_of`] (the stable HRW home) and `upsert`ed
/// into its owner's shard. The result is a **disjoint cover**: every fact lands on
/// exactly one shard, the union of shard fact-sets is the input, and all facts of a
/// `(entity, pair)` cell are co-resident (the §2 guarantee). Shards are returned
/// sorted by [`ReplicaId`] inside a [`FleetReducer`] for deterministic reduction.
///
/// # Errors
/// [`RouteError`] if the membership cannot route a fact (empty set). Health is
/// **ignored** for assignment — the aggregation tier reads each shard's owned
/// slice regardless of live health; failover is a routing concern, not an
/// aggregation one.
pub fn partition_facts(
    facts: &[RiskFact],
    replicas: &ReplicaSet,
) -> Result<FleetReducer, RouteError> {
    partition_facts_with(facts, replicas, partition_key_of)
}

/// [`partition_facts`] with an explicit [`PartitionStrategy`] (e.g.
/// [`partition_key_sub_book`] to sub-shard hot pairs). The reduction algebra is
/// partition-shape-agnostic, so any strategy reconciles to the same single-node
/// firm aggregate.
///
/// # Errors
/// [`RouteError`] if the membership cannot route a fact (empty set).
pub fn partition_facts_with(
    facts: &[RiskFact],
    replicas: &ReplicaSet,
    strategy: PartitionStrategy,
) -> Result<FleetReducer, RouteError> {
    if replicas.is_empty() {
        return Err(RouteError::EmptySet);
    }
    let map = PartitionMap::new(replicas);
    // Assign each fact to the natural HRW owner of its partition key.
    let mut shards: Vec<LogicalShard> = Vec::new();
    for fact in facts {
        let owner = map
            .natural_owner(strategy(fact))
            .ok_or(RouteError::EmptySet)?;
        let idx = match shards.iter().position(|s| s.replica() == owner) {
            Some(i) => i,
            None => {
                shards.push(LogicalShard::new(owner));
                shards.len() - 1
            }
        };
        shards[idx].upsert(*fact);
    }
    // Deterministic reduction order: ascending replica id.
    shards.sort_by_key(|s| s.replica().0);
    Ok(FleetReducer { shards })
}

/// The combined firm-level result of one fan-out / fan-in pass: the additive firm
/// roll-up plus a re-gathered non-additive VaR/ES and FRTB-SbM curvature, all
/// reconcilable to the single-node cube (`docs/RISK-HIERARCHY.md` §3.4).
#[derive(Debug, Clone)]
pub struct FleetAggregate {
    /// The additive firm node (net Greeks + vega ladder + the gathered union of
    /// constituents) — `merge_additive` across shards, exact to summation order.
    pub firm: NodeAggregate,
    /// Firm VaR/ES re-derived once over the gathered union (bump-and-revalue
    /// oracle), at the requested `alpha`. Never a sum of shard VaRs.
    pub var_es: VarEs,
    /// Firm FRTB-SbM spot curvature re-derived once over the gathered union, at the
    /// requested risk weight. Never a sum of shard curvatures.
    pub curvature_spot: f64,
    /// The number of logical shards the facts fanned across.
    pub shard_count: usize,
}

/// The headline cross-fleet entry point: **partition → fan-out → fan-in** in one
/// call (`docs/RISK-HIERARCHY.md` §3.4).
///
/// Partitions `facts` across the HRW map (`(entity, pair)` strategy), rolls each
/// logical shard up locally, then reduces: **additive** measures via the
/// associative cross-shard `merge_additive`, and **non-additive** VaR/ES +
/// curvature via the §3.4 firm re-gather (re-derive once over the union of all
/// shards' constituents). The result reconciles to the single-node
/// [`Cube::firm_aggregate`] + [`Cube::node_var_es`] + [`Cube::node_curvature_spot`]
/// (additive to tight tolerance; non-additive exact-to-summation-order over the
/// identical constituent multiset — see [`fan_out_aggregate`]'s tests).
///
/// In-process logical-shard simulation of the fan-out algebra — see the crate-level
/// honest-scope note; no cross-node transport is performed.
///
/// # Errors
/// [`RouteError`] if the membership cannot route a fact (empty replica set).
pub fn fan_out_aggregate<P: VegaPillarMap>(
    facts: &[RiskFact],
    replicas: &ReplicaSet,
    pillars: &P,
    scenarios: &[Scenario],
    alpha: f64,
    curvature_rw: f64,
) -> Result<FleetAggregate, RouteError> {
    let reducer = partition_facts(facts, replicas)?;
    let firm = reducer.gather_firm_node(pillars);
    let var_es = Cube::node_var_es(&firm, scenarios, alpha);
    let curvature_spot = Cube::node_curvature_spot(&firm, curvature_rw);
    Ok(FleetAggregate {
        firm,
        var_es,
        curvature_spot,
        shard_count: reducer.shard_count(),
    })
}

/// The **deployment topology** of the fleet risk tier — whether the fan-out runs
/// entirely **in-process** (every logical shard is a local [`Cube`], the algebra
/// validated here) or would be **distributed** across physical nodes reached via a
/// set of endpoints (the cross-node transport itself is designed-only — see the
/// crate-level honest-scope note and `docs/SCALE-OUT.md` §0).
///
/// This enum names *what* the topology is, not *how* the transport works; the
/// distributed variant carries only the opaque backend endpoints a transport would
/// dial. Parsing here is a **pure** function of its arguments — the server reads any
/// environment/config and passes the resolved strings in (so this stays testable and
/// side-effect-free).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FleetTopology {
    /// All logical shards are co-resident in this process (the validated algebra).
    #[default]
    InProcess,
    /// The shards would live on separate nodes reached via these backend endpoints.
    /// The physical transport is designed-only (`docs/SCALE-OUT.md` §0); the variant
    /// records the membership a transport would dial.
    Distributed {
        /// Opaque backend endpoints (one per node), in the order supplied.
        endpoints: Vec<String>,
    },
}

impl FleetTopology {
    /// Resolve a topology from a mode string and a comma-separated backend list —
    /// a **pure** function (no environment or I/O; the server reads config and passes
    /// the resolved strings in).
    ///
    /// `mode == "distributed"` **and** at least one non-empty comma-split backend ⇒
    /// [`FleetTopology::Distributed`] over those backends (each trimmed; empty
    /// segments dropped). Every other input — any other mode, or `"distributed"` with
    /// no usable backend — resolves to [`FleetTopology::InProcess`], the safe default.
    #[must_use]
    pub fn parse(mode: &str, backends: &str) -> FleetTopology {
        if mode == "distributed" {
            let endpoints: Vec<String> = backends
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if !endpoints.is_empty() {
                return FleetTopology::Distributed { endpoints };
            }
        }
        FleetTopology::InProcess
    }
}

/// An error from pulling a shard's risk through the [`ShardRiskSource`] seam.
///
/// [`FleetError::ShardUnavailable`] exists for the future **distributed** source (a
/// node that fails to answer); the in-process source never returns it (every shard
/// it knows about is locally present). [`FleetError::Route`] wraps the router's
/// partitioning error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetError {
    /// The membership could not route a fact (e.g. empty replica set).
    Route(RouteError),
    /// A shard could not be reached / did not answer (distributed transport only).
    ShardUnavailable(ReplicaId),
}

impl From<RouteError> for FleetError {
    fn from(e: RouteError) -> Self {
        FleetError::Route(e)
    }
}

impl core::fmt::Display for FleetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FleetError::Route(e) => write!(f, "fleet routing error: {e}"),
            FleetError::ShardUnavailable(r) => {
                write!(f, "fleet shard {} unavailable", r.0)
            }
        }
    }
}

impl core::error::Error for FleetError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            FleetError::Route(e) => Some(e),
            FleetError::ShardUnavailable(_) => None,
        }
    }
}

/// A **transport-agnostic source of per-shard risk** — the seam the firm reducer
/// pulls partial aggregates through, identical whether the shards are in-process
/// [`Cube`]s (here) or remote nodes behind a transport (`docs/SCALE-OUT.md` §0).
///
/// The trait is **object-safe** (no generic methods, no `Self`-typed returns), so a
/// reducer can hold a `&dyn ShardRiskSource` and the in-process and (future)
/// distributed sources are interchangeable behind one `dyn`.
///
/// # The additive-vs-constituent data-movement split (`docs/SCALE-OUT.md` §2)
///
/// The two roll-up methods exist to keep the **common path cheap** in a distributed
/// deployment:
///
/// - [`ShardRiskSource::shard_additive`] returns only the **additive** roll-up — net
///   Greeks + the vega ladder — which is `O(shards × small)` to ship and combine. A
///   distributed implementation **omits the constituent positions** from this node
///   (so the book never crosses the wire); callers must therefore **not** rely on
///   `.positions` / `.leaves` being populated on its result.
/// - [`ShardRiskSource::shard_constituents`] returns the roll-up **with every
///   constituent position retained** — `O(positions)` to ship — for the firm
///   **non-additive** re-gather (VaR/ES, FRTB-SbM curvature). This runs **off the hot
///   path** at the firm tier and is called **only** when a non-additive measure is
///   actually needed.
///
/// In-process both methods return the full node (there is nothing to ship); the
/// split is observable only for a distributed source.
pub trait ShardRiskSource {
    /// The shard ids this source exposes, in **deterministic ascending order** (so
    /// the fan-in summation order is reproducible).
    fn shard_ids(&self) -> Vec<ReplicaId>;

    /// A shard's **additive** roll-up — net Greeks + vega ladder, the cheap/common
    /// path. Callers must **not** rely on `.positions`/`.leaves` being populated (a
    /// distributed source omits them to avoid shipping the book).
    ///
    /// # Errors
    /// [`FleetError::ShardUnavailable`] if `shard` is unknown to this source (or, for
    /// a distributed source, unreachable).
    fn shard_additive(&self, shard: ReplicaId) -> Result<NodeAggregate, FleetError>;

    /// A shard's roll-up **with constituent positions retained**, for the firm
    /// non-additive re-gather. Heavier (`O(positions)`); call only when a
    /// non-additive measure is needed.
    ///
    /// # Errors
    /// [`FleetError::ShardUnavailable`] if `shard` is unknown to this source (or, for
    /// a distributed source, unreachable).
    fn shard_constituents(&self, shard: ReplicaId) -> Result<NodeAggregate, FleetError>;

    /// A **zero firm node** (an empty [`NodeAggregate`]) — so the generic reducer can
    /// handle an empty fleet without needing a [`VegaPillarMap`] in hand.
    fn empty_node(&self) -> NodeAggregate;
}

/// The **in-process** [`ShardRiskSource`]: pulls each shard's roll-up straight from
/// the matching [`LogicalShard`]'s local [`Cube`] in a [`FleetReducer`].
///
/// Because everything is local, both [`ShardRiskSource::shard_additive`] and
/// [`ShardRiskSource::shard_constituents`] return the **full** node (positions
/// retained) — there is no wire to keep them off; the additive/constituent split is
/// observable only for a distributed source. The reducer's fixed ascending-replica
/// order is preserved via [`ShardRiskSource::shard_ids`].
#[derive(Debug, Clone, Copy)]
pub struct InProcessShards<'a, P: VegaPillarMap> {
    reducer: &'a FleetReducer,
    pillars: &'a P,
}

impl<'a, P: VegaPillarMap> InProcessShards<'a, P> {
    /// Wrap a [`FleetReducer`] and its pillar map as an in-process risk source.
    #[must_use]
    pub fn new(reducer: &'a FleetReducer, pillars: &'a P) -> Self {
        Self { reducer, pillars }
    }

    /// The matching shard for a replica id, if this source owns it.
    fn shard(&self, shard: ReplicaId) -> Result<&LogicalShard, FleetError> {
        self.reducer
            .shards()
            .iter()
            .find(|s| s.replica() == shard)
            .ok_or(FleetError::ShardUnavailable(shard))
    }
}

impl<P: VegaPillarMap> ShardRiskSource for InProcessShards<'_, P> {
    fn shard_ids(&self) -> Vec<ReplicaId> {
        // The reducer already holds shards in ascending replica order.
        self.reducer
            .shards()
            .iter()
            .map(LogicalShard::replica)
            .collect()
    }

    fn shard_additive(&self, shard: ReplicaId) -> Result<NodeAggregate, FleetError> {
        Ok(self.shard(shard)?.local_aggregate(self.pillars))
    }

    fn shard_constituents(&self, shard: ReplicaId) -> Result<NodeAggregate, FleetError> {
        Ok(self.shard(shard)?.local_aggregate(self.pillars))
    }

    fn empty_node(&self) -> NodeAggregate {
        Cube::new().firm_aggregate(self.pillars)
    }
}

/// **Additive fan-in over the [`ShardRiskSource`] seam.** Pulls each shard's
/// additive roll-up ([`ShardRiskSource::shard_additive`]) in the source's
/// deterministic order and combines them via [`NodeAggregate::merge_additive`].
///
/// Uses only the additive fields (net Greeks + vega ladder) — the cheap/common path;
/// it does **not** depend on `.positions` being populated, so it is correct over a
/// distributed source that omits the book. The result reconciles to the single-node
/// firm additive roll-up exactly (up to summation order).
///
/// # Errors
/// [`FleetError`] if a shard is unavailable or routing failed upstream.
pub fn fan_in_additive_over(src: &dyn ShardRiskSource) -> Result<NodeAggregate, FleetError> {
    let ids = src.shard_ids();
    let mut iter = ids.into_iter();
    let Some(first) = iter.next() else {
        return Ok(src.empty_node());
    };
    let mut acc = src.shard_additive(first)?;
    for id in iter {
        acc.merge_additive(&src.shard_additive(id)?);
    }
    Ok(acc)
}

/// **Firm constituent re-gather over the [`ShardRiskSource`] seam.** Pulls each
/// shard's roll-up **with constituents retained**
/// ([`ShardRiskSource::shard_constituents`]) and merges them, so the result carries
/// the union of every shard's positions — the off-hot-path firm node a non-additive
/// measure (VaR/ES, FRTB-SbM curvature) is re-derived **once** over.
///
/// # Errors
/// [`FleetError`] if a shard is unavailable or routing failed upstream.
pub fn gather_firm_node_over(src: &dyn ShardRiskSource) -> Result<NodeAggregate, FleetError> {
    let ids = src.shard_ids();
    let mut iter = ids.into_iter();
    let Some(first) = iter.next() else {
        return Ok(src.empty_node());
    };
    let mut acc = src.shard_constituents(first)?;
    for id in iter {
        acc.merge_additive(&src.shard_constituents(id)?);
    }
    Ok(acc)
}

/// The headline cross-fleet entry point **over the [`ShardRiskSource`] seam** —
/// mirrors [`fan_out_aggregate`] but pulls partial results through the trait, so it
/// is identical for an in-process or (future) distributed source.
///
/// The additive firm node uses only `net_greeks` + `vega_ladder` from
/// [`ShardRiskSource::shard_additive`]; the non-additive VaR/ES + curvature are
/// re-derived once over the constituent re-gather
/// ([`ShardRiskSource::shard_constituents`]) — the §3.4 firm tier. Reconciles to
/// [`fan_out_aggregate`] (and thus to the single-node cube) for the in-process
/// source.
///
/// # Errors
/// [`FleetError`] if a shard is unavailable or routing failed upstream.
pub fn fan_out_aggregate_over(
    src: &dyn ShardRiskSource,
    scenarios: &[Scenario],
    alpha: f64,
    curvature_rw: f64,
) -> Result<FleetAggregate, FleetError> {
    let additive = fan_in_additive_over(src)?;
    let firm = gather_firm_node_over(src)?;
    let var_es = Cube::node_var_es(&firm, scenarios, alpha);
    let curvature_spot = Cube::node_curvature_spot(&firm, curvature_rw);
    let shard_count = src.shard_ids().len();
    Ok(FleetAggregate {
        firm: NodeAggregate {
            // Additive fields from the cheap path; constituents from the re-gather —
            // matching what fan_out_aggregate's `firm` carries in-process.
            group: firm.group,
            net_greeks: additive.net_greeks,
            vega_ladder: additive.vega_ladder,
            positions: firm.positions,
            leaves: firm.leaves,
        },
        var_es,
        curvature_spot,
        shard_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_risk_cube::{
        BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId,
        TraderId, VegaPillar,
    };
    use celnet_risk_normalize::{CanonicalLeaf, PositionRisk, canonicalize};
    use celnet_router::Replica;
    use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

    fn pair(b: Ccy, q: Ccy) -> CcyPair {
        CcyPair::new(b, q)
    }

    /// A pillar map that buckets by tenor-in-days and a fixed 0.50Δ pillar.
    struct DaysPillar;
    impl VegaPillarMap for DaysPillar {
        fn pillar_of(&self, _leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar {
            let days = (position.inputs.t * 365.0).round() as u32;
            VegaPillar::new(days, 5000)
        }
    }

    fn pos(p: CcyPair, opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
        PositionRisk::new(
            p,
            opt,
            notional,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn fact(
        id: u32,
        trader: u32,
        book: u32,
        desk: u32,
        loc: u32,
        entity: u32,
        position: PositionRisk,
    ) -> RiskFact {
        RiskFact {
            position_id: PositionId(id),
            key: FactKey {
                trader: TraderId(trader),
                book: CubeBookId(book),
                desk: DeskId(desk),
                ccy_pair: position.pair,
                location: LocationId(loc),
                entity: EntityId(entity),
            },
            measure: FactMeasure {
                leaf: canonicalize(&position),
                position,
            },
            surface_version: 1,
        }
    }

    fn replicas(ids: &[u64]) -> ReplicaSet {
        ReplicaSet::new(ids.iter().map(|&i| Replica::up(ReplicaId(i))).collect()).unwrap()
    }

    /// A non-trivial multi-entity / multi-pair book: 3 legal entities × several
    /// pairs × several tenors, with deliberate cross-pair offsets so a non-additive
    /// firm measure genuinely diversifies (and a naive sum-of-shard-VaRs would be
    /// wrong).
    fn firm_book() -> Vec<RiskFact> {
        let eurusd = pair(Ccy::EUR, Ccy::USD);
        let gbpusd = pair(Ccy::GBP, Ccy::USD);
        let usdjpy = pair(Ccy::USD, Ccy::JPY);
        vec![
            // Entity 1 — EURUSD long call + short call (intra-pair partial offset),
            // and a GBPUSD position (different pair → likely different shard).
            fact(
                1,
                10,
                100,
                1,
                1,
                1,
                pos(
                    eurusd,
                    OptionType::Call,
                    12_000_000.0,
                    VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
                ),
            ),
            fact(
                2,
                10,
                100,
                1,
                1,
                1,
                pos(
                    eurusd,
                    OptionType::Call,
                    -5_000_000.0,
                    VanillaInputs::new(1.10, 1.15, 0.095, 0.5, 0.04, 0.02),
                ),
            ),
            fact(
                3,
                11,
                101,
                1,
                1,
                1,
                pos(
                    gbpusd,
                    OptionType::Put,
                    8_000_000.0,
                    VanillaInputs::new(1.27, 1.25, 0.105, 0.75, 0.04, 0.02),
                ),
            ),
            // Entity 2 — USDJPY straddle-ish + EURUSD short (cross-entity, same pair
            // as entity 1's EURUSD → DIFFERENT (entity,pair) cell → may shard apart).
            fact(
                4,
                20,
                200,
                2,
                2,
                2,
                pos(
                    usdjpy,
                    OptionType::Call,
                    9_000_000.0,
                    VanillaInputs::new(156.0, 158.0, 0.11, 1.0, 0.01, 0.05),
                ),
            ),
            fact(
                5,
                20,
                200,
                2,
                2,
                2,
                pos(
                    usdjpy,
                    OptionType::Put,
                    9_000_000.0,
                    VanillaInputs::new(156.0, 154.0, 0.11, 1.0, 0.01, 0.05),
                ),
            ),
            fact(
                6,
                21,
                201,
                2,
                2,
                2,
                pos(
                    eurusd,
                    OptionType::Put,
                    7_000_000.0,
                    VanillaInputs::new(1.10, 1.07, 0.115, 1.5, 0.04, 0.02),
                ),
            ),
            // Entity 3 — GBPUSD short call (offsets entity 1's GBPUSD put curvature),
            // plus EURUSD long for cross-entity netting at the firm reducer.
            fact(
                7,
                30,
                300,
                3,
                3,
                3,
                pos(
                    gbpusd,
                    OptionType::Call,
                    -6_000_000.0,
                    VanillaInputs::new(1.27, 1.30, 0.10, 1.0, 0.04, 0.02),
                ),
            ),
            fact(
                8,
                31,
                301,
                3,
                3,
                3,
                pos(
                    eurusd,
                    OptionType::Call,
                    4_000_000.0,
                    VanillaInputs::new(1.10, 1.11, 0.10, 0.25, 0.04, 0.02),
                ),
            ),
        ]
    }

    /// A moderate spot×vol scenario ladder (the regime daily VaR uses).
    fn ladder() -> Vec<Scenario> {
        let mut v = Vec::new();
        for si in -5..=5 {
            for vj in -4..=4 {
                v.push(Scenario {
                    spot_rel: f64::from(si) * 0.01,
                    vol_abs: f64::from(vj) * 0.005,
                    rate_dom_abs: 0.0,
                    rate_for_abs: 0.0,
                });
            }
        }
        v
    }

    /// Build the single-node reference cube over the whole book.
    fn single_node(facts: &[RiskFact]) -> Cube {
        let mut c = Cube::new();
        for f in facts {
            c.upsert(*f);
        }
        c
    }

    /// **RECONCILIATION — additive fan-out == single-node.** The cross-shard
    /// `merge_additive` fan-in of N>=3 HRW-partitioned shards equals the single
    /// node's `firm_aggregate` to 1e-12 across the full net-Greek set and the vega
    /// ladder.
    #[test]
    fn additive_fan_out_equals_single_node() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);

        let single = single_node(&facts).firm_aggregate(&DaysPillar);
        let reducer = partition_facts(&facts, &set).unwrap();
        let fleet = reducer.fan_in_additive(&DaysPillar);

        // Every additive Greek field reconciles to 1e-12 relative.
        let s = &single.net_greeks;
        let f = &fleet.net_greeks;
        for (a, b, name) in [
            (s.delta_base, f.delta_base, "delta_base"),
            (s.gamma, f.gamma, "gamma"),
            (s.vega, f.vega, "vega"),
            (s.theta, f.theta, "theta"),
            (s.vanna, f.vanna, "vanna"),
            (s.volga, f.volga, "volga"),
            (s.charm, f.charm, "charm"),
            (s.speed, f.speed, "speed"),
            (s.zomma, f.zomma, "zomma"),
            (s.color, f.color, "color"),
            (s.premium_quote, f.premium_quote, "premium_quote"),
        ] {
            assert!(
                is_close(a, b, 1e-12, 1e-6),
                "additive {name}: single {a} vs fleet {b}"
            );
        }
        // Vega ladder: every single-node pillar reconciles in the fleet ladder.
        for (p, v) in single.vega_ladder.pillars() {
            assert!(
                is_close(v, fleet.vega_ladder.vega_in(p), 1e-12, 1e-6),
                "vega pillar {p:?}: single {v} vs fleet {}",
                fleet.vega_ladder.vega_in(p)
            );
        }
        // The fleet gathered exactly the same constituents (disjoint-cover union).
        assert_eq!(fleet.positions.len(), single.positions.len());
        assert_eq!(fleet.positions.len(), facts.len());
        // It genuinely fanned across multiple shards (not all on one).
        assert!(
            reducer.shard_count() >= 3,
            "expected >=3 shards, got {}",
            reducer.shard_count()
        );
    }

    /// **RECONCILIATION — non-additive (VaR/ES) fan-out == single-node, EXACT to
    /// floating-point summation order.** The firm re-gather re-derives VaR/ES once
    /// over the union of every shard's constituents — the **identical constituent
    /// multiset** the single-node cube holds — so it is *the same computation*, NOT
    /// a sum of shard VaRs.
    ///
    /// We assert two things: (1) the gathered firm node holds the **exact same
    /// multiset** of positions as the single node (algebra exactness — the
    /// constituent set is identical), and (2) the resulting VaR/ES matches the
    /// single-node oracle to **1e-12 relative** (≈ a few ULP). The only source of
    /// any sub-ULP difference is the **order** in which `node_pnl` sums each
    /// scenario's per-position P&L (shard order vs fact-insertion order):
    /// floating-point addition is non-associative, so a reordered exact sum can
    /// differ in the last bit. That is a summation-order artifact, **not** an
    /// aggregation error — the measure is re-derived over identical inputs. A
    /// dedicated order-preserving case (`single_shard_nonadditive_is_bit_identical`)
    /// proves bit-equality when the summation order is held fixed.
    #[test]
    fn nonadditive_var_es_fan_out_reconciles_exactly() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let scen = ladder();

        let single_node_node = single_node(&facts).firm_aggregate(&DaysPillar);
        let single = Cube::node_var_es(&single_node_node, &scen, 0.99);

        let reducer = partition_facts(&facts, &set).unwrap();
        let fleet_node = reducer.gather_firm_node(&DaysPillar);
        let fleet = Cube::node_var_es(&fleet_node, &scen, 0.99);

        assert!(single.var > 0.0, "reference must see a tail loss");
        // (1) Identical constituent multiset (algebra exactness): same positions,
        // any order. Compare as sorted notional+spot+strike signatures.
        assert_eq!(fleet_node.positions.len(), single_node_node.positions.len());
        let sig = |ps: &[PositionRisk]| {
            let mut v: Vec<(u64, u64, u64)> = ps
                .iter()
                .map(|p| {
                    (
                        p.notional_base.to_bits(),
                        p.inputs.spot.to_bits(),
                        p.inputs.strike.to_bits(),
                    )
                })
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(
            sig(&fleet_node.positions),
            sig(&single_node_node.positions),
            "fleet must gather the identical constituent multiset"
        );
        // (2) Same measure to a few ULP (summation-order only).
        assert!(
            is_close(fleet.var, single.var, 1e-12, 1e-3),
            "firm VaR fleet {} vs single {}",
            fleet.var,
            single.var
        );
        assert!(is_close(fleet.es, single.es, 1e-12, 1e-3));
    }

    /// **RECONCILIATION — non-additive (curvature) fan-out == single-node**, exact to
    /// summation order (same constituent multiset, re-derived once at the firm tier;
    /// see `nonadditive_var_es_fan_out_reconciles_exactly` for the ULP rationale).
    #[test]
    fn nonadditive_curvature_fan_out_reconciles_exactly() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let rw = 0.18;

        let single_node_node = single_node(&facts).firm_aggregate(&DaysPillar);
        let single = Cube::node_curvature_spot(&single_node_node, rw);
        let fleet = partition_facts(&facts, &set)
            .unwrap()
            .firm_curvature_spot(&DaysPillar, rw);

        assert!(
            is_close(fleet, single, 1e-12, 1e-3),
            "firm curvature fleet {fleet} vs single {single}"
        );
    }

    /// **Bit-exact non-additive reconciliation when summation order is held fixed.**
    /// With all facts on one `(entity, pair)` cell they form a single shard whose
    /// gather order equals the single-node insertion order, so `node_pnl` sums in
    /// the identical order and the firm VaR/ES is **byte-identical** to single-node
    /// — proving the re-gather differs from single-node ONLY by summation order in
    /// the multi-shard case, never by the algebra.
    #[test]
    fn single_shard_nonadditive_is_bit_identical() {
        let eurusd = pair(Ccy::EUR, Ccy::USD);
        let facts: Vec<RiskFact> = (0u32..6)
            .map(|i| {
                fact(
                    i,
                    1,
                    1,
                    1,
                    1,
                    1,
                    pos(
                        eurusd,
                        if i % 2 == 0 {
                            OptionType::Call
                        } else {
                            OptionType::Put
                        },
                        f64::from(i + 1) * 2_000_000.0,
                        VanillaInputs::new(1.10, 1.08 + f64::from(i) * 0.01, 0.10, 1.0, 0.04, 0.02),
                    ),
                )
            })
            .collect();
        let set = replicas(&[1, 2, 3]);
        let scen = ladder();
        let reducer = partition_facts(&facts, &set).unwrap();
        assert_eq!(reducer.shard_count(), 1);

        let single = Cube::node_var_es(
            &single_node(&facts).firm_aggregate(&DaysPillar),
            &scen,
            0.99,
        );
        let fleet = reducer.firm_var_es(&DaysPillar, &scen, 0.99);
        assert_eq!(
            fleet.var.to_bits(),
            single.var.to_bits(),
            "single-shard firm VaR must be bit-identical: fleet {} vs single {}",
            fleet.var,
            single.var
        );
        assert_eq!(fleet.es.to_bits(), single.es.to_bits());
    }

    /// **The headline `fan_out_aggregate` entry point reconciles end-to-end.** One
    /// call partitions + fans out + fans in; its additive firm node matches
    /// single-node `firm_aggregate` to 1e-12, and its non-additive VaR/curvature
    /// match the single-node reducers to summation-order tolerance.
    #[test]
    fn fan_out_aggregate_reconciles_end_to_end() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let scen = ladder();
        let rw = 0.18;

        let single = single_node(&facts).firm_aggregate(&DaysPillar);
        let single_var = Cube::node_var_es(&single, &scen, 0.99);
        let single_cvr = Cube::node_curvature_spot(&single, rw);

        let agg = fan_out_aggregate(&facts, &set, &DaysPillar, &scen, 0.99, rw).unwrap();

        assert!(agg.shard_count >= 3);
        assert!(is_close(
            agg.firm.net_greeks.delta_base,
            single.net_greeks.delta_base,
            1e-12,
            1e-6
        ));
        assert!(is_close(
            agg.firm.net_greeks.vega,
            single.net_greeks.vega,
            1e-12,
            1e-6
        ));
        assert!(is_close(agg.var_es.var, single_var.var, 1e-12, 1e-3));
        assert!(is_close(agg.var_es.es, single_var.es, 1e-12, 1e-3));
        assert!(is_close(agg.curvature_spot, single_cvr, 1e-12, 1e-3));
    }

    /// **Summing shard VaRs would be WRONG.** Proof that the re-gather is not just
    /// cosmetically different from a naive shard-sum: the sum of per-shard VaRs
    /// strictly OVER-states the firm VaR because cross-shard positions diversify.
    /// This is the whole reason the firm tier re-gathers (sub-additivity, §3.4).
    #[test]
    fn summing_shard_vars_overstates_firm_var() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let scen = ladder();
        let reducer = partition_facts(&facts, &set).unwrap();

        // The correct (re-gathered) firm VaR.
        let firm_var = reducer.firm_var_es(&DaysPillar, &scen, 0.99).var;

        // The WRONG naive answer: sum each shard's standalone VaR.
        let naive_sum: f64 = reducer
            .shards()
            .iter()
            .map(|s| {
                let node = s.local_aggregate(&DaysPillar);
                Cube::node_var_es(&node, &scen, 0.99).var
            })
            .sum();

        assert!(firm_var > 0.0 && naive_sum > 0.0);
        // Diversification: the joint firm VaR is strictly below the shard-sum.
        assert!(
            firm_var < naive_sum,
            "firm re-gather VaR {firm_var} must be < naive shard-sum {naive_sum} \
             (sub-additivity / diversification) — summing shard VaRs over-states risk"
        );
    }

    /// **HRW partition is a disjoint cover.** Every fact lands on exactly one shard;
    /// the union of shard fact-sets is the whole book; shards are pairwise disjoint.
    ///
    /// We re-derive each fact's owner independently (the same HRW route the
    /// partitioner used) and bucket the position ids, then assert: (a) every fact is
    /// assigned to exactly one owner, (b) the per-owner counts match the reducer's
    /// shard lens (so the reducer placed each fact on its routed owner and nowhere
    /// else), and (c) the union count equals the input count.
    #[test]
    fn partition_is_disjoint_cover() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5, 6, 7]);
        let map = PartitionMap::new(&set);
        let reducer = partition_facts(&facts, &set).unwrap();

        // Owner per fact (independent re-route) → expected per-shard counts.
        let mut counts: Vec<(ReplicaId, usize)> = Vec::new();
        let mut seen: Vec<PositionId> = Vec::new();
        for f in &facts {
            assert!(
                !seen.contains(&f.position_id),
                "duplicate position id {:?} in book",
                f.position_id
            );
            seen.push(f.position_id);
            let owner = natural_owner_of(f, &map).unwrap();
            match counts.iter_mut().find(|(r, _)| *r == owner) {
                Some((_, n)) => *n += 1,
                None => counts.push((owner, 1)),
            }
        }
        // Union covers every fact exactly once.
        assert_eq!(seen.len(), facts.len());
        assert_eq!(
            reducer.total_facts(),
            facts.len(),
            "no fact dropped/duplicated"
        );

        // Each reducer shard's len matches the independently-routed owner count
        // (disjoint placement: fact on its routed owner and nowhere else).
        for shard in reducer.shards() {
            let expect = counts
                .iter()
                .find(|(r, _)| *r == shard.replica())
                .map_or(0, |(_, n)| *n);
            assert_eq!(
                shard.len(),
                expect,
                "shard {:?} holds {} facts, HRW routes {} to it",
                shard.replica(),
                shard.len(),
                expect
            );
        }
        // The reducer has exactly the shards that own at least one fact.
        assert_eq!(reducer.shard_count(), counts.len());
    }

    /// **Pair co-residency (§2).** Every fact of a given `(entity, pair)` cell lands
    /// on the same shard — the partition key folds all tenors/strikes of a pair to
    /// one owner. We assert all facts sharing `(entity, ccy_pair)` share a replica.
    #[test]
    fn entity_pair_cell_is_co_resident() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let map = PartitionMap::new(&set);

        // Group facts by (entity, pair) and assert one owner per cell.
        let mut cells: Vec<((u32, CcyPair), ReplicaId)> = Vec::new();
        for f in &facts {
            let owner = natural_owner_of(f, &map).unwrap();
            let cell = (f.key.entity.0, f.key.ccy_pair);
            if let Some((_, o)) = cells.iter().find(|(c, _)| *c == cell) {
                assert_eq!(
                    *o, owner,
                    "(entity {:?}, pair {:?}) split across shards {:?} and {:?}",
                    cell.0, cell.1, o, owner
                );
            } else {
                cells.push((cell, owner));
            }
        }
        // And all tenors/strikes of EURUSD entity-1 (facts 1 & 2) co-reside.
        let o1 = natural_owner_of(&facts[0], &map).unwrap();
        let o2 = natural_owner_of(&facts[1], &map).unwrap();
        assert_eq!(o1, o2, "EURUSD entity-1 tenors must co-reside");
    }

    /// **HRW minimal reshuffle on adding a replica.** Adding a 6th replica to a
    /// 5-replica fleet moves only a small fraction of the `(entity, pair)` cells
    /// (the HRW property reused from celnet-router) — and crucially, a cell only
    /// ever moves *onto* the new replica, never between two old ones.
    #[test]
    fn adding_replica_reshuffles_minimally() {
        // Use many synthetic (entity, pair) cells for a statistically meaningful
        // reshuffle fraction.
        let pairs = [
            pair(Ccy::EUR, Ccy::USD),
            pair(Ccy::GBP, Ccy::USD),
            pair(Ccy::USD, Ccy::JPY),
            pair(Ccy::AUD, Ccy::USD),
            pair(Ccy::USD, Ccy::CHF),
        ];
        let keys: Vec<PartitionKey> = (0u32..200)
            .flat_map(|e| pairs.iter().map(move |&p| partition_key_for(e, p)))
            .collect();

        let before = replicas(&[1, 2, 3, 4, 5]);
        let after = replicas(&[1, 2, 3, 4, 5, 6]);
        let mb = PartitionMap::new(&before);
        let ma = PartitionMap::new(&after);

        let mut moved = 0usize;
        for &k in &keys {
            let o_before = mb.natural_owner(k).unwrap();
            let o_after = ma.natural_owner(k).unwrap();
            if o_before != o_after {
                moved += 1;
                // A moved key must have moved ONTO the new replica (6) — HRW never
                // shuffles a key between two incumbents on a pure add.
                assert_eq!(
                    o_after,
                    ReplicaId(6),
                    "HRW add moved a key between incumbents (not onto the new node)"
                );
            }
        }
        let frac = moved as f64 / keys.len() as f64;
        // Expected ~1/6 ≈ 0.167; allow a generous band for finite-sample variance.
        assert!(
            (0.05..0.30).contains(&frac),
            "reshuffle fraction {frac} should be near 1/N (~0.167)"
        );
    }

    /// **Determinism / bit-reproducibility.** The fan-out additive aggregate and the
    /// firm non-additive VaR are byte-identical across two independent partition +
    /// reduce runs for a fixed fact set and replica set.
    #[test]
    fn fan_out_is_bit_reproducible() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let scen = ladder();

        let r1 = partition_facts(&facts, &set).unwrap();
        let r2 = partition_facts(&facts, &set).unwrap();

        let a1 = r1.fan_in_additive(&DaysPillar);
        let a2 = r2.fan_in_additive(&DaysPillar);
        assert_eq!(
            a1.net_greeks.delta_base.to_bits(),
            a2.net_greeks.delta_base.to_bits()
        );
        assert_eq!(a1.net_greeks.vega.to_bits(), a2.net_greeks.vega.to_bits());
        assert_eq!(
            a1.vega_ladder.total().to_bits(),
            a2.vega_ladder.total().to_bits()
        );

        let v1 = r1.firm_var_es(&DaysPillar, &scen, 0.99);
        let v2 = r2.firm_var_es(&DaysPillar, &scen, 0.99);
        assert_eq!(v1.var.to_bits(), v2.var.to_bits());
        assert_eq!(v1.es.to_bits(), v2.es.to_bits());
    }

    /// **Additive fan-out is bit-identical to single-node** (not merely close) when
    /// the firm has a single shard's worth of facts in HRW order — proving the merge
    /// preserves the exact summation the single cube performs for a co-resident set.
    #[test]
    fn single_shard_fan_out_is_bit_identical() {
        // All facts on one (entity, pair) → one shard → identical summation order.
        let eurusd = pair(Ccy::EUR, Ccy::USD);
        let facts: Vec<RiskFact> = (0u32..6)
            .map(|i| {
                fact(
                    i,
                    1,
                    1,
                    1,
                    1,
                    1,
                    pos(
                        eurusd,
                        if i % 2 == 0 {
                            OptionType::Call
                        } else {
                            OptionType::Put
                        },
                        f64::from(i + 1) * 1_000_000.0,
                        VanillaInputs::new(1.10, 1.10 + f64::from(i) * 0.01, 0.10, 1.0, 0.04, 0.02),
                    ),
                )
            })
            .collect();
        let set = replicas(&[1, 2, 3]);
        let reducer = partition_facts(&facts, &set).unwrap();
        assert_eq!(reducer.shard_count(), 1, "one (entity,pair) → one shard");

        let single = single_node(&facts).firm_aggregate(&DaysPillar);
        let fleet = reducer.fan_in_additive(&DaysPillar);
        assert_eq!(
            fleet.net_greeks.delta_base.to_bits(),
            single.net_greeks.delta_base.to_bits()
        );
        assert_eq!(
            fleet.net_greeks.gamma.to_bits(),
            single.net_greeks.gamma.to_bits()
        );
        assert_eq!(
            fleet.net_greeks.vega.to_bits(),
            single.net_greeks.vega.to_bits()
        );
    }

    /// Empty membership is a typed error, not a panic.
    #[test]
    fn empty_replica_set_errors() {
        let facts = firm_book();
        let empty = ReplicaSet::default();
        assert_eq!(
            partition_facts(&facts, &empty).map(|_| ()),
            Err(RouteError::EmptySet)
        );
    }

    /// An empty book fans out to an empty firm aggregate without panicking.
    #[test]
    fn empty_book_fans_out_to_empty() {
        let set = replicas(&[1, 2, 3]);
        let reducer = partition_facts(&[], &set).unwrap();
        assert_eq!(reducer.shard_count(), 0);
        let agg = reducer.fan_in_additive(&DaysPillar);
        assert_eq!(agg.net_greeks.delta_base, 0.0);
        assert!(agg.positions.is_empty());
    }

    // ------------------------------------------------------------------------
    // Transport-seam tests (additive `ShardRiskSource` layer). These prove the
    // generic seam path EQUALS the existing free-fn path; the 13 tests above are
    // unchanged.
    // ------------------------------------------------------------------------

    /// **The generic seam reconciles to the existing free-fn path.** Build a
    /// `FleetReducer` over `firm_book()`, wrap it in `InProcessShards`, and assert
    /// `fan_out_aggregate_over(&src, …)` matches `fan_out_aggregate(facts, …)` —
    /// additive to 1e-12, non-additive bit-identical (same source, same order).
    #[test]
    fn seam_fan_out_equals_free_fn() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let scen = ladder();
        let rw = 0.18;

        let free = fan_out_aggregate(&facts, &set, &DaysPillar, &scen, 0.99, rw).unwrap();

        let reducer = partition_facts(&facts, &set).unwrap();
        let src = InProcessShards::new(&reducer, &DaysPillar);
        let seam = fan_out_aggregate_over(&src, &scen, 0.99, rw).unwrap();

        assert_eq!(seam.shard_count, free.shard_count);
        assert!(seam.shard_count >= 3);

        // Additive firm node: every Greek reconciles to 1e-12.
        let a = &seam.firm.net_greeks;
        let b = &free.firm.net_greeks;
        for (x, y, name) in [
            (a.delta_base, b.delta_base, "delta_base"),
            (a.gamma, b.gamma, "gamma"),
            (a.vega, b.vega, "vega"),
            (a.theta, b.theta, "theta"),
            (a.vanna, b.vanna, "vanna"),
            (a.volga, b.volga, "volga"),
            (a.charm, b.charm, "charm"),
            (a.premium_quote, b.premium_quote, "premium_quote"),
        ] {
            assert!(is_close(x, y, 1e-12, 1e-6), "seam {name}: {x} vs {y}");
        }
        for (p, v) in free.firm.vega_ladder.pillars() {
            assert!(
                is_close(seam.firm.vega_ladder.vega_in(p), v, 1e-12, 1e-6),
                "seam vega pillar {p:?}"
            );
        }
        assert_eq!(seam.firm.positions.len(), free.firm.positions.len());

        // Non-additive: same constituent source and same (ascending-replica) order
        // as the free fn ⇒ bit-identical VaR/ES/curvature.
        assert_eq!(seam.var_es.var.to_bits(), free.var_es.var.to_bits());
        assert_eq!(seam.var_es.es.to_bits(), free.var_es.es.to_bits());
        assert_eq!(seam.curvature_spot.to_bits(), free.curvature_spot.to_bits());
    }

    /// **Additive seam fan-in == single-node**, to 1e-12 — independent of the
    /// non-additive path.
    #[test]
    fn seam_additive_equals_single_node() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);

        let single = single_node(&facts).firm_aggregate(&DaysPillar);
        let reducer = partition_facts(&facts, &set).unwrap();
        let src = InProcessShards::new(&reducer, &DaysPillar);
        let seam = fan_in_additive_over(&src).unwrap();

        assert!(is_close(
            seam.net_greeks.delta_base,
            single.net_greeks.delta_base,
            1e-12,
            1e-6
        ));
        assert!(is_close(
            seam.net_greeks.vega,
            single.net_greeks.vega,
            1e-12,
            1e-6
        ));
        // Constituent re-gather carries the full union (matches single-node count).
        let gathered = gather_firm_node_over(&src).unwrap();
        assert_eq!(gathered.positions.len(), single.positions.len());
        assert_eq!(gathered.positions.len(), facts.len());
    }

    /// **`ShardRiskSource` is object-safe.** Bind through `&dyn ShardRiskSource` and
    /// drive the generic reducers — this would not compile if the trait were not
    /// object-safe, and proves the in-process source is `dyn`-usable.
    #[test]
    fn shard_risk_source_is_object_safe() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let scen = ladder();
        let reducer = partition_facts(&facts, &set).unwrap();
        let src = InProcessShards::new(&reducer, &DaysPillar);

        let s: &dyn ShardRiskSource = &src;
        let ids = s.shard_ids();
        // Deterministic ascending order.
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
        assert!(ids.len() >= 3);
        // Drive the generic reducers through the `dyn` reference.
        let add = fan_in_additive_over(s).unwrap();
        let gathered = gather_firm_node_over(s).unwrap();
        let agg = fan_out_aggregate_over(s, &scen, 0.99, 0.18).unwrap();
        assert_eq!(gathered.positions.len(), facts.len());
        assert!(is_close(
            agg.firm.net_greeks.vega,
            add.net_greeks.vega,
            1e-12,
            1e-6
        ));
        // An unknown shard id is a typed ShardUnavailable, never a panic.
        let bogus = ReplicaId(9_999);
        assert_eq!(
            s.shard_additive(bogus),
            Err(FleetError::ShardUnavailable(bogus))
        );
    }

    /// **Empty fleet through the seam.** An empty book partitions to zero shards;
    /// the generic reducer returns an empty firm node without needing pillars at the
    /// call site (the source supplies `empty_node`).
    #[test]
    fn seam_empty_fleet() {
        let set = replicas(&[1, 2, 3]);
        let scen = ladder();
        let reducer = partition_facts(&[], &set).unwrap();
        let src = InProcessShards::new(&reducer, &DaysPillar);
        let s: &dyn ShardRiskSource = &src;

        assert!(s.shard_ids().is_empty());
        let add = fan_in_additive_over(s).unwrap();
        assert_eq!(add.net_greeks.delta_base, 0.0);
        assert!(add.positions.is_empty());
        let agg = fan_out_aggregate_over(s, &scen, 0.99, 0.18).unwrap();
        assert_eq!(agg.shard_count, 0);
        assert!(agg.firm.positions.is_empty());
    }

    /// **`FleetTopology::parse` is a pure resolver.** Distributed only when mode is
    /// exactly "distributed" with at least one non-empty backend; everything else is
    /// the in-process default.
    #[test]
    fn topology_parse_cases() {
        assert_eq!(FleetTopology::default(), FleetTopology::InProcess);
        assert_eq!(
            FleetTopology::parse("in-process", "a,b"),
            FleetTopology::InProcess
        );
        assert_eq!(
            FleetTopology::parse("distributed", ""),
            FleetTopology::InProcess
        );
        assert_eq!(
            FleetTopology::parse("distributed", " , , "),
            FleetTopology::InProcess
        );
        assert_eq!(
            FleetTopology::parse("DISTRIBUTED", "a"),
            FleetTopology::InProcess,
            "mode match is exact"
        );
        assert_eq!(
            FleetTopology::parse("distributed", "node-a:7000, node-b:7000 , ,node-c:7000"),
            FleetTopology::Distributed {
                endpoints: vec![
                    "node-a:7000".to_string(),
                    "node-b:7000".to_string(),
                    "node-c:7000".to_string(),
                ]
            }
        );
    }

    /// **`FleetError` Display + Error source.** Routing errors wrap and forward; an
    /// unavailable shard names its replica.
    #[test]
    fn fleet_error_display() {
        let route = FleetError::from(RouteError::EmptySet);
        assert_eq!(
            route.to_string(),
            "fleet routing error: replica set is empty"
        );
        let unavail = FleetError::ShardUnavailable(ReplicaId(7));
        assert_eq!(unavail.to_string(), "fleet shard 7 unavailable");
        // Error source: Route forwards, ShardUnavailable is a leaf.
        use core::error::Error;
        assert!(route.source().is_some());
        assert!(unavail.source().is_none());
    }
}
