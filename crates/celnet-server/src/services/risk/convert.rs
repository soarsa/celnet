//! Wire ↔ domain mapping for [`RiskService`](super::RiskEdge).
//!
//! The single place the `celnet-proto` risk messages cross into the
//! `celnet-risk-*` domain types and back. Keeping it here means the gRPC edge and
//! the WS mirror dispatch onto the **same** mapping — one contract, two encodings
//! (guardrail #9). Every mapping is total or fails with a typed `tonic::Status`
//! `invalid_argument` (a malformed request is rejected loudly, never silently
//! coerced).

use std::collections::HashMap;

use celnet_entitlements::{Principal, Rule};
use celnet_limits::{Enforcement, LimitMetric, RagStatus};
use celnet_proto::{AdditiveRisk as WireAdditive, convert::WireError};
use celnet_proto::{
    AttributionRecord, CcyExposureLeg, EntitlementPrincipal, NonAdditiveRisk as WireNonAdditive,
    OrgKey, ReportingNumeraire, RiskDimension, RiskNode as WireRiskNode, RiskPosition, RiskScope,
    VegaLadderBucket, VegaPillar as WireVegaPillar,
};
use celnet_proto::{Enforcement as WireEnforcement, LimitMetricKind, RagStatus as WireRag};
use celnet_risk_cube::{
    BookId, DeskId, DimensionId, EntityId, FactKey, FactMeasure, LocationId, NodeAggregate,
    PositionId, RiskFact, TraderId, VegaPillar,
};
use celnet_risk_normalize::{NumeraireError, PositionRisk, SpotResolver, canonicalize};
use celnet_types::{
    Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
};
use tonic::Status;

/// Map a wire [`RiskDimension`] enum value (the proto `i32`) onto an optional cube
/// [`DimensionId`]. `FIRM` has no `DimensionId` — it is the implicit apex, handled
/// by `firm_aggregate` — so it maps to `None`. An unknown value is an error.
///
/// # Errors
/// `invalid_argument` if the `i32` is not a known `RiskDimension`.
pub fn dimension_of(dim: i32) -> Result<Option<DimensionId>, Status> {
    let d = RiskDimension::try_from(dim)
        .map_err(|_| Status::invalid_argument(format!("unknown RiskDimension {dim}")))?;
    Ok(match d {
        RiskDimension::Firm => None,
        RiskDimension::Trader => Some(DimensionId::Trader),
        RiskDimension::Book => Some(DimensionId::Book),
        RiskDimension::Desk => Some(DimensionId::Desk),
        RiskDimension::Underlying => Some(DimensionId::Underlying),
        RiskDimension::Location => Some(DimensionId::Location),
        RiskDimension::Entity => Some(DimensionId::Entity),
    })
}

/// The wire enum value for a cube [`DimensionId`] (the inverse of [`dimension_of`]
/// for the non-apex axes).
#[must_use]
pub fn dimension_to_wire(dim: DimensionId) -> i32 {
    let d = match dim {
        DimensionId::Trader => RiskDimension::Trader,
        DimensionId::Book => RiskDimension::Book,
        DimensionId::Desk => RiskDimension::Desk,
        DimensionId::Underlying => RiskDimension::Underlying,
        DimensionId::Location => RiskDimension::Location,
        DimensionId::Entity => RiskDimension::Entity,
    };
    d as i32
}

/// Map a wire [`RiskScope`] onto an entitlement [`Rule`] pinning that one axis. A
/// `FIRM`-dimension scope (or a scope with no axis) covers everything — the firm
/// root rule.
///
/// # Errors
/// `invalid_argument` if the scope's dimension is an unknown enum value.
pub fn scope_to_rule(scope: &RiskScope) -> Result<Rule, Status> {
    match dimension_of(scope.dimension)? {
        None => Ok(Rule::firm()),
        Some(dim) => Ok(Rule::on(dim, scope.value)),
    }
}

