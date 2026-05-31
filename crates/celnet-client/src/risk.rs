//! Firm-scale hierarchical risk — the typed SDK face of the `RiskService` contract.
//!
//! An external client (an SDK user, the GUI, the Excel add-in) gets the **same**
//! risk surface through this module as the GUI Book view: it lists the open
//! positions, asks the server for a rolled-up aggregate over an org dimension,
//! reads limit RAG, and drills a node to its constituents — every one a single
//! round-trip whose heavy lifting (convention canonicalization, common-numeraire
//! conversion, additive roll-up, non-additive bump-and-revalue, entitlement
//! pruning, limit utilization/RAG) runs **server-side**. A client never loops
//! positions and sums (the API-first parity rule): it asks for the node tree.
//!
//! The vocabulary here is celnet-logical, never raw proto:
//!
//! * [`OrgDimension`] is the roll-up axis ([`OrgDimension::Firm`] / `Trader` /
//!   `Book` / `Desk` / `CcyPair` / `Location` / `Entity`), each an orthogonal axis
//!   the cube groups facts onto, not one nesting.
//! * [`OrgKey`] is a position's placement across those axes (interned `u32`
//!   handles + the [`CcyPair`]); [`Scope`] pins one `(dimension, value)` subtree.
//! * [`Entitlements`] is the read principal applied as a pre-aggregation pruning
//!   predicate — the default ([`Entitlements::grant_all`]) is the show-all-now
//!   posture (a request that omits a principal is treated as grant-all server-side).
//! * [`Numeraire`] is the reporting currency + the per-currency spot rates a node's
//!   per-ccy exposure legs collapse through — so the aggregate is in ONE currency,
//!   not "native premium units" (the historical GUI caveat this work resolved).
//! * [`RiskNode`] carries the [`AdditiveRisk`] (netted, numeraire-converted) and the
//!   presence-tracked [`NonAdditiveRisk`] (VaR / ES / FRTB curvature, re-derived per
//!   node — absent, never a spurious zero, when not evaluated this cycle).
//! * [`LimitStatus`] carries the per-limit [`LimitUtilization`] (cap / exposure /
//!   ratio / [`Rag`] / [`Enforcement`]) and the hard-breach escalation flag.
//!
//! The four operations are exposed as ergonomic [`crate::Client`] methods:
//! [`Client::list_positions`](crate::Client::list_positions),
//! [`Client::aggregate_risk`](crate::Client::aggregate_risk),
//! [`Client::drill_risk`](crate::Client::drill_risk), and
//! [`Client::limit_status`](crate::Client::limit_status). The aggregate / drill /
//! limit requests are assembled with the fluent [`AggregateQuery`], [`DrillQuery`],
//! and [`LimitQuery`] builders so the long parameter lists (numeraire, principal,
//! pillars, VaR shocks) stay readable and self-describing.

use celnet_proto::{
    AggregateRiskRequest, DrillRiskRequest, EntitlementPrincipal, EntitlementRule,
    LimitStatusRequest, ListPositionsRequest, NumeraireRate, ReportingNumeraire,
};
use celnet_types::{CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

use crate::error::{ClientError, ClientResult};
use crate::vocab::Attribution;

// ===========================================================================
// dimensions / scope
// ===========================================================================

/// An organizational roll-up dimension — the axis a hierarchical aggregate groups
/// the firm's positions onto. These are ORTHOGONAL axes (a position is at once a
/// book fact, a desk fact, a ccy-pair fact), not one nesting: a group-by selects
/// one axis to roll the facts up along. The typed form of the wire
/// `RiskDimension`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum OrgDimension {
    /// The whole firm — the implicit apex every dimension rolls into (a single
    /// firm node). The default.
    #[default]
    Firm,
    /// The trader who owns the position.
    Trader,
    /// The book the position sits in.
    Book,
    /// The desk the book belongs to (an FRTB regulatory unit).
    Desk,
    /// The currency pair (the underlying axis, orthogonal to the org axes).
    CcyPair,
    /// The booking location (follow-the-sun / country).
    Location,
    /// The legal entity (the regulatory-capital unit).
    Entity,
}

impl OrgDimension {
    fn to_wire(self) -> celnet_proto::RiskDimension {
        match self {
            OrgDimension::Firm => celnet_proto::RiskDimension::Firm,
            OrgDimension::Trader => celnet_proto::RiskDimension::Trader,
            OrgDimension::Book => celnet_proto::RiskDimension::Book,
            OrgDimension::Desk => celnet_proto::RiskDimension::Desk,
            OrgDimension::CcyPair => celnet_proto::RiskDimension::CcyPair,
            OrgDimension::Location => celnet_proto::RiskDimension::Location,
            OrgDimension::Entity => celnet_proto::RiskDimension::Entity,
        }
    }

    fn to_tag(self) -> i32 {
        self.to_wire() as i32
    }

    fn from_tag(tag: i32) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        let w = celnet_proto::RiskDimension::try_from(tag).map_err(|_| WireError::UnknownEnum {
            kind: "RiskDimension",
            tag,
        })?;
        Ok(match w {
            celnet_proto::RiskDimension::Firm => OrgDimension::Firm,
            celnet_proto::RiskDimension::Trader => OrgDimension::Trader,
            celnet_proto::RiskDimension::Book => OrgDimension::Book,
            celnet_proto::RiskDimension::Desk => OrgDimension::Desk,
            celnet_proto::RiskDimension::CcyPair => OrgDimension::CcyPair,
            celnet_proto::RiskDimension::Location => OrgDimension::Location,
            celnet_proto::RiskDimension::Entity => OrgDimension::Entity,
        })
    }
}

