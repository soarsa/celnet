//! Exposure extraction, utilization, and pre/post-trade limit checks
//! (`docs/RISK-HIERARCHY.md` §5.3).
//!
//! This is the layer that joins a [`LimitTree`](crate::tree::LimitTree) to a
//! `celnet_risk_cube` node: it reads the **node's already-aggregated exposure** for
//! a limit's metric and classifies it.
//!
//! - **Additive metrics** (greeks, bucketed/tenor vega, concentration) are read
//!   straight off the node's `NetGreeks` / `VegaLadder` — the cube already summed
//!   them, so a check is O(1) and runs inside the µs-class additive-Greek pre-trade
//!   budget (RH §3.5/§5.3).
//! - **Non-additive metrics** (VaR / ES / stop-loss) are **re-derived per node** by
//!   the cube's bump-and-revalue reducers (RH §2.5) — these run on the
//!   recompute-trigger cadence (RH §3.3/§5.3), not the hot additive path.
//!
//! # Pre-trade vs post-trade (RH §5.3)
//!
//! - **Pre-trade** ([`pre_trade_check`]): the *incremental* exposure of a proposed
//!   trade is added to the node's current exposure and checked against every limit
//!   on the path *before* execution. A **hard** breach **rejects**; a soft breach
//!   warns (MiFID II / SEC 15c3-5 pre-trade controls — RH §5.3).
//! - **Post-trade** ([`post_trade_check`]): the node's *current* exposure is
//!   classified continuously; a breach drives the [`EscalationStatus`] workflow.

use celnet_risk_cube::{NetGreeks, NodeAggregate, Scenario, VegaPillar};
use celnet_risk_fleet::RatesNodeAggregate;

use crate::limit::{
    ConcentrationMetric, Enforcement, LimitMetric, LimitSpec, RagStatus, Utilization,
};
use crate::tree::{LimitScope, LimitTree, ScopePath};

/// The non-additive exposure inputs for a node, supplied by the caller so the check
/// layer stays pure (no scenario generation of its own). A limit on a non-additive
/// metric (`Var`/`ExpectedShortfall`/`StopLoss`) needs the node's re-derived loss
/// number; the caller computes it from the node's positions via the cube reducers
/// (or a richer scenario engine) and hands it in.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NonAdditiveExposure {
    /// The node's Value-at-Risk loss magnitude (`celnet_risk_cube::node_var_es`),
    /// or `None` if not evaluated this cycle (a VaR limit then reads `0`).
    pub var: Option<f64>,
    /// The node's Expected-Shortfall loss magnitude.
    pub es: Option<f64>,
    /// The node's realized/scenario stop-loss magnitude (a loss is a positive
    /// number; a profit is `0` against a stop-loss).
    pub stop_loss: Option<f64>,
}

impl NonAdditiveExposure {
    /// Re-derive VaR and ES for a node from its constituent positions over a shock
    /// grid (bump-and-revalue, RH §2.5), leaving stop-loss unset. This is the
    /// convenience path; a caller with its own scenario engine may build the struct
    /// directly.
    #[must_use]
    pub fn from_scenarios<P: celnet_core::carry::CarryPricer>(
        pricer: &P,
        exotic_pricer: &dyn celnet_core::ExoticLegPricer,
        node: &NodeAggregate,
        scenarios: &[Scenario],
        alpha: f64,
    ) -> Self {
        let ve = celnet_risk_cube::Cube::node_var_es(pricer, exotic_pricer, node, scenarios, alpha);
        Self {
            var: Some(ve.var),
            es: Some(ve.es),
            stop_loss: None,
        }
    }

    /// Set the realized/scenario stop-loss magnitude (a positive loss number).
    #[must_use]
    pub fn with_stop_loss(mut self, loss: f64) -> Self {
        self.stop_loss = Some(loss.max(0.0));
        self
    }
}