/// Resolve a request's optional [`EntitlementPrincipal`] into a [`Principal`].
///
/// **Post-boundary semantics**: this mapping runs strictly *after* the
/// authorization decision boundary ([`crate::services::access::authorize`]) has
/// allowed the request — the service trait entry denies an absent principal by
/// default, so the absent ⇒ grant-all arm here is reachable from a client only
/// through the explicit, audited permissive dev-mode, or internally from the
/// federation's staged re-derivation (whose gathered set was already
/// entitlement-pruned at the backends, so grant-all over it is exact, not a
/// bypass). A present principal with `grant_all=false` and no grants admits
/// nothing (deny-by-default predicate).
///
/// # Errors
/// `invalid_argument` if any rule's scope carries an unknown dimension.
pub fn principal_of(principal: Option<&EntitlementPrincipal>) -> Result<Principal, Status> {
    let Some(p) = principal else {
        // Absent ⇒ grant-all, ONLY post-boundary (permissive dev-mode or the
        // federation's already-pruned staged set — doc comment above).
        return Ok(Principal::grant_all());
    };
    if p.grant_all {
        // Grant-all, but denies still apply (deny wins) — layer in any barriers.
        let mut principal = Principal::grant_all();
        for d in &p.denies {
            principal = principal.deny(rule_of(d)?);
        }
        return Ok(principal);
    }
    // Scoped (deny-by-default): start from no grants, add each grant, then denies.
    let mut principal = Principal::scoped();
    for g in &p.grants {
        principal = principal.grant(rule_of(g)?);
    }
    for d in &p.denies {
        principal = principal.deny(rule_of(d)?);
    }
    Ok(principal)
}

/// **Session-derived desk narrowing** (item B §3): intersect a caller's asserted
/// (or grant-all-defaulted) `base` principal with their session's desk subtree, so
/// a non-admin desk-bound session cannot widen to a firm-wide view via an
/// omitted/grant-all body principal.
///
/// * a **grant-all** `base` becomes `scoped().grant(Desk = desk_value)` — exactly
///   the explicit desk scope, never wider — with any deny barriers carried (deny
///   wins, the §4 Chinese-wall semantics);
/// * a **scoped** `base` has each of its grants conjoined with `Desk = desk_value`
///   (`Rule::and`), so an asserted scope is *intersected* with the desk, never
///   widened beyond it; denies carried unchanged.
///
/// This is the post-boundary algebraic narrowing the §3 hardening requires: it is
/// invoked only for a desk-bound non-admin caller — admin / no-session
/// (`DeskScope::All`) callers skip it entirely, so their result is byte-identical to
/// before. `desk_value` is the caller's canonical numeric desk id
/// (`PositionStore::intern(slug)`); `0` is the house/unowned desk (`DeskId(0)`), the
/// deskless trader's narrowed view.
#[must_use]
pub fn narrow_to_desk(base: Principal, desk_value: u64) -> Principal {
    narrow_to_desks(base, &[desk_value])
}

/// **Many-to-many desk narrowing** — the union generalization of
/// [`narrow_to_desk`] for a caller who belongs to a **set** of desks. The narrowed
/// view is the *union* of the per-desk narrowings (a fact is visible iff it belongs
/// to ANY of the caller's desks), still bounded below by the asserted scope so it
/// can never widen past what the body principal grants:
///
/// * a **grant-all** `base` becomes `scoped()` with one `grant(Desk = d)` per desk
///   `d` in `desk_values` — the grants are disjunctive, so their union is exactly
///   "any of the caller's desks", never wider; deny barriers carried (deny wins);
/// * a **scoped** `base` conjoins **each** asserted grant with **each** desk
///   (`Rule::and`), emitting the cartesian set of `grant(g ∧ Desk = d)` — the
///   asserted scope intersected with the desk union; denies carried unchanged.
///
/// An empty `desk_values` yields a scoped principal with no grants (sees nothing) —
/// the security-conservative direction; in practice the caller's set is non-empty
/// (a deskless trader narrows to the house desk `0` via [`narrow_to_desk`]).
#[must_use]
pub fn narrow_to_desks(base: Principal, desk_values: &[u64]) -> Principal {
    let dim = DimensionId::Desk;
    let mut p = Principal::scoped();
    if base.is_grant_all() {
        for &desk_value in desk_values {
            p = p.grant(Rule::on(dim, desk_value));
        }
    } else {
        for g in base.grants() {
            for &desk_value in desk_values {
                p = p.grant(g.clone().and(dim, desk_value));
            }
        }
    }
    for d in base.denies() {
        p = p.deny(d.clone());
    }
    p
}