/// A scope pinning a listing / aggregate / drill to one dimension subtree (e.g.
/// `Desk 99`) — the same `(dimension, value)` key space the cube groups by and an
/// entitlement scope covers. The typed form of the wire `RiskScope`.
///
/// `value` is the interned group handle on the dimension (ignored for
/// [`OrgDimension::Firm`], which covers everything).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope {
    /// The dimension this scope pins.
    pub dimension: OrgDimension,
    /// The group handle on that dimension (ignored when `dimension` is `Firm`).
    pub value: u64,
}

impl Scope {
    /// The whole-firm scope (the apex — covers every position).
    #[must_use]
    pub fn firm() -> Self {
        Self {
            dimension: OrgDimension::Firm,
            value: 0,
        }
    }

    /// A scope pinning dimension `dimension` to the group handle `value`.
    #[must_use]
    pub fn at(dimension: OrgDimension, value: u64) -> Self {
        Self { dimension, value }
    }

    fn to_wire(self) -> celnet_proto::RiskScope {
        celnet_proto::RiskScope {
            dimension: self.dimension.to_tag(),
            value: self.value,
        }
    }

    fn from_wire(w: &celnet_proto::RiskScope) -> ClientResult<Self> {
        Ok(Self {
            dimension: OrgDimension::from_tag(w.dimension)?,
            value: w.value,
        })
    }
}

// ===========================================================================
// entitlements
// ===========================================================================

/// A read grant or deny rule — a conjunction of pinned dimension [`Scope`]s. A fact
/// is covered iff EVERY scope in the rule covers it; an empty rule covers everything
/// (the firm root). The typed form of the wire `EntitlementRule`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntitlementScope {
    /// The conjunctive scopes; empty ⇒ covers everything (firm root).
    pub scopes: Vec<Scope>,
}

impl EntitlementScope {
    /// A rule covering exactly the positions in `scope`.
    #[must_use]
    pub fn covering(scope: Scope) -> Self {
        Self {
            scopes: vec![scope],
        }
    }

    /// A rule covering the firm root (everything).
    #[must_use]
    pub fn everything() -> Self {
        Self { scopes: Vec::new() }
    }

    fn to_wire(&self) -> EntitlementRule {
        EntitlementRule {
            scopes: self.scopes.iter().map(|s| s.to_wire()).collect(),
        }
    }
}

/// Who is looking — the entitlement principal the server applies as a
/// pre-aggregation pruning predicate (so a node total can never leak a subtree the
/// principal cannot see). The decision per fact is
/// `(grant_all OR some grant covers) AND no deny covers`.
///
/// The default is **grant-all** ([`Entitlements::grant_all`]) — the show-all-now
/// posture matching the GUI's `principal: "grant-all"`. A request that passes no
/// principal at all is also treated as grant-all server-side. A scoped principal
/// ([`Entitlements::scoped`]) is deny-by-default: it sees only what a grant covers.
/// `denies` are information barriers that win over any grant (Chinese walls), and
/// may be layered onto a grant-all firm view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entitlements {
    grant_all: bool,
    grants: Vec<EntitlementScope>,
    denies: Vec<EntitlementScope>,
}

impl Entitlements {
    /// The grant-all principal — admits every fact (the show-all-now default).
    #[must_use]
    pub fn grant_all() -> Self {
        Self {
            grant_all: true,
            grants: Vec::new(),
            denies: Vec::new(),
        }
    }

    /// A deny-by-default principal seeing only the facts a `grants` rule covers.
    /// Start from here and layer grants/denies with the builders.
    #[must_use]
    pub fn scoped() -> Self {
        Self {
            grant_all: false,
            grants: Vec::new(),
            denies: Vec::new(),
        }
    }

    /// Add a read grant — a (scoped) principal sees a fact if ANY grant covers it.
    #[must_use]
    pub fn grant(mut self, rule: EntitlementScope) -> Self {
        self.grants.push(rule);
        self
    }

    /// Add an information-barrier deny — a fact covered by ANY deny is cut even if
    /// granted (deny wins). Layer onto a grant-all firm view for a Chinese wall.
    #[must_use]
    pub fn deny(mut self, rule: EntitlementScope) -> Self {
        self.denies.push(rule);
        self
    }

    fn to_wire(&self) -> EntitlementPrincipal {
        EntitlementPrincipal {
            grant_all: self.grant_all,
            grants: self.grants.iter().map(EntitlementScope::to_wire).collect(),
            denies: self.denies.iter().map(EntitlementScope::to_wire).collect(),
        }
    }
}

// ===========================================================================
// numeraire
// ===========================================================================

/// The reporting currency + the per-currency spot rates every node's per-ccy
/// exposure legs collapse through, so the aggregate is expressed in ONE currency
/// (not "native premium units"). The typed form of the wire `ReportingNumeraire`.
///
/// The numeraire's own rate is implicitly `1.0` and need not be listed; a rate
/// missing for a currency that appears in the aggregated book fails the request
/// loudly server-side (no silent leg drop).
#[derive(Debug, Clone, PartialEq)]
pub struct Numeraire {
    /// The reporting currency every node measure is expressed in (a 3-letter code).
    pub currency: String,
    /// Units-of-numeraire-per-unit-ccy at spot, one per currency in the book.
    pub rates: Vec<(String, f64)>,
}

impl Numeraire {
    /// A reporting numeraire `currency` with no extra rates yet (add with
    /// [`Numeraire::rate`]). The numeraire's own rate is implicitly `1.0`.
    #[must_use]
    pub fn new(currency: impl Into<String>) -> Self {
        Self {
            currency: currency.into(),
            rates: Vec::new(),
        }
    }