/// The signed exposure of a node for a limit `metric`, in the metric's native
/// units. Additive metrics read the node's aggregate directly; non-additive metrics
/// read the supplied [`NonAdditiveExposure`] (treated as `0` when not evaluated, so
/// an un-recomputed VaR is never spuriously breaching).
#[must_use]
pub fn exposure_of(
    node: &NodeAggregate,
    metric: LimitMetric,
    nonadditive: &NonAdditiveExposure,
) -> f64 {
    match metric {
        LimitMetric::Delta => node.net_greeks.delta_base,
        LimitMetric::Gamma => node.net_greeks.gamma,
        LimitMetric::Vega => node.net_greeks.vega,
        LimitMetric::Vanna => node.net_greeks.vanna,
        LimitMetric::Volga => node.net_greeks.volga,
        LimitMetric::VegaBucket(pillar) => node.vega_ladder.vega_in(pillar),
        LimitMetric::TenorVega { tenor_days } => node
            .vega_ladder
            .pillars()
            .filter(|(p, _)| p.tenor_days == tenor_days)
            .map(|(_, v)| v)
            .sum(),
        LimitMetric::Concentration(c) => gross_concentration(node, c),
        LimitMetric::Var => nonadditive.var.unwrap_or(0.0),
        LimitMetric::ExpectedShortfall => nonadditive.es.unwrap_or(0.0),
        LimitMetric::StopLoss => nonadditive.stop_loss.unwrap_or(0.0),
        // The linear fixed-income metrics carry **no exposure on an FX-Greeks node**: a
        // `NodeAggregate` is an options/Greeks roll-up with no linear-rate DV01 line.
        // They are enforced against a `celnet_risk_fleet::RatesNodeAggregate` via
        // [`exposure_of_rates`]; here they read a true `0` (inert), so a rates limit that
        // is (mis)configured onto an FX limit tree can never spuriously breach an options
        // booking. Every pre-existing FX-Greeks metric is untouched — `exposure_of` stays
        // byte-identical on the options path.
        //
        // ⚠ This zero is only safe because a caller that HAS FI limits routes them to
        // [`exposure_of_rates`] instead of here. A booking path that ran the FX-Greeks
        // [`pre_trade_check`] alone over a tree carrying FI limits would silently render
        // every one of them inert. [`pre_trade_check_mixed`] is the entry point that makes
        // that impossible for a mixed (linear-rates) tree; use it, not `pre_trade_check`,
        // whenever the tree may carry both families.
        LimitMetric::Dv01 | LimitMetric::Pvbp | LimitMetric::RateTenorBucket { .. } => 0.0,
    }
}

/// The signed exposure of a **linear fixed-income** node for a limit `metric`, in the
/// metric's native units — the FI counterpart to [`exposure_of`], reading a
/// [`RatesNodeAggregate`] (the curve-priced per-currency rates roll-up) instead of an
/// options `NodeAggregate`.
///
/// The three FI metrics read the aggregate's already-summed measures directly, so a
/// check is O(ladder) with no re-derivation and stays in the additive pre-trade budget
/// (RH §3.5/§5.3), exactly as the additive Greek metrics do:
///
/// - [`LimitMetric::Dv01`] → the net parallel DV01 ([`RatesNodeAggregate::net_dv01`]);
/// - [`LimitMetric::Pvbp`] → the net analytic PV01 ([`RatesNodeAggregate::net_pv01`]);
/// - [`LimitMetric::RateTenorBucket`] → the net DV01 in that tenor bucket of the merged
///   key-rate ladder ([`RatesNodeAggregate::key_rate_ladder`]); a tenor with no bucket
///   reads `0` (no exposure, never a spurious breach).
///
/// All are **signed** in the platform's one P&L convention (a rate rise is a loss ⇒
/// negative for a long bond / receive-fixed swap; `celnet-rates-risk` ladder), and the
/// ladder is already merged to one bucket per tenor, so the per-tenor sum is that one
/// bucket. [`LimitSpec::utilization`](crate::limit::LimitSpec::utilization) compares the
/// **magnitude** against the cap, so the two-sided cap is sign-agnostic — identical
/// handling to the signed Greek metrics.
///
/// An **FX-Greeks metric** (delta/vega/…) carries no exposure on a linear-FI node — a
/// rates roll-up has no option Greeks — so it reads a true `0` here, the mirror of the
/// FI-metrics-on-a-Greeks-node case in [`exposure_of`].
#[must_use]
pub fn exposure_of_rates(agg: &RatesNodeAggregate, metric: LimitMetric) -> f64 {
    match metric {
        LimitMetric::Dv01 => agg.net_dv01,
        LimitMetric::Pvbp => agg.net_pv01,
        LimitMetric::RateTenorBucket { tenor_years } => agg
            .key_rate_ladder
            .iter()
            .filter(|b| b.tenor_years == tenor_years)
            .map(|b| b.dv01)
            .sum(),
        LimitMetric::Delta
        | LimitMetric::Gamma
        | LimitMetric::Vega
        | LimitMetric::Vanna
        | LimitMetric::Volga
        | LimitMetric::VegaBucket(_)
        | LimitMetric::TenorVega { .. }
        | LimitMetric::Concentration(_)
        | LimitMetric::Var
        | LimitMetric::ExpectedShortfall
        | LimitMetric::StopLoss => 0.0,
    }
}