/// Map a domain [`Rule`] back onto a wire [`EntitlementRule`](celnet_proto::EntitlementRule)
/// — the inverse of [`rule_of`]. Every domain [`Scope`](celnet_entitlements::Scope)
/// carries its [`DimensionId`] and `u64` value verbatim, so the round-trip is exact.
fn rule_to_wire(rule: &Rule) -> celnet_proto::EntitlementRule {
    celnet_proto::EntitlementRule {
        scopes: rule
            .scopes()
            .iter()
            .map(|s| RiskScope {
                dimension: dimension_to_wire(s.dimension),
                value: s.value,
            })
            .collect(),
    }
}

/// Map a domain [`Principal`] back onto a wire [`EntitlementPrincipal`] — the
/// inverse of [`principal_of`]. Used by the distributed risk federation to forward
/// the **desk-narrowed** principal (item B §3) to the backends, so each backend
/// re-prunes by the same narrowed rule-set the aggregating edge resolved. The
/// round-trip is exact: grant-all/scoped flag, every grant, every deny.
#[must_use]
pub fn principal_to_wire(principal: &Principal) -> EntitlementPrincipal {
    EntitlementPrincipal {
        grant_all: principal.is_grant_all(),
        grants: principal.grants().iter().map(rule_to_wire).collect(),
        denies: principal.denies().iter().map(rule_to_wire).collect(),
    }
}

/// Map a wire [`EntitlementRule`](celnet_proto::EntitlementRule) (a conjunction of
/// scopes) onto a domain [`Rule`]. An empty rule covers everything (the firm root).
fn rule_of(rule: &celnet_proto::EntitlementRule) -> Result<Rule, Status> {
    let mut out = Rule::firm();
    for scope in &rule.scopes {
        match dimension_of(scope.dimension)? {
            None => { /* a FIRM-pinned scope inside a rule covers everything; no-op */ }
            Some(dim) => out = out.and(dim, scope.value),
        }
    }
    Ok(out)
}

/// A [`SpotResolver`] over a wire [`ReportingNumeraire`]: the numeraire currency
/// plus a `ccy → rate` table. The numeraire's own rate is implicitly `1.0`. Built
/// once per request and handed to the cube's numeraire collapse.
pub struct WireResolver {
    numeraire: Ccy,
    rates: HashMap<Ccy, f64>,
}

impl WireResolver {
    /// Build a resolver from a wire reporting-numeraire table.
    ///
    /// # Errors
    /// `invalid_argument` if the numeraire code or any rate currency is not a valid
    /// 3-letter code.
    pub fn new(numeraire: &ReportingNumeraire) -> Result<Self, Status> {
        let n = Ccy::parse(&numeraire.numeraire).ok_or_else(|| {
            Status::invalid_argument(format!("invalid numeraire `{}`", numeraire.numeraire))
        })?;
        let mut rates = HashMap::new();
        for r in &numeraire.rates {
            let c = Ccy::parse(&r.ccy)
                .ok_or_else(|| Status::invalid_argument(format!("invalid rate ccy `{}`", r.ccy)))?;
            rates.insert(c, r.rate);
        }
        Ok(Self {
            numeraire: n,
            rates,
        })
    }

    /// The reporting numeraire code (for echoing on the response).
    #[must_use]
    pub fn numeraire_code(&self) -> &str {
        self.numeraire.as_str()
    }

    /// The reporting numeraire currency.
    #[must_use]
    pub fn numeraire_ccy(&self) -> Ccy {
        self.numeraire
    }
}

impl SpotResolver for WireResolver {
    fn numeraire(&self) -> Ccy {
        self.numeraire
    }

    fn rate_into_numeraire(&self, ccy: Ccy) -> Option<f64> {
        if ccy == self.numeraire {
            return Some(1.0);
        }
        self.rates.get(&ccy).copied()
    }
}

/// Map a numeraire-conversion error into a `failed_precondition` status: a missing
/// or invalid rate fails the request loudly (no silent leg drop), the §2.3 contract.
#[must_use]
pub fn numeraire_status(e: NumeraireError) -> Status {
    Status::failed_precondition(format!("reporting numeraire conversion failed: {e}"))
}