    /// Add a spot conversion rate: `rate` units of the reporting numeraire per 1
    /// unit of `ccy`.
    #[must_use]
    pub fn rate(mut self, ccy: impl Into<String>, rate: f64) -> Self {
        self.rates.push((ccy.into(), rate));
        self
    }

    fn to_wire(&self) -> ReportingNumeraire {
        ReportingNumeraire {
            numeraire: self.currency.clone(),
            rates: self
                .rates
                .iter()
                .map(|(ccy, rate)| NumeraireRate {
                    ccy: ccy.clone(),
                    rate: *rate,
                })
                .collect(),
        }
    }
}

// ===========================================================================
// vega pillar
// ===========================================================================

/// A `(tenor × delta)` vega pillar — the bucket key for the vega ladder, quantized
/// to integer pillars so buckets net exactly across leaves. The typed form of the
/// wire `VegaPillar` (delta as basis-points: `0.25Δ → 2500`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RiskPillar {
    /// The tenor pillar, in calendar days.
    pub tenor_days: u32,
    /// The signed delta pillar in basis-points of delta (`0.25Δ → 2500`).
    pub delta_bp: i32,
}

impl RiskPillar {
    /// A pillar at `tenor_days` calendar days and `delta_bp` basis-points of delta.
    #[must_use]
    pub fn new(tenor_days: u32, delta_bp: i32) -> Self {
        Self {
            tenor_days,
            delta_bp,
        }
    }

    fn to_wire(self) -> celnet_proto::VegaPillar {
        celnet_proto::VegaPillar {
            tenor_days: self.tenor_days,
            delta_bp: self.delta_bp,
        }
    }

    fn from_wire(w: &celnet_proto::VegaPillar) -> Self {
        Self {
            tenor_days: w.tenor_days,
            delta_bp: w.delta_bp,
        }
    }
}

// ===========================================================================
// org key / position (response leaves)
// ===========================================================================

/// A position's placement across the orthogonal org dimensions — the foreign-key
/// tuple into the cube hierarchies. The identifiers are the cube's interned `u32`
/// dimension handles; `desk`/`entity` of `0` mean "resolve from the parent pointer"
/// (`Book → Desk` / `Location → Entity`). The typed form of the wire `OrgKey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrgKey {
    /// Interned handle for the owning trader.
    pub trader: u32,
    /// Interned handle for the book.
    pub book: u32,
    /// Interned handle for the desk (`0` ⇒ resolve from the book's parent).
    pub desk: u32,
    /// The currency pair underlying the position.
    pub ccy_pair: CcyPair,
    /// Interned handle for the booking location.
    pub location: u32,
    /// Interned handle for the legal entity (`0` ⇒ resolve from the location's
    /// parent).
    pub entity: u32,
}

impl OrgKey {
    fn from_wire(w: &celnet_proto::OrgKey) -> ClientResult<Self> {
        let ccy_pair = w
            .ccy_pair
            .as_ref()
            .ok_or(ClientError::MissingField("OrgKey.ccy_pair"))
            .and_then(|p| CcyPair::try_from(p.clone()).map_err(ClientError::Wire))?;
        Ok(Self {
            trader: w.trader,
            book: w.book,
            desk: w.desk,
            ccy_pair,
            location: w.location,
            entity: w.entity,
        })
    }
}

/// One open position the cube aggregates: its identity, org placement, raw
/// economics, pricing inputs, quoted conventions, and attribution. The convention-
/// free canonical leaf the cube sums is re-derived server-side from `inputs`, so
/// this carries the raw position, never a convention-baked Greek. The typed form of
/// the wire `RiskPosition`.
#[derive(Debug, Clone)]
pub struct RiskPosition {
    /// The position identity (one current fact per id).
    pub position_id: u64,
    /// The org placement across the orthogonal dimensions.
    pub org: OrgKey,
    /// Call or put on the base currency.
    pub option_type: OptionType,
    /// Signed base-currency notional (positive = long the option).
    pub notional_base: f64,
    /// The pricing inputs the position was marked under.
    pub inputs: VanillaInputs,
    /// The delta convention the position was quoted under (provenance).
    pub quoted_delta: DeltaConvention,
    /// The premium style the position was quoted under (provenance).
    pub premium_style: PremiumStyle,
    /// The marked-surface version that produced this fact.
    pub surface_version: u64,
    /// The who's-trading attribution chain, if the line was attributed.
    pub attribution: Option<Attribution>,
}

impl RiskPosition {
    fn from_wire(w: &celnet_proto::RiskPosition) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        let org = w
            .org
            .as_ref()
            .ok_or(ClientError::MissingField("RiskPosition.org"))
            .and_then(OrgKey::from_wire)?;
        let inputs = w
            .inputs
            .map(VanillaInputs::from)
            .ok_or(ClientError::MissingField("RiskPosition.inputs"))?;
        let option_type = celnet_proto::OptionType::try_from(w.option_type)
            .map_err(|_| WireError::UnknownEnum {
                kind: "OptionType",
                tag: w.option_type,
            })?
            .into();
        let quoted_delta = celnet_proto::DeltaConvention::try_from(w.quoted_delta)
            .map_err(|_| WireError::UnknownEnum {
                kind: "DeltaConvention",
                tag: w.quoted_delta,
            })?
            .into();
        let premium_style = celnet_proto::PremiumStyle::try_from(w.premium_style)
            .map_err(|_| WireError::UnknownEnum {
                kind: "PremiumStyle",
                tag: w.premium_style,
            })?
            .into();
        let attribution = w
            .attribution
            .as_ref()
            .map(Attribution::from_wire)
            .transpose()?;
        Ok(Self {
            position_id: w.position_id,
            org,
            option_type,
            notional_base: w.notional_base,
            inputs,
            quoted_delta,
            premium_style,
            surface_version: w.surface_version,
            attribution,
        })
    }
}