/// The **gross** (sum-of-absolute) magnitude of an additive metric across a node's
/// constituent leaves — the concentration measure (RH §5.1). Unlike the netted
/// `NetGreeks`, this does **not** let offsetting long/short legs cancel, so it
/// charges a large two-sided book that nets to ~0 but concentrates risk in one
/// slice.
fn gross_concentration(node: &NodeAggregate, metric: ConcentrationMetric) -> f64 {
    match metric {
        ConcentrationMetric::Delta => node.leaves.iter().map(|l| l.greeks.delta_base.abs()).sum(),
        ConcentrationMetric::Vega => node.leaves.iter().map(|l| l.greeks.vega.abs()).sum(),
    }
}

/// One limit's evaluation at one scope: the limit, the scope it sits at, and the
/// computed utilization/RAG (`docs/RISK-HIERARCHY.md` §5.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitCheck {
    /// The scope (hierarchy node) this limit sits at.
    pub scope: LimitScope,
    /// The limit that was evaluated.
    pub limit: LimitSpec,
    /// The computed utilization + RAG status.
    pub utilization: Utilization,
}

impl LimitCheck {
    /// Whether this is a **hard** breach (a hard limit over its cap) — the
    /// condition that blocks a pre-trade and escalates post-trade.
    #[must_use]
    pub fn is_hard_breach(&self) -> bool {
        self.limit.enforcement == Enforcement::Hard && self.utilization.status.is_breach()
    }

    /// Whether this is a **soft** breach (a soft limit over its cap — warns only).
    #[must_use]
    pub fn is_soft_breach(&self) -> bool {
        self.limit.enforcement == Enforcement::Soft && self.utilization.status.is_breach()
    }
}

/// Evaluate **every** limit configured at a scope against a node's exposure
/// (`docs/RISK-HIERARCHY.md` §5.2). Pure: the node's exposure is read, never
/// mutated.
#[must_use]
pub fn check_scope(
    tree: &LimitTree,
    scope: LimitScope,
    node: &NodeAggregate,
    nonadditive: &NonAdditiveExposure,
) -> Vec<LimitCheck> {
    tree.at(scope)
        .iter()
        .map(|limit| {
            let exposure = exposure_of(node, limit.metric, nonadditive);
            LimitCheck {
                scope,
                limit: *limit,
                utilization: limit.classify(exposure),
            }
        })
        .collect()
}

/// The decision of a pre-trade check (`docs/RISK-HIERARCHY.md` §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreTradeDecision {
    /// No limit on the path is breached — the trade may proceed.
    Accept,
    /// At least one **hard** limit would be breached — the trade is **blocked**.
    Reject,
    /// No hard breach, but a **soft** limit warns (the trade proceeds with an
    /// early-warning).
    Warn,
}

/// The full result of a pre-trade check across a position's whole scope path.
#[derive(Debug, Clone, PartialEq)]
pub struct PreTradeResult {
    /// The accept / warn / reject decision.
    pub decision: PreTradeDecision,
    /// Every limit evaluated on the path, with its post-trade (incremental)
    /// utilization — the full evidence behind the decision (RH §5.3 audit).
    pub checks: Vec<LimitCheck>,
}

impl PreTradeResult {
    /// Whether the trade is allowed (accepted or warned — not rejected).
    #[must_use]
    pub fn allowed(&self) -> bool {
        self.decision != PreTradeDecision::Reject
    }

    /// The hard breaches that caused a rejection (empty unless `decision ==
    /// Reject`).
    pub fn hard_breaches(&self) -> impl Iterator<Item = &LimitCheck> + '_ {
        self.checks.iter().filter(|c| c.is_hard_breach())
    }
}