/// Map a wire [`VegaPillar`](WireVegaPillar) onto the cube's [`VegaPillar`].
#[must_use]
pub fn pillar_of(p: &WireVegaPillar) -> VegaPillar {
    VegaPillar::new(p.tenor_days, p.delta_bp)
}

/// Map a cube [`VegaPillar`] back to the wire form.
#[must_use]
pub fn pillar_to_wire(p: VegaPillar) -> WireVegaPillar {
    WireVegaPillar {
        tenor_days: p.tenor_days,
        delta_bp: p.delta_bp,
    }
}

/// Map a wire [`RiskPosition`] into a [`RiskFact`] (re-deriving the canonical leaf
/// via `celnet-risk-normalize::canonicalize`). Used when a client supplies positions
/// directly (the explicit-fact path); the live book path interns from attribution.
///
/// # Errors
/// `invalid_argument` if the org key / pair / inputs / enums are malformed.
pub fn position_to_fact(p: &RiskPosition) -> Result<RiskFact, Status> {
    let org = p
        .org
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("RiskPosition missing `org`"))?;
    let wire_underlying = org
        .underlying
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("OrgKey missing `underlying`"))?;
    let pair: CcyPair = celnet_proto::convert::validate_fx_underlying(wire_underlying)
        .map_err(|e: WireError| Status::invalid_argument(e.to_string()))?
        .as_fx()
        .ok_or_else(|| Status::invalid_argument("OrgKey underlying is not an FX pair"))?;
    let inputs: VanillaInputs = p
        .inputs
        .ok_or_else(|| Status::invalid_argument("RiskPosition missing `inputs`"))?
        .into();
    let option = OptionType::from(
        celnet_proto::OptionType::try_from(p.option_type)
            .map_err(|_| Status::invalid_argument("unknown OptionType"))?,
    );
    let quoted_delta = DeltaConvention::from(
        celnet_proto::DeltaConvention::try_from(p.quoted_delta)
            .map_err(|_| Status::invalid_argument("unknown DeltaConvention"))?,
    );
    let premium_style = PremiumStyle::from(
        celnet_proto::PremiumStyle::try_from(p.premium_style)
            .map_err(|_| Status::invalid_argument("unknown PremiumStyle"))?,
    );
    let position = PositionRisk::fx(
        pair,
        option,
        p.notional_base,
        inputs,
        quoted_delta,
        premium_style,
    );
    let leaf = canonicalize(&position)
        .map_err(|e| Status::invalid_argument(format!("position is not priceable: {e}")))?;
    let handle = u32::try_from(p.position_id).map_err(|_| {
        Status::invalid_argument(format!(
            "position_id {} exceeds the u32 cube handle space",
            p.position_id
        ))
    })?;
    Ok(RiskFact {
        position_id: PositionId(handle),
        key: FactKey {
            trader: TraderId(org.trader),
            book: BookId(org.book),
            desk: DeskId(org.desk),
            underlying: Underlying::Fx(pair),
            location: LocationId(org.location),
            entity: EntityId(org.entity),
        },
        measure: FactMeasure {
            leaf,
            position,
            // The wire `RiskPosition` carries a vanilla leg (the proto is unchanged
            // — guardrail #9); a federated exotic leg, when present, is staged via
            // the cube's exotic path, not this vanilla wire conversion.
            exotic: None,
        },
        surface_version: p.surface_version,
    })
}