// ===========================================================================
// risk node (additive + non-additive)
// ===========================================================================

/// One signed currency leg of a node's netted delta-exposure vector: the per-
/// currency netting result before the vector is collapsed to the numeraire. The
/// typed form of the wire `CcyExposureLeg`.
#[derive(Debug, Clone, PartialEq)]
pub struct CcyExposure {
    /// The currency this leg is denominated in (a 3-letter code).
    pub ccy: String,
    /// Signed amount in units of `ccy` (positive = long that currency).
    pub amount: f64,
}

/// One `(tenor × delta)` bucket of the additive vega ladder: the netted vega in
/// that pillar, in the reporting numeraire. The typed form of the wire
/// `VegaLadderBucket`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VegaLadderBucket {
    /// The pillar this bucket sums.
    pub pillar: RiskPillar,
    /// The netted vega in this pillar (reporting numeraire, per `1.0` absolute vol).
    pub vega: f64,
}

/// The additive measures of a node, netted and expressed in the reporting
/// numeraire. Additive ⇒ a parent node's measures equal the sum of its children's.
/// The typed form of the wire `AdditiveRisk`.
#[derive(Debug, Clone, PartialEq)]
pub struct AdditiveRisk {
    /// Net delta collapsed to the reporting numeraire.
    pub delta_numeraire: f64,
    /// The netted per-currency delta-exposure vector (legs in their own currencies;
    /// reconciles to `delta_numeraire` through the numeraire rates).
    pub delta_vector: Vec<CcyExposure>,
    /// Net gamma (raw second-order sensitivity, not numeraire-scaled).
    pub gamma: f64,
    /// Net vega in the reporting numeraire (per `1.0` absolute vol).
    pub vega_numeraire: f64,
    /// Net theta (per year).
    pub theta: f64,
    /// Net vanna (FX spot × vol cross-Greek).
    pub vanna: f64,
    /// Net volga (FX vol-convexity Greek).
    pub volga: f64,
    /// Net charm (delta decay per year).
    pub charm: f64,
    /// Net speed (third spot derivative).
    pub speed: f64,
    /// Net zomma (`∂gamma/∂σ`).
    pub zomma: f64,
    /// Net color (`∂gamma/∂T`).
    pub color: f64,
    /// Total premium (option PV) in the reporting numeraire.
    pub premium_numeraire: f64,
    /// Vega bucketed by `(tenor × delta)` pillar, in the reporting numeraire.
    pub vega_ladder: Vec<VegaLadderBucket>,
}

impl AdditiveRisk {
    fn from_wire(w: &celnet_proto::AdditiveRisk) -> Self {
        Self {
            delta_numeraire: w.delta_numeraire,
            delta_vector: w
                .delta_vector
                .iter()
                .map(|l| CcyExposure {
                    ccy: l.ccy.clone(),
                    amount: l.amount,
                })
                .collect(),
            gamma: w.gamma,
            vega_numeraire: w.vega_numeraire,
            theta: w.theta,
            vanna: w.vanna,
            volga: w.volga,
            charm: w.charm,
            speed: w.speed,
            zomma: w.zomma,
            color: w.color,
            premium_numeraire: w.premium_numeraire,
            vega_ladder: w
                .vega_ladder
                .iter()
                .filter_map(|b| {
                    b.pillar.map(|p| VegaLadderBucket {
                        pillar: RiskPillar::from_wire(&p),
                        vega: b.vega,
                    })
                })
                .collect(),
        }
    }
}

/// The non-additive measures of a node, RE-DERIVED per node by bump-and-revalue
/// (NOT summed from children — VaR/ES diversify, curvature charges short-gamma
/// only). Every field is presence-tracked: a measure not evaluated this cycle is
/// `None`, never a spurious zero. The typed form of the wire `NonAdditiveRisk`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NonAdditiveRisk {
    /// Value-at-Risk loss magnitude at `var_alpha` (reporting numeraire), if
    /// evaluated.
    pub var: Option<f64>,
    /// Expected-Shortfall tail-loss magnitude at `var_alpha`, if evaluated.
    pub es: Option<f64>,
    /// The confidence level VaR/ES were evaluated at (present iff `var` or `es` is).
    pub var_alpha: Option<f64>,
    /// FRTB-SbM spot curvature charge (`max(CVR+, CVR-, 0)`), if evaluated.
    pub curvature_spot: Option<f64>,
}

impl NonAdditiveRisk {
    fn from_wire(w: &celnet_proto::NonAdditiveRisk) -> Self {
        Self {
            var: w.var,
            es: w.es,
            var_alpha: w.var_alpha,
            curvature_spot: w.curvature_spot,
        }
    }
}

/// One node of the rolled-up risk tree: a group along a dimension, its netted
/// additive measures, its re-derived non-additive measures, and the count of
/// constituent positions. The typed form of the wire `RiskNode`.
#[derive(Debug, Clone)]
pub struct RiskNode {
    /// The dimension this node groups along.
    pub dimension: OrgDimension,
    /// The group handle along that dimension (`0` for the firm apex).
    pub group: u64,
    /// The netted additive measures, in the reporting numeraire.
    pub additive: AdditiveRisk,
    /// The re-derived non-additive measures (presence-tracked).
    pub nonadditive: NonAdditiveRisk,
    /// The number of constituent positions that rolled into this node.
    pub position_count: u32,
}