/// **Pre-trade check** (`docs/RISK-HIERARCHY.md` §5.3): the incremental greeks of a
/// proposed trade are added to **each node on the position's path** and checked
/// against every limit there *before* execution. A hard breach at **any** node
/// rejects; otherwise a soft breach warns and a clean path accepts.
///
/// `incremental` is the proposed trade's additive contribution (a single position's
/// canonical net greeks + vega pillar). `path_nodes` supplies the *current*
/// aggregate at each scope on `path` (the caller resolves these from the cube, one
/// `group_by`/`firm_aggregate` per dimension on the path); `nonadditive` per scope
/// is the optional re-derived loss for that node *post-trade*. A scope on the path
/// with no current node aggregate is treated as empty (a fresh book).
#[must_use]
pub fn pre_trade_check<F, N>(
    tree: &LimitTree,
    path: &ScopePath,
    incremental: &IncrementalTrade,
    mut node_at: F,
    mut nonadditive_at: N,
) -> PreTradeResult
where
    F: FnMut(LimitScope) -> NodeAggregate,
    N: FnMut(LimitScope) -> NonAdditiveExposure,
{
    let mut checks = Vec::new();
    let mut hard_breach = false;
    let mut soft_warn = false;

    for scope in path.scopes() {
        if !tree.has(scope) {
            continue;
        }
        // The node *after* the proposed trade: current aggregate + incremental.
        let mut projected = node_at(scope);
        incremental.apply_to(&mut projected);
        let nonadditive = nonadditive_at(scope);

        for limit in tree.at(scope) {
            let exposure = exposure_of(&projected, limit.metric, &nonadditive);
            let utilization = limit.classify(exposure);
            let check = LimitCheck {
                scope,
                limit: *limit,
                utilization,
            };
            if check.is_hard_breach() {
                hard_breach = true;
            } else if utilization.status.is_breach() {
                soft_warn = true;
            }
            checks.push(check);
        }
    }

    let decision = if hard_breach {
        PreTradeDecision::Reject
    } else if soft_warn {
        PreTradeDecision::Warn
    } else {
        PreTradeDecision::Accept
    };
    PreTradeResult { decision, checks }
}