/// Map a [`RiskFact`] back to a wire [`RiskPosition`] (for `ListPositions` /
/// `DrillRisk` leaf reporting), re-attaching the recorded attribution chain and the
/// wire (business) `u64` id (the cube handle is a `u32` internal key).
#[must_use]
pub fn fact_to_position(
    fact: &RiskFact,
    wire_id: u64,
    attribution: Option<AttributionRecord>,
    risk_book: String,
) -> RiskPosition {
    let p = &fact.measure.position;
    // The wire `RiskPosition` carries an FX vanilla leg (the proto is unchanged —
    // guardrail #9). Project the carry-tagged position back to its FX pair + the FX
    // two-rate vanilla inputs (byte-identical to the originating leg); a non-FX leg
    // has no FX wire form, so it falls back to the leaf's spot-derived inputs.
    let wire_pair: celnet_proto::CcyPair = fact
        .key
        .underlying
        .as_ccy_pair()
        .or_else(|| p.inputs.underlying.as_ccy_pair())
        .unwrap_or_else(|| CcyPair::new(Ccy::USD, Ccy::USD))
        .into();
    let vanilla = celnet_core::carry::fx_vanilla_inputs(&p.inputs).unwrap_or_else(|_| {
        VanillaInputs::new(
            p.inputs.spot,
            p.inputs.strike,
            p.inputs.vol,
            p.inputs.t,
            0.0,
            0.0,
        )
    });
    let quoted_delta = p.quoted_delta.unwrap_or(DeltaConvention::SpotUnadjusted);
    let premium_style = p.premium_style.unwrap_or(PremiumStyle::DomesticPips);
    RiskPosition {
        position_id: wire_id,
        org: Some(OrgKey {
            trader: fact.key.trader.0,
            book: fact.key.book.0,
            desk: fact.key.desk.0,
            underlying: Some(celnet_proto::Underlying::fx(wire_pair)),
            location: fact.key.location.0,
            entity: fact.key.entity.0,
        }),
        option_type: celnet_proto::OptionType::from(p.option) as i32,
        notional_base: p.notional_base,
        inputs: Some(vanilla.into()),
        quoted_delta: celnet_proto::DeltaConvention::from(quoted_delta) as i32,
        premium_style: celnet_proto::PremiumStyle::from(premium_style) as i32,
        surface_version: fact.surface_version,
        attribution,
        risk_book,
        // The canonical, notional-scaled sensitivities straight off the marked leaf — the
        // SAME per-position numbers the risk cube aggregates and `fx_transfer_view` hands
        // the transfer applier. Published so a client can show what a position carries
        // without re-deriving it; `dv01` is 0 for FX vanilla (the rates arm carries it).
        risk: Some(celnet_proto::RiskVectorDesc {
            dv01: 0.0,
            delta: fact.measure.leaf.greeks.delta_base,
            gamma: fact.measure.leaf.greeks.gamma,
            vega: fact.measure.leaf.greeks.vega,
            theta: fact.measure.leaf.greeks.theta,
        }),
    }
}

/// Map a domain [`LimitMetric`] onto its wire `(kind, vega_pillar, tenor_days)`
/// triple. The pillar/tenor payloads ride on dedicated fields, selected by `kind`
/// (mirroring the proto contract).
/// Map a domain [`LimitMetric`] onto its wire `LimitStatus` kind + pillar/tenor payload.
///
/// The wire `LimitStatus` surface reports the **options** limit tree, whose metrics are
/// the FX-Greeks family only (the proto `LimitMetricKind` enumerates exactly these).
///
/// # Errors
/// `internal` for a **fixed-income** metric ([`LimitMetric::Dv01`] / [`LimitMetric::Pvbp`]
/// / [`LimitMetric::RateTenorBucket`]): FI limits are enforced against the rates risk
/// aggregate ([`celnet_limits::exposure_of_rates`]) and surfaced as a `LimitBreached`
/// status at the `AggregateRatesRisk` edge — never through this converter — so there is
/// deliberately no `LimitMetricKind` for them. An FI metric reaching here is an internal
/// invariant violation, rejected loudly rather than silently coerced to a wrong wire kind.
pub fn limit_metric_to_wire(
    metric: LimitMetric,
) -> Result<(LimitMetricKind, Option<WireVegaPillar>, u32), Status> {
    use celnet_limits::ConcentrationMetric;
    Ok(match metric {
        LimitMetric::Delta => (LimitMetricKind::Delta, None, 0),
        LimitMetric::Gamma => (LimitMetricKind::Gamma, None, 0),
        LimitMetric::Vega => (LimitMetricKind::Vega, None, 0),
        LimitMetric::Vanna => (LimitMetricKind::Vanna, None, 0),
        LimitMetric::Volga => (LimitMetricKind::Volga, None, 0),
        LimitMetric::VegaBucket(p) => (LimitMetricKind::VegaBucket, Some(pillar_to_wire(p)), 0),
        LimitMetric::TenorVega { tenor_days } => (LimitMetricKind::TenorVega, None, tenor_days),
        LimitMetric::Concentration(ConcentrationMetric::Delta) => {
            (LimitMetricKind::ConcentrationDelta, None, 0)
        }
        LimitMetric::Concentration(ConcentrationMetric::Vega) => {
            (LimitMetricKind::ConcentrationVega, None, 0)
        }
        LimitMetric::Var => (LimitMetricKind::Var, None, 0),
        LimitMetric::ExpectedShortfall => (LimitMetricKind::ExpectedShortfall, None, 0),
        LimitMetric::StopLoss => (LimitMetricKind::StopLoss, None, 0),
        LimitMetric::Dv01 | LimitMetric::Pvbp | LimitMetric::RateTenorBucket { .. } => {
            return Err(Status::internal(format!(
                "fixed-income limit metric {metric:?} has no wire LimitStatus representation; \
                 FI limits are enforced against the rates risk aggregate, not surfaced via LimitStatus"
            )));
        }
    })
}