impl RiskNode {
    /// The `(dimension, group)` [`Scope`] that addresses this node — pass it to
    /// [`Client::drill_risk`](crate::Client::drill_risk) or
    /// [`Client::limit_status`](crate::Client::limit_status) to expand / inspect it.
    #[must_use]
    pub fn scope(&self) -> Scope {
        Scope {
            dimension: self.dimension,
            value: self.group,
        }
    }

    fn from_wire(w: &celnet_proto::RiskNode) -> ClientResult<Self> {
        let additive = w
            .additive
            .as_ref()
            .map(AdditiveRisk::from_wire)
            .ok_or(ClientError::MissingField("RiskNode.additive"))?;
        let nonadditive = w
            .nonadditive
            .as_ref()
            .map(NonAdditiveRisk::from_wire)
            .unwrap_or_default();
        Ok(Self {
            dimension: OrgDimension::from_tag(w.dimension)?,
            group: w.group,
            additive,
            nonadditive,
            position_count: w.position_count,
        })
    }
}

// ===========================================================================
// aggregate / drill responses
// ===========================================================================

/// The rolled-up risk node tree for an [`Client::aggregate_risk`](crate::Client::aggregate_risk)
/// request: the dimension it grouped along, the reporting numeraire, and one
/// [`RiskNode`] per group (a single firm-apex node for [`OrgDimension::Firm`]).
#[derive(Debug, Clone)]
pub struct RiskAggregate {
    /// The dimension the aggregate rolled up onto.
    pub dimension: OrgDimension,
    /// The reporting currency every node measure is expressed in.
    pub numeraire: String,
    /// The rolled-up nodes (one per group along `dimension`).
    pub nodes: Vec<RiskNode>,
    /// Echo of the request's correlation id, if one was supplied.
    pub correlation_id: Option<u64>,
}

/// The result of drilling one node ([`Client::drill_risk`](crate::Client::drill_risk)):
/// its child sub-nodes broken out at the finer dimension and/or its contributing
/// positions (the leaves a node total reconciles to). The typed form of the wire
/// `DrillRiskResponse`.
#[derive(Debug, Clone)]
pub struct RiskDrill {
    /// The node that was drilled.
    pub node: Scope,
    /// The child sub-nodes broken out by the request's child dimension (empty when
    /// children were not requested).
    pub children: Vec<RiskNode>,
    /// The node's contributing positions (empty when positions were not requested).
    pub positions: Vec<RiskPosition>,
    /// Echo of the request's correlation id, if one was supplied.
    pub correlation_id: Option<u64>,
}

// ===========================================================================
// limits
// ===========================================================================

/// Which exposure a limit constrains. The pillar / tenor payloads ride on the
/// dedicated fields of [`LimitUtilization`], selected by this kind. The typed form
/// of the wire `LimitMetricKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitMetric {
    /// Net base-currency delta amount (additive). The default.
    Delta,
    /// Net gamma (additive).
    Gamma,
    /// Net vega per `1.0` absolute vol (additive).
    Vega,
    /// Net vanna (additive).
    Vanna,
    /// Net volga (additive).
    Volga,
    /// Vega in one `(tenor × delta)` pillar (selected by the utilization's pillar).
    VegaBucket,
    /// Aggregate vega in one tenor across delta pillars (selected by the tenor).
    TenorVega,
    /// Gross (un-netted) base-currency delta concentration.
    ConcentrationDelta,
    /// Gross (un-netted) vega concentration.
    ConcentrationVega,
    /// Value-at-Risk loss magnitude (non-additive, re-derived per node).
    Var,
    /// Expected-Shortfall tail-loss magnitude (non-additive).
    ExpectedShortfall,
    /// Stop-loss realized/scenario loss magnitude.
    StopLoss,
}

impl LimitMetric {
    fn from_tag(tag: i32) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        let w =
            celnet_proto::LimitMetricKind::try_from(tag).map_err(|_| WireError::UnknownEnum {
                kind: "LimitMetricKind",
                tag,
            })?;
        Ok(match w {
            celnet_proto::LimitMetricKind::Delta => LimitMetric::Delta,
            celnet_proto::LimitMetricKind::Gamma => LimitMetric::Gamma,
            celnet_proto::LimitMetricKind::Vega => LimitMetric::Vega,
            celnet_proto::LimitMetricKind::Vanna => LimitMetric::Vanna,
            celnet_proto::LimitMetricKind::Volga => LimitMetric::Volga,
            celnet_proto::LimitMetricKind::VegaBucket => LimitMetric::VegaBucket,
            celnet_proto::LimitMetricKind::TenorVega => LimitMetric::TenorVega,
            celnet_proto::LimitMetricKind::ConcentrationDelta => LimitMetric::ConcentrationDelta,
            celnet_proto::LimitMetricKind::ConcentrationVega => LimitMetric::ConcentrationVega,
            celnet_proto::LimitMetricKind::Var => LimitMetric::Var,
            celnet_proto::LimitMetricKind::ExpectedShortfall => LimitMetric::ExpectedShortfall,
            celnet_proto::LimitMetricKind::StopLoss => LimitMetric::StopLoss,
        })
    }
}

/// Traffic-light status of a limit's utilization, ordered by severity
/// (`Breach` worst). The typed form of the wire `RagStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rag {
    /// Below the amber band — comfortable headroom. The default.
    Green,
    /// At/above amber, below red — early warning.
    Amber,
    /// At/above red, at/below the cap — pre-breach.
    Red,
    /// Over the cap — a breach.
    Breach,
}