/// **Mixed-family pre-trade check** — the gate a *linear fixed-income* booking path
/// runs (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §6.1 G2).
///
/// A rates limit tree legitimately carries limits from **both** metric families: a
/// coarse [`LimitMetric::Delta`] proxy the booking sink charges, *and* precise
/// [`LimitMetric::Dv01`] / [`LimitMetric::Pvbp`] / [`LimitMetric::RateTenorBucket`] caps.
/// [`pre_trade_check`] alone cannot enforce the latter: [`exposure_of`] reads a true `0`
/// for every FI metric on an FX-Greeks [`NodeAggregate`], so **an FI limit configured on
/// a rates book was silently inert — it could never breach**. This entry point closes
/// that hole by routing each limit to the node its own family is measured on:
///
/// | Family | Node | Evaluator |
/// | --- | --- | --- |
/// | FX Greeks (delta/gamma/vega/…, concentration, VaR/ES/stop-loss) | projected [`NodeAggregate`] from `node_at` + `incremental` | [`exposure_of`] |
/// | Linear FI ([`LimitMetric::is_fixed_income`]) | [`RatesNodeAggregate`] from `rates_at` | [`exposure_of_rates`] |
///
/// Every limit is charged **exactly once**, by exactly one family, so the two can never
/// double-charge a node — the structural guarantee `is_fixed_income()` was introduced for.
///
/// `rates_at(scope)` must return the scope's **already-projected** aggregate (current
/// booked risk *plus* the proposed trade). Projecting on the caller's side rather than
/// applying an increment here is deliberate: [`RatesNodeAggregate`] is built by a
/// fixed-order additive fold whose exactness is its contract, so the projection is done
/// by folding the proposed position in as one more fact — never by mutating a finalized
/// aggregate.
///
/// Both closures are invoked **at most once per scope, and only when that scope actually
/// carries a limit of that family** — a tree with no FI limits never builds a rates
/// aggregate, so the FI path costs nothing until it is configured.
#[must_use]
pub fn pre_trade_check_mixed<F, N, R>(
    tree: &LimitTree,
    path: &ScopePath,
    incremental: &IncrementalTrade,
    mut node_at: F,
    mut nonadditive_at: N,
    mut rates_at: R,
) -> PreTradeResult
where
    F: FnMut(LimitScope) -> NodeAggregate,
    N: FnMut(LimitScope) -> NonAdditiveExposure,
    R: FnMut(LimitScope) -> RatesNodeAggregate,
{
    let mut checks = Vec::new();
    let mut hard_breach = false;
    let mut soft_warn = false;

    for scope in path.scopes() {
        if !tree.has(scope) {
            continue;
        }
        let limits = tree.at(scope);
        // Lazily materialize each family's node: a scope carrying only Greek limits never
        // builds a rates aggregate, and vice versa.
        let mut greeks: Option<(NodeAggregate, NonAdditiveExposure)> = None;
        let mut rates: Option<RatesNodeAggregate> = None;

        for limit in limits {
            let exposure = if limit.metric.is_fixed_income() {
                let agg = rates.get_or_insert_with(|| rates_at(scope));
                exposure_of_rates(agg, limit.metric)
            } else {
                let (node, nonadditive) = greeks.get_or_insert_with(|| {
                    let mut projected = node_at(scope);
                    incremental.apply_to(&mut projected);
                    (projected, nonadditive_at(scope))
                });
                exposure_of(node, limit.metric, nonadditive)
            };
            let utilization = limit.classify(exposure);
            let check = LimitCheck {
                scope,
                limit: *limit,
                utilization,
            };
            if check.is_hard_breach() {
                hard_breach = true;
            } else if utilization.status.is_breach() {
                soft_warn = true;
            }
            checks.push(check);
        }
    }

    let decision = if hard_breach {
        PreTradeDecision::Reject
    } else if soft_warn {
        PreTradeDecision::Warn
    } else {
        PreTradeDecision::Accept
    };
    PreTradeResult { decision, checks }
}

/// The additive contribution of a single proposed trade to a node — what a
/// pre-trade check adds to each node on the path before classifying
/// (`docs/RISK-HIERARCHY.md` §5.3).
///
/// Built from the proposed position's canonical leaf so the projection is exact and
/// convention-free (the same additive quantities the cube would sum once the trade
/// books). Carries its vega pillar so bucketed/tenor-vega limits see the increment
/// in the right bucket.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IncrementalTrade {
    /// The trade's canonical net greeks contribution.
    pub greeks: NetGreeks,
    /// The `(tenor × delta)` pillar the trade's vega lands in.
    pub vega_pillar: VegaPillar,
    /// The trade's vega (added into `vega_pillar`); usually `greeks.vega`, carried
    /// explicitly so a multi-leg structure can split vega across pillars.
    pub vega: f64,
}

impl IncrementalTrade {
    /// Build the incremental contribution from a canonical leaf and the pillar its
    /// vega buckets into.
    #[must_use]
    pub fn from_leaf(leaf: &celnet_risk_normalize::CanonicalLeaf, vega_pillar: VegaPillar) -> Self {
        let mut greeks = NetGreeks::zero();
        greeks.add_leaf(leaf);
        Self {
            greeks,
            vega_pillar,
            vega: leaf.greeks.vega,
        }
    }

    /// Add this trade's additive contribution into a node aggregate (the projected
    /// post-trade node). Concentration metrics also see the trade, because the
    /// caller adds the proposed leaf to `projected.leaves` when concentration is in
    /// scope; here we add the netted greeks and the vega pillar.
    fn apply_to(&self, node: &mut NodeAggregate) {
        node.net_greeks = node.net_greeks + self.greeks;
        node.vega_ladder.add(self.vega_pillar, self.vega);
    }
}

/// The escalation state of a node under post-trade monitoring
/// (`docs/RISK-HIERARCHY.md` §5.3). Mirrors the documented incumbent workflow
/// (suspend / hedge / block, four-eyes approval for a temporary excess).
///
/// Ordered by severity so the firm-wide status is the `max` over all nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EscalationStatus {
    /// All limits within band — nothing to escalate.
    Clear,
    /// A soft (warn-only) limit is breached — surfaced to the trader/risk desk, no
    /// blocking action.
    SoftBreach,
    /// A hard limit is breached — the escalation workflow fires (suspend / hedge /
    /// block, four-eyes for temporary excess).
    HardBreach,
}