/// The wire enum value for a domain [`RagStatus`].
#[must_use]
pub fn rag_to_wire(status: RagStatus) -> i32 {
    let w = match status {
        RagStatus::Green => WireRag::Green,
        RagStatus::Amber => WireRag::Amber,
        RagStatus::Red => WireRag::Red,
        RagStatus::Breach => WireRag::Breach,
    };
    w as i32
}

/// The wire enum value for a domain [`Enforcement`].
#[must_use]
pub fn enforcement_to_wire(e: Enforcement) -> i32 {
    let w = match e {
        Enforcement::Soft => WireEnforcement::Soft,
        Enforcement::Hard => WireEnforcement::Hard,
    };
    w as i32
}

/// Assemble a wire [`AdditiveRisk`](WireAdditive) from a node's numeraire view and
/// its (numeraire-converted) vega ladder. The delta vector legs are reported in
/// their own currencies; gamma and the higher Greeks are the raw summed canonical
/// sensitivities (`NetGreeks`); premium and vega are in the reporting numeraire.
#[must_use]
pub fn additive_to_wire(
    numeraire: &celnet_risk_normalize::Numeraire,
    net: &celnet_risk_cube::NetGreeks,
    vega_ladder: Vec<VegaLadderBucket>,
) -> WireAdditive {
    let delta_vector = numeraire
        .delta_vector
        .legs()
        .map(|leg| CcyExposureLeg {
            ccy: leg.ccy.as_str().to_owned(),
            amount: leg.amount,
        })
        .collect();
    WireAdditive {
        delta_numeraire: numeraire.delta_numeraire,
        delta_vector,
        gamma: net.gamma,
        vega_numeraire: numeraire.vega_numeraire,
        theta: net.theta,
        vanna: net.vanna,
        volga: net.volga,
        charm: net.charm,
        speed: net.speed,
        zomma: net.zomma,
        color: net.color,
        premium_numeraire: numeraire.premium_numeraire,
        vega_ladder,
    }
}

/// Assemble a wire [`NonAdditiveRisk`](WireNonAdditive) from optional re-derived
/// measures. Each is presence-tracked: an unevaluated measure is absent (never a
/// spurious zero), the §2.5 contract.
#[must_use]
pub fn nonadditive_to_wire(
    var: Option<f64>,
    es: Option<f64>,
    var_alpha: Option<f64>,
    curvature_spot: Option<f64>,
) -> WireNonAdditive {
    WireNonAdditive {
        var,
        es,
        var_alpha,
        curvature_spot,
    }
}

/// The `position_count` of a node aggregate (its constituent leaf count).
#[must_use]
pub fn node_position_count(node: &NodeAggregate) -> u32 {
    u32::try_from(node.positions.len()).unwrap_or(u32::MAX)
}

/// Build a wire [`RiskNode`](WireRiskNode) shell (dimension + group + counts);
/// the additive / nonadditive measures are filled by the aggregator.
#[must_use]
pub fn risk_node_shell(dimension: i32, group: u64, position_count: u32) -> WireRiskNode {
    WireRiskNode {
        dimension,
        group,
        additive: None,
        nonadditive: None,
        position_count,
    }
}