impl Rag {
    fn from_tag(tag: i32) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        let w = celnet_proto::RagStatus::try_from(tag).map_err(|_| WireError::UnknownEnum {
            kind: "RagStatus",
            tag,
        })?;
        Ok(match w {
            celnet_proto::RagStatus::Green => Rag::Green,
            celnet_proto::RagStatus::Amber => Rag::Amber,
            celnet_proto::RagStatus::Red => Rag::Red,
            celnet_proto::RagStatus::Breach => Rag::Breach,
        })
    }
}

/// Whether a limit warns or blocks. The typed form of the wire `Enforcement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Enforcement {
    /// A breach warns but never blocks a trade. The default.
    Soft,
    /// A breach blocks a pre-trade and escalates post-trade.
    Hard,
}

impl Enforcement {
    fn from_tag(tag: i32) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        let w = celnet_proto::Enforcement::try_from(tag).map_err(|_| WireError::UnknownEnum {
            kind: "Enforcement",
            tag,
        })?;
        Ok(match w {
            celnet_proto::Enforcement::Soft => Enforcement::Soft,
            celnet_proto::Enforcement::Hard => Enforcement::Hard,
        })
    }
}

/// One limit's evaluated utilization at a scope node: the metric, its optional
/// pillar/tenor selector, the cap, the measured exposure, the ratio, its [`Rag`]
/// classification, the [`Enforcement`] mode, and the signed headroom. The typed
/// form of the wire `LimitUtilization`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitUtilization {
    /// What this limit constrains.
    pub metric: LimitMetric,
    /// The `(tenor × delta)` pillar for a [`LimitMetric::VegaBucket`] (else ignored).
    pub vega_pillar: RiskPillar,
    /// The tenor (calendar days) for a [`LimitMetric::TenorVega`] (else ignored).
    pub tenor_days: u32,
    /// The cap, a non-negative magnitude in the metric's native units.
    pub cap: f64,
    /// The signed exposure measured against the cap.
    pub exposure: f64,
    /// `|exposure| / cap`, in `[0, ∞)`.
    pub ratio: f64,
    /// The RAG classification of `ratio` against the limit's bands.
    pub status: Rag,
    /// Soft (warn-only) or hard (blocking).
    pub enforcement: Enforcement,
    /// Remaining headroom (positive = room left, negative = over the cap).
    pub headroom: f64,
}

impl LimitUtilization {
    fn from_wire(w: &celnet_proto::LimitUtilization) -> ClientResult<Self> {
        Ok(Self {
            metric: LimitMetric::from_tag(w.metric)?,
            vega_pillar: w
                .vega_pillar
                .as_ref()
                .map(RiskPillar::from_wire)
                .unwrap_or(RiskPillar {
                    tenor_days: 0,
                    delta_bp: 0,
                }),
            tenor_days: w.tenor_days,
            cap: w.cap,
            exposure: w.exposure,
            ratio: w.ratio,
            status: Rag::from_tag(w.status)?,
            enforcement: Enforcement::from_tag(w.enforcement)?,
            headroom: w.headroom,
        })
    }
}

/// The limit utilizations + worst RAG for a scope node
/// ([`Client::limit_status`](crate::Client::limit_status)): every limit configured
/// at the scope with its current RAG, the worst status across them, and the
/// hard-breach escalation flag. The typed form of the wire `LimitStatusResponse`.
#[derive(Debug, Clone)]
pub struct LimitStatus {
    /// The scope the limits were evaluated at.
    pub scope: Scope,
    /// Every limit configured at the scope, with its current utilization/RAG.
    pub limits: Vec<LimitUtilization>,
    /// The worst RAG status across the scope's limits (`Green` if none breaches).
    pub worst: Rag,
    /// True if any HARD limit at the scope is breached (the escalation trigger).
    pub hard_breach: bool,
    /// Echo of the request's correlation id, if one was supplied.
    pub correlation_id: Option<u64>,
}

// ===========================================================================
// query builders
// ===========================================================================

/// A fluent builder for a hierarchical-aggregate request. Defaults are the
/// show-all-now firm view: grant-all principal, no scope (the whole entitled book),
/// the server's default vega-pillar grid, and no VaR/ES/curvature (those are
/// opt-in, so a node's non-additive measures are absent rather than a spurious
/// zero). Pass to [`Client::aggregate_risk`](crate::Client::aggregate_risk).
#[derive(Debug, Clone)]
pub struct AggregateQuery {
    dimension: OrgDimension,
    numeraire: Numeraire,
    principal: Option<Entitlements>,
    scope: Option<Scope>,
    vega_pillars: Vec<RiskPillar>,
    var_spot_shocks: Vec<f64>,
    var_alpha: f64,
    curvature_risk_weight: f64,
    correlation_id: Option<u64>,
}

impl AggregateQuery {
    /// A new aggregate query rolling up onto `dimension`, reported in `numeraire`.
    #[must_use]
    pub fn new(dimension: OrgDimension, numeraire: Numeraire) -> Self {
        Self {
            dimension,
            numeraire,
            principal: None,
            scope: None,
            vega_pillars: Vec::new(),
            var_spot_shocks: Vec::new(),
            var_alpha: 0.0,
            curvature_risk_weight: 0.0,
            correlation_id: None,
        }
    }