/// The post-trade monitoring result for one scope: its escalation status, the worst
/// RAG across its limits, and every breached limit (`docs/RISK-HIERARCHY.md` §5.3).
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeMonitor {
    /// The scope monitored.
    pub scope: LimitScope,
    /// The worst RAG status across this scope's limits.
    pub worst: RagStatus,
    /// The escalation status driven by the worst breach at this scope.
    pub escalation: EscalationStatus,
    /// Every limit evaluated at this scope (the audit trail).
    pub checks: Vec<LimitCheck>,
}

/// **Post-trade check** (`docs/RISK-HIERARCHY.md` §5.3): classify a node's *current*
/// exposure against its limits and derive the escalation status. Unlike pre-trade,
/// nothing is projected — this is continuous monitoring of the booked state.
#[must_use]
pub fn post_trade_check(
    tree: &LimitTree,
    scope: LimitScope,
    node: &NodeAggregate,
    nonadditive: &NonAdditiveExposure,
) -> ScopeMonitor {
    let checks = check_scope(tree, scope, node, nonadditive);
    let mut worst = RagStatus::Green;
    let mut escalation = EscalationStatus::Clear;
    for c in &checks {
        worst = worst.max(c.utilization.status);
        if c.is_hard_breach() {
            escalation = escalation.max(EscalationStatus::HardBreach);
        } else if c.is_soft_breach() {
            escalation = escalation.max(EscalationStatus::SoftBreach);
        }
    }
    ScopeMonitor {
        scope,
        worst,
        escalation,
        checks,
    }
}

// ============================ fixed-income parity ============================
//
// The linear-FI counterparts of [`check_scope`] / [`post_trade_check`], reading a
// curve-priced `celnet_risk_fleet::RatesNodeAggregate` (net DV01 / PV01 + key-rate
// ladder) instead of the options `NodeAggregate`. They give linear fixed-income
// positions the same limit breach-detection + escalation the options cube already has,
// enforced against the rates risk aggregate (ADR-0016 A3).

/// Evaluate the **fixed-income** limits configured at a scope against a rates node's
/// aggregate — the FI counterpart to [`check_scope`]. Only
/// [`LimitMetric::is_fixed_income`] limits are evaluated: a Greek limit that also sits in
/// the rates tree (e.g. the coarse linear-delta proxy the rates booking sink charges at
/// pre-trade) is left to the options path, so the two families never double-charge the
/// same node. Pure: the aggregate is read, never mutated.
#[must_use]
pub fn check_scope_rates(
    tree: &LimitTree,
    scope: LimitScope,
    agg: &RatesNodeAggregate,
) -> Vec<LimitCheck> {
    tree.at(scope)
        .iter()
        .filter(|limit| limit.metric.is_fixed_income())
        .map(|limit| {
            let exposure = exposure_of_rates(agg, limit.metric);
            LimitCheck {
                scope,
                limit: *limit,
                utilization: limit.classify(exposure),
            }
        })
        .collect()
}

/// **Post-trade FI monitoring** — the fixed-income counterpart to [`post_trade_check`]:
/// classify a rates node's *current* aggregate against the FI limits at `scope` and
/// derive the escalation status. Continuous monitoring of the booked rates state,
/// enforced against the curve-priced rates risk aggregate ([`RatesNodeAggregate`]).
#[must_use]
pub fn post_trade_check_rates(
    tree: &LimitTree,
    scope: LimitScope,
    agg: &RatesNodeAggregate,
) -> ScopeMonitor {
    let checks = check_scope_rates(tree, scope, agg);
    let mut worst = RagStatus::Green;
    let mut escalation = EscalationStatus::Clear;
    for c in &checks {
        worst = worst.max(c.utilization.status);
        if c.is_hard_breach() {
            escalation = escalation.max(EscalationStatus::HardBreach);
        } else if c.is_soft_breach() {
            escalation = escalation.max(EscalationStatus::SoftBreach);
        }
    }
    ScopeMonitor {
        scope,
        worst,
        escalation,
        checks,
    }
}