    /// Apply an entitlement principal pruning the facts before the roll-up. Omit to
    /// use the grant-all (show-all-now) default.
    #[must_use]
    pub fn entitled(mut self, principal: Entitlements) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Narrow the aggregate to one dimension subtree before the group-by (composes
    /// with the principal).
    #[must_use]
    pub fn scoped(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Pin the `(tenor × delta)` vega-ladder grid (external data). Empty ⇒ the
    /// server derives the live pillar set from the positions' tenors.
    #[must_use]
    pub fn vega_pillars(mut self, pillars: impl IntoIterator<Item = RiskPillar>) -> Self {
        self.vega_pillars = pillars.into_iter().collect();
        self
    }

    /// Evaluate VaR/ES by bumping spot over `shocks` (relative, multiplicative) at
    /// confidence `alpha` (e.g. `0.99`). Without this the non-additive measures are
    /// not evaluated (absent in the response, never a spurious zero).
    #[must_use]
    pub fn value_at_risk(mut self, shocks: impl IntoIterator<Item = f64>, alpha: f64) -> Self {
        self.var_spot_shocks = shocks.into_iter().collect();
        self.var_alpha = alpha;
        self
    }

    /// Charge FRTB-SbM spot curvature at `risk_weight` (external data). `0` ⇒
    /// curvature is not evaluated.
    #[must_use]
    pub fn curvature(mut self, risk_weight: f64) -> Self {
        self.curvature_risk_weight = risk_weight;
        self
    }

    /// Attach a caller correlation id, echoed on the response.
    #[must_use]
    pub fn correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    fn to_wire(&self) -> AggregateRiskRequest {
        AggregateRiskRequest {
            dimension: self.dimension.to_tag(),
            numeraire: Some(self.numeraire.to_wire()),
            principal: self.principal.as_ref().map(Entitlements::to_wire),
            scope: self.scope.map(Scope::to_wire),
            vega_pillars: self.vega_pillars.iter().map(|p| p.to_wire()).collect(),
            var_spot_shocks: self.var_spot_shocks.clone(),
            var_alpha: self.var_alpha,
            curvature_risk_weight: self.curvature_risk_weight,
            correlation_id: self.correlation_id,
        }
    }
}

/// A fluent builder for a node-drill request (the Book → Risk drill). By default it
/// returns neither children nor positions — call [`DrillQuery::children`] and/or
/// [`DrillQuery::positions`] to opt in. Pass to
/// [`Client::drill_risk`](crate::Client::drill_risk).
#[derive(Debug, Clone)]
pub struct DrillQuery {
    node: Scope,
    child_dimension: OrgDimension,
    numeraire: Numeraire,
    principal: Option<Entitlements>,
    vega_pillars: Vec<RiskPillar>,
    include_children: bool,
    include_positions: bool,
    correlation_id: Option<u64>,
}

impl DrillQuery {
    /// A new drill of `node`, with child sub-nodes (when requested) broken out at
    /// `child_dimension`, reported in `numeraire`.
    #[must_use]
    pub fn new(node: Scope, child_dimension: OrgDimension, numeraire: Numeraire) -> Self {
        Self {
            node,
            child_dimension,
            numeraire,
            principal: None,
            vega_pillars: Vec::new(),
            include_children: false,
            include_positions: false,
            correlation_id: None,
        }
    }

    /// Apply an entitlement principal pruning the drill. Omit ⇒ grant-all default.
    #[must_use]
    pub fn entitled(mut self, principal: Entitlements) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Pin the `(tenor × delta)` vega-ladder grid for the child nodes.
    #[must_use]
    pub fn vega_pillars(mut self, pillars: impl IntoIterator<Item = RiskPillar>) -> Self {
        self.vega_pillars = pillars.into_iter().collect();
        self
    }

    /// Return the child sub-nodes broken out by the drill's child dimension.
    #[must_use]
    pub fn children(mut self) -> Self {
        self.include_children = true;
        self
    }

    /// Return the node's contributing positions (the drill-down leaves).
    #[must_use]
    pub fn positions(mut self) -> Self {
        self.include_positions = true;
        self
    }

    /// Attach a caller correlation id, echoed on the response.
    #[must_use]
    pub fn correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    fn to_wire(&self) -> DrillRiskRequest {
        DrillRiskRequest {
            node: Some(self.node.to_wire()),
            child_dimension: self.child_dimension.to_tag(),
            numeraire: Some(self.numeraire.to_wire()),
            principal: self.principal.as_ref().map(Entitlements::to_wire),
            vega_pillars: self.vega_pillars.iter().map(|p| p.to_wire()).collect(),
            include_children: self.include_children,
            include_positions: self.include_positions,
            correlation_id: self.correlation_id,
        }
    }
}

/// A fluent builder for a limit-status request: the limits configured at a scope
/// with their current utilization/RAG. Defaults are the grant-all principal, the
/// server's default pillar grid, and no VaR/ES shocks (so the non-additive limits
/// read `0` and cannot spuriously breach). Pass to
/// [`Client::limit_status`](crate::Client::limit_status).
#[derive(Debug, Clone)]
pub struct LimitQuery {
    scope: Scope,
    numeraire: Numeraire,
    principal: Option<Entitlements>,
    vega_pillars: Vec<RiskPillar>,
    var_spot_shocks: Vec<f64>,
    var_alpha: f64,
    correlation_id: Option<u64>,
}

impl LimitQuery {
    /// A new limit-status query for `scope`, with additive exposures in `numeraire`.
    #[must_use]
    pub fn new(scope: Scope, numeraire: Numeraire) -> Self {
        Self {
            scope,
            numeraire,
            principal: None,
            vega_pillars: Vec::new(),
            var_spot_shocks: Vec::new(),
            var_alpha: 0.0,
            correlation_id: None,
        }
    }

    /// Apply an entitlement principal pruning the facts before the scope aggregate.
    /// Omit ⇒ grant-all default.
    #[must_use]
    pub fn entitled(mut self, principal: Entitlements) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Pin the `(tenor × delta)` vega-ladder grid for bucketed/tenor-vega limits.
    #[must_use]
    pub fn vega_pillars(mut self, pillars: impl IntoIterator<Item = RiskPillar>) -> Self {
        self.vega_pillars = pillars.into_iter().collect();
        self
    }

    /// Evaluate the non-additive (VaR/ES/stop-loss) limit exposures by bumping spot
    /// over `shocks` at confidence `alpha`. Without this they read `0`.
    #[must_use]
    pub fn value_at_risk(mut self, shocks: impl IntoIterator<Item = f64>, alpha: f64) -> Self {
        self.var_spot_shocks = shocks.into_iter().collect();
        self.var_alpha = alpha;
        self
    }

    /// Attach a caller correlation id, echoed on the response.
    #[must_use]
    pub fn correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    fn to_wire(&self) -> LimitStatusRequest {
        LimitStatusRequest {
            scope: Some(self.scope.to_wire()),
            numeraire: Some(self.numeraire.to_wire()),
            principal: self.principal.as_ref().map(Entitlements::to_wire),
            vega_pillars: self.vega_pillars.iter().map(|p| p.to_wire()).collect(),
            var_spot_shocks: self.var_spot_shocks.clone(),
            var_alpha: self.var_alpha,
            correlation_id: self.correlation_id,
        }
    }
}

/// A fluent builder for a position-listing request. Defaults list the whole
/// entitled book under the grant-all (show-all-now) principal. Pass to
/// [`Client::list_positions`](crate::Client::list_positions).
#[derive(Debug, Clone, Default)]
pub struct PositionQuery {
    scope: Option<Scope>,
    principal: Option<Entitlements>,
    correlation_id: Option<u64>,
}

impl PositionQuery {
    /// A new position-listing query (the whole entitled book by default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin the listing to one dimension subtree. Omit ⇒ the whole entitled book.
    #[must_use]
    pub fn scoped(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Apply an entitlement principal. Omit ⇒ grant-all (show-all-now) default.
    #[must_use]
    pub fn entitled(mut self, principal: Entitlements) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Attach a caller correlation id, echoed on the response.
    #[must_use]
    pub fn correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    fn to_wire(&self) -> ListPositionsRequest {
        ListPositionsRequest {
            scope: self.scope.map(Scope::to_wire),
            principal: self.principal.as_ref().map(Entitlements::to_wire),
            correlation_id: self.correlation_id,
        }
    }
}

/// The result of [`Client::list_positions`](crate::Client::list_positions): the
/// entitled open book, each [`RiskPosition`] carrying its org placement and
/// attribution.
#[derive(Debug, Clone)]
pub struct PositionList {
    /// The open positions admitted for the principal under the scope.
    pub positions: Vec<RiskPosition>,
    /// Echo of the request's correlation id, if one was supplied.
    pub correlation_id: Option<u64>,
}

// ===========================================================================
// the decode entry points used by `Client` (in lib.rs)
// ===========================================================================

pub(crate) fn list_positions_request(q: &PositionQuery) -> ListPositionsRequest {
    q.to_wire()
}

pub(crate) fn aggregate_request(q: &AggregateQuery) -> AggregateRiskRequest {
    q.to_wire()
}

pub(crate) fn drill_request(q: &DrillQuery) -> DrillRiskRequest {
    q.to_wire()
}

pub(crate) fn limit_request(q: &LimitQuery) -> LimitStatusRequest {
    q.to_wire()
}

pub(crate) fn position_list_from_wire(
    w: celnet_proto::ListPositionsResponse,
) -> ClientResult<PositionList> {
    let positions = w
        .positions
        .iter()
        .map(RiskPosition::from_wire)
        .collect::<ClientResult<Vec<_>>>()?;
    Ok(PositionList {
        positions,
        correlation_id: w.correlation_id,
    })
}

pub(crate) fn aggregate_from_wire(
    w: celnet_proto::AggregateRiskResponse,
) -> ClientResult<RiskAggregate> {
    let nodes = w
        .nodes
        .iter()
        .map(RiskNode::from_wire)
        .collect::<ClientResult<Vec<_>>>()?;
    Ok(RiskAggregate {
        dimension: OrgDimension::from_tag(w.dimension)?,
        numeraire: w.numeraire,
        nodes,
        correlation_id: w.correlation_id,
    })
}

pub(crate) fn drill_from_wire(w: celnet_proto::DrillRiskResponse) -> ClientResult<RiskDrill> {
    let node = w
        .node
        .as_ref()
        .ok_or(ClientError::MissingField("DrillRiskResponse.node"))
        .and_then(Scope::from_wire)?;
    let children = w
        .children
        .iter()
        .map(RiskNode::from_wire)
        .collect::<ClientResult<Vec<_>>>()?;
    let positions = w
        .positions
        .iter()
        .map(RiskPosition::from_wire)
        .collect::<ClientResult<Vec<_>>>()?;
    Ok(RiskDrill {
        node,
        children,
        positions,
        correlation_id: w.correlation_id,
    })
}

pub(crate) fn limit_status_from_wire(
    w: celnet_proto::LimitStatusResponse,
) -> ClientResult<LimitStatus> {
    let scope = w
        .scope
        .as_ref()
        .ok_or(ClientError::MissingField("LimitStatusResponse.scope"))
        .and_then(Scope::from_wire)?;
    let limits = w
        .limits
        .iter()
        .map(LimitUtilization::from_wire)
        .collect::<ClientResult<Vec<_>>>()?;
    Ok(LimitStatus {
        scope,
        limits,
        worst: Rag::from_tag(w.worst)?,
        hard_breach: w.hard_breach,
        correlation_id: w.correlation_id,
    })
}
