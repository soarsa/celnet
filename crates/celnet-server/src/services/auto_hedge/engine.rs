//! The `AutoHedgeEngine` — the off-core decision + provenance core of auto-hedging.
//!
//! On each `(book × instrument)` risk state (built off-core from the position / rates
//! stores + aggregation on a `risk_version` bump — `docs/AUTO-HEDGING-AND-
//! INTERNALISATION-REQUIREMENTS.md` §7.1), [`AutoHedgeEngine::evaluate`] resolves the
//! trader's [`HedgeGraph`] to an [`ExitAction`], sizes the shed against the resolved
//! [`WarehouseThreshold`](celnet_hedge_routing::WarehouseThreshold) (overflow-to-edge, clipped), decomposes it internal-first vs
//! external ([`netting_split`]), applies the safety guards (global + per-desk
//! kill-switch, advisory-only gating of *external* actions, max-clip / max-hedges-per-
//! interval / daily-external-notional caps), and stamps an immutable [`HedgeProvenance`]
//! plus a live-streamable [`HedgeIntent`].
//!
//! # What executes vs what is advisory (the honest boundary)
//!
//! The engine is the **complete, pure decision + provenance layer**. The internal-cross
//! and external-hedge *bookings* are applied through an injected
//! [`HedgeExecutor`](super::executor::HedgeExecutor): the [`AdvisoryExecutor`](super::executor::AdvisoryExecutor)
//! books **nothing** (the mandatory shadow-run — the default posture, §8.4), while a live
//! executor (the P2/P3 server seam) drives the same offsetting-booking sinks risk-transfer
//! uses. `Warehouse`/`Skew`/`Escalate` place no order by construction; `CrossInternal`
//! nets at the consolidated mid; `SubmitMarketOrder`/`RfqOut`/`Split` externalise and are
//! **advisory-gated** unless a desk explicitly disarms advisory with the kill-switch off.
//! Whatever executes, the emitted intent + provenance are **real** (never fabricated).
//!
//! # Off-core (guardrail 11)
//!
//! The engine touches no pricing thread: it reads an already-built [`HedgeContext`] and a
//! resolved threshold, records into a bounded in-memory provenance ring, and mutates only
//! its own rate counters under one short-held lock. No float work on any hot path.

use std::collections::{BTreeSet, VecDeque};
use std::sync::RwLock;

use celnet_hedge_routing::{
    ExitAction, HedgeContext, HedgeGraph, HedgeRouter, HedgeSize, LimitMetric, RagStatus,
    netting_split,
};
use celnet_proto::{ExitActionDesc, HedgeIntent, HedgeProvenance};

use super::wire::{band_label, exit_action_to_wire};
use crate::config::hedge_policy::{HedgeConfigDef, HedgeMetric, HedgeThresholdDef};

/// The default bounded depth of the fired-hedge provenance ring (the audit trail the
/// `ListHedgeProvenance` RPC reads). Old records fall off the back — a control-plane
/// history, not an unbounded log.
pub const PROVENANCE_RING_CAPACITY: usize = 4_096;

/// The outcome of one [`AutoHedgeEngine::evaluate`] call.
#[derive(Debug, Clone, PartialEq)]
pub struct HedgeOutcome {
    /// The live-streamable intent — the projection of what the engine resolved (always
    /// present, even for a green-band `Warehouse` hold, so a desk sees the shadow run).
    pub intent: HedgeIntent,
    /// The immutable audit record, present only when an action other than `Warehouse`
    /// fired (a hold changes nothing, so it is not stamped). When present it has also
    /// been appended to the engine's provenance ring.
    pub provenance: Option<HedgeProvenance>,
}

/// The mutable engine state behind one lock: the provenance ring + the rate-guard counters.
#[derive(Debug)]
struct Inner {
    ring: VecDeque<HedgeProvenance>,
    next_hedge: u64,
    /// Hedges fired live (non-advisory external) in the current rate-limit interval.
    hedges_in_interval: u32,
    /// Externalised notional booked live in the current daily window.
    daily_external: f64,
}

/// The off-core auto-hedge decision + provenance engine. Shared behind an `Arc` between
/// the control loop (which calls [`Self::evaluate`]) and the `ListHedgeProvenance` handler
/// (which calls [`Self::provenance`]).
#[derive(Debug)]
pub struct AutoHedgeEngine {
    inner: RwLock<Inner>,
    capacity: usize,
}

impl Default for AutoHedgeEngine {
    fn default() -> Self {
        Self::new(PROVENANCE_RING_CAPACITY)
    }
}

impl AutoHedgeEngine {
    /// A fresh engine with a provenance ring of `capacity` records.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: RwLock::new(Inner {
                ring: VecDeque::new(),
                next_hedge: 1,
                hedges_in_interval: 0,
                daily_external: 0.0,
            }),
            capacity: capacity.max(1),
        }
    }

    /// Reset the per-interval rate counter (the control loop calls this each interval
    /// boundary) — max-hedges-per-interval is a *rate*, not a lifetime cap.
    pub fn roll_rate_interval(&self) {
        let mut g = self.lock();
        g.hedges_in_interval = 0;
    }

    /// Reset the daily externalised-notional counter (called at the day boundary).
    pub fn roll_daily_window(&self) {
        let mut g = self.lock();
        g.daily_external = 0.0;
    }

    /// The recorded provenance, newest first, optionally filtered by `book` / `instrument`.
    #[must_use]
    pub fn provenance(&self, book: Option<&str>, instrument: Option<&str>) -> Vec<HedgeProvenance> {
        let g = self.lock();
        g.ring
            .iter()
            .rev()
            .filter(|p| book.is_none_or(|b| p.book == b))
            .filter(|p| instrument.is_none_or(|i| p.instrument == i))
            .cloned()
            .collect()
    }

    /// Resolve one risk state to an exit action and its intent + provenance.
    ///
    /// `now_nanos` is the trusted-source fire timestamp (injected so the pure decision is
    /// deterministic under test). `known_lps` is the live known-LP registry (the
    /// aggregation hub / FIX LP sessions), supplied by the caller off-core, used to
    /// resolve the effective hedging LP set for external actions. The full pipeline:
    /// guard (kill-switch / desk) → resolve policy → size → net → resolve LP panel →
    /// advisory/rate gate → stamp intent + provenance.
    pub fn evaluate(
        &self,
        graph: &HedgeGraph,
        threshold: &HedgeThresholdDef,
        config: &HedgeConfigDef,
        ctx: &HedgeContext,
        known_lps: &BTreeSet<String>,
        now_nanos: i64,
    ) -> HedgeOutcome {
        let wh = threshold.to_threshold();
        let net_risk = net_risk_for(threshold.metric, ctx);
        let band = wh.classify(net_risk);
        let utilization = wh.utilization(net_risk);
        let overflow = wh.overflow(net_risk);

        // Guard: a kill-switched or disabled desk simply warehouses — no action fires.
        if !config.desk_active(&ctx.desk) {
            return HedgeOutcome {
                intent: warehouse_intent(
                    ctx,
                    band,
                    net_risk,
                    wh.cap,
                    utilization,
                    overflow,
                    now_nanos,
                    "halted: kill-switch / desk disabled",
                ),
                provenance: None,
            };
        }

        // Resolve the policy. A validated graph never errors; a structurally broken one
        // (an unvalidated store) degrades safely to a warehouse hold, never a panic.
        let Ok(resolution) = HedgeRouter::resolve(graph, ctx) else {
            return HedgeOutcome {
                intent: warehouse_intent(
                    ctx,
                    band,
                    net_risk,
                    wh.cap,
                    utilization,
                    overflow,
                    now_nanos,
                    "policy graph did not resolve",
                ),
                provenance: None,
            };
        };
        let action = resolution.action.clone();
        let path = resolution.path.clone();

        // A green-band Warehouse hold changes nothing — emit the intent, stamp no record.
        if matches!(action, ExitAction::Warehouse) {
            return HedgeOutcome {
                intent: HedgeIntent {
                    book: ctx.book.clone(),
                    instrument: ctx.instrument_id.clone(),
                    action: Some(exit_action_to_wire(&action)),
                    band: band_label(band).to_owned(),
                    net_risk,
                    threshold: wh.cap,
                    utilization,
                    overflow,
                    size: 0.0,
                    internal_crossed: 0.0,
                    external_hedged: 0.0,
                    advisory: false,
                    fired_at: now_nanos,
                    policy_path: path,
                    reason: format!("{} · WAREHOUSE", band_label(band)),
                    lps: Vec::new(),
                },
                provenance: None,
            };
        }

        // Size the shed against the threshold for the size-bearing actions.
        let requested = requested_size(&action);
        let mut sized = requested.map_or(0.0, |s| wh.resolve_size(net_risk, s));
        if config.max_clip > 0.0 {
            sized = sized.min(config.max_clip);
        }

        // Decompose internal-first vs external per the action's semantics.
        let split = decompose(&action, sized, ctx.internal_offset_available);

        // Resolve the effective hedging LP set the external action TARGETS — the
        // standing scope panel (most-specific-wins), else the per-rule RFQ include list,
        // else the full known panel (§6.2). Internal / no-trade actions target no LP.
        let lps = if action.is_external() {
            resolve_effective_lps(config, ctx, &action, known_lps)
        } else {
            Vec::new()
        };

        // Advisory gate: external actions are advisory unless the desk has disarmed it
        // (advisory_only == false) with the kill-switch off; internal actions execute.
        let is_external = action.is_external();
        let mut advisory = is_external && config.advisory_only;
        let mut guard_reason: Option<&str> = None;

        // Rate / size guards apply only to a live (non-advisory) external fire.
        if is_external && !advisory {
            let mut g = self.lock();
            if config.max_hedges_per_interval > 0
                && g.hedges_in_interval >= config.max_hedges_per_interval
            {
                advisory = true;
                guard_reason = Some("rate cap: max hedges per interval reached");
            } else if config.daily_external_notional_cap > 0.0
                && g.daily_external + split.external > config.daily_external_notional_cap
            {
                advisory = true;
                guard_reason = Some("daily external-notional cap reached");
            } else {
                // A genuine live external fire — count it against the guards.
                g.hedges_in_interval = g.hedges_in_interval.saturating_add(1);
                g.daily_external += split.external;
            }
        }

        let reason = match guard_reason {
            Some(r) => format!("{} · {} · {r} → advisory", band_label(band), action.kind()),
            None => format!("{} · {}", band_label(band), action.kind()),
        };
        let action_wire = exit_action_to_wire(&action);

        let intent = HedgeIntent {
            book: ctx.book.clone(),
            instrument: ctx.instrument_id.clone(),
            action: Some(action_wire.clone()),
            band: band_label(band).to_owned(),
            net_risk,
            threshold: wh.cap,
            utilization,
            overflow,
            size: sized,
            internal_crossed: split.internal,
            external_hedged: split.external,
            advisory,
            fired_at: now_nanos,
            policy_path: path.clone(),
            reason,
            lps: lps.clone(),
        };

        let provenance = self.stamp_provenance(
            ctx,
            threshold.metric,
            wh.cap,
            net_risk,
            utilization,
            band,
            &path,
            action_wire,
            &split,
            advisory,
            lps,
            now_nanos,
        );

        HedgeOutcome {
            intent,
            provenance: Some(provenance),
        }
    }

    /// Build, record (into the bounded ring) and return one immutable [`HedgeProvenance`].
    #[allow(clippy::too_many_arguments)]
    fn stamp_provenance(
        &self,
        ctx: &HedgeContext,
        metric: HedgeMetric,
        cap: f64,
        net_risk: f64,
        utilization: f64,
        band: RagStatus,
        path: &[u32],
        action: ExitActionDesc,
        split: &Split,
        advisory: bool,
        lps: Vec<String>,
        now_nanos: i64,
    ) -> HedgeProvenance {
        let mut g = self.lock();
        let id = g.next_hedge;
        g.next_hedge = g.next_hedge.saturating_add(1);
        let prov = HedgeProvenance {
            hedge_id: format!("HDG-{id}"),
            book: ctx.book.clone(),
            instrument: ctx.instrument_id.clone(),
            fired_at: now_nanos,
            metric: metric.as_i32(),
            threshold: cap,
            net_risk,
            utilization,
            band: band_label(band).to_owned(),
            policy_path: path.to_vec(),
            action: Some(action),
            internal_crossed: split.internal,
            external_hedged: split.external,
            residual: split.residual,
            // No live fill is booked in this (advisory/decision) layer — the price fields
            // are honestly zero until a live executor books a real leg (the P2/P3 seam).
            hedge_price: 0.0,
            mid_at_fire: 0.0,
            slippage_bp: 0.0,
            lp_won: None,
            advisory,
            lps,
        };
        if g.ring.len() >= self.capacity {
            g.ring.pop_front();
        }
        g.ring.push_back(prov.clone());
        prov
    }

    fn lock(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().expect("auto-hedge engine lock poisoned")
    }
}

/// The internal / external / residual decomposition of a sized shed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Split {
    internal: f64,
    external: f64,
    residual: f64,
}

/// The signed net risk in the threshold's metric, read off the risk-state snapshot.
fn net_risk_for(metric: HedgeMetric, ctx: &HedgeContext) -> f64 {
    match metric {
        HedgeMetric::Dv01 => ctx.net_dv01,
        HedgeMetric::NetNotional | HedgeMetric::NetDelta => ctx.net_notional,
        HedgeMetric::NetVega => ctx.net_vega,
    }
}

/// The [`HedgeSize`] a size-bearing action requests (or `None` for a no-shed action).
fn requested_size(action: &ExitAction) -> Option<HedgeSize> {
    match action {
        ExitAction::CrossInternal { max_size, .. } => Some(*max_size),
        ExitAction::SubmitMarketOrder { size, .. } | ExitAction::RfqOut { size, .. } => Some(*size),
        // Split sheds the overflow to the edge; Warehouse/Skew/Escalate shed nothing.
        ExitAction::Split { .. } => Some(HedgeSize::Overflow),
        ExitAction::Warehouse | ExitAction::Skew { .. } | ExitAction::Escalate { .. } => None,
    }
}

/// Decompose a sized shed into internal-cross / external-hedge / warehoused-residual per
/// the action's semantics (§6.3 netting waterfall).
fn decompose(action: &ExitAction, sized: f64, internal_offset: f64) -> Split {
    match action {
        // Cross what we can internally; the unshed remainder stays warehoused.
        ExitAction::CrossInternal { .. } => {
            let s = netting_split(sized, internal_offset, true);
            Split {
                internal: s.internal_crossed,
                external: 0.0,
                residual: sized - s.internal_crossed,
            }
        }
        // Net internally first (or not), externalise the residual — nothing warehoused.
        ExitAction::Split { internal_first, .. } => {
            let s = netting_split(sized, internal_offset, *internal_first);
            Split {
                internal: s.internal_crossed,
                external: s.external_hedged,
                residual: 0.0,
            }
        }
        // Straight externalisation — all to the street.
        ExitAction::SubmitMarketOrder { .. } | ExitAction::RfqOut { .. } => Split {
            internal: 0.0,
            external: sized,
            residual: 0.0,
        },
        // Escalation hands the whole amount to a human — warehoused pending action.
        ExitAction::Escalate { .. } => Split {
            internal: 0.0,
            external: 0.0,
            residual: sized,
        },
        // No-trade actions shed nothing.
        ExitAction::Warehouse | ExitAction::Skew { .. } => Split {
            internal: 0.0,
            external: 0.0,
            residual: 0.0,
        },
    }
}

/// A green-band / halted `Warehouse` intent (no provenance).
#[allow(clippy::too_many_arguments)]
fn warehouse_intent(
    ctx: &HedgeContext,
    band: RagStatus,
    net_risk: f64,
    cap: f64,
    utilization: f64,
    overflow: f64,
    now_nanos: i64,
    reason: &str,
) -> HedgeIntent {
    HedgeIntent {
        book: ctx.book.clone(),
        instrument: ctx.instrument_id.clone(),
        action: Some(exit_action_to_wire(&ExitAction::Warehouse)),
        band: band_label(band).to_owned(),
        net_risk,
        threshold: cap,
        utilization,
        overflow,
        size: 0.0,
        internal_crossed: 0.0,
        external_hedged: 0.0,
        advisory: true,
        fired_at: now_nanos,
        policy_path: Vec::new(),
        reason: reason.to_owned(),
        lps: Vec::new(),
    }
}

/// Resolve the effective hedging LP set an external [`ExitAction`] targets (§6.2).
///
/// Precedence:
/// 1. the standing **scope LP panel** (most-specific-wins, instrument > book > desk) —
///    inherited by BOTH `SUBMIT_MARKET_ORDER` and `RFQ_OUT`, generalising the include
///    list with exclude semantics. A stale panel (an id the registry no longer knows)
///    degrades safely to the full known set rather than dropping the hedge;
/// 2. the per-rule `RFQ_OUT` **include list** (the shipped back-compat behaviour), when
///    no scope panel is configured;
/// 3. the **full known panel** (every known LP) — the default for `SUBMIT_MARKET_ORDER`
///    / `SPLIT` / an empty `RFQ_OUT` with no scope panel.
fn resolve_effective_lps(
    config: &HedgeConfigDef,
    ctx: &HedgeContext,
    action: &ExitAction,
    known_lps: &BTreeSet<String>,
) -> Vec<String> {
    if let Some(panel) = config.resolve_lp_panel(&ctx.desk, &ctx.book, &ctx.instrument_id) {
        return panel
            .effective_lps(known_lps)
            .unwrap_or_else(|_| known_lps.iter().cloned().collect());
    }
    if let ExitAction::RfqOut { lps, .. } = action
        && !lps.is_empty()
    {
        return lps.clone();
    }
    known_lps.iter().cloned().collect()
}

/// Map the pure [`WarehouseThreshold`](celnet_hedge_routing::WarehouseThreshold) metric back to the wire metric ordinal — only used
/// where a caller holds a bare [`WarehouseThreshold`](celnet_hedge_routing::WarehouseThreshold) rather than a [`HedgeThresholdDef`].
#[must_use]
pub fn limit_metric_to_wire(metric: LimitMetric) -> i32 {
    match metric {
        LimitMetric::Dv01 => HedgeMetric::Dv01.as_i32(),
        LimitMetric::Vega => HedgeMetric::NetVega.as_i32(),
        // Delta and everything else surface as net-delta on the wire.
        _ => HedgeMetric::NetDelta.as_i32(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_hedge_routing::{ExecStyle, HedgeNode, RouteOp, RouteValue};
    use celnet_proto::ExitActionKind;
    use std::collections::BTreeMap;

    /// A graph: `breached == false ? WAREHOUSE : <hedge>`.
    fn graph(hedge: ExitAction) -> HedgeGraph {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            HedgeNode::Condition {
                field: celnet_hedge_routing::HedgeField::Breached,
                op: RouteOp::Eq,
                value: RouteValue::Text("false".into()),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1,
            HedgeNode::Action {
                exit: ExitAction::Warehouse,
            },
        );
        nodes.insert(2, HedgeNode::Action { exit: hedge });
        HedgeGraph { entry: 0, nodes }
    }

    fn thr() -> HedgeThresholdDef {
        HedgeThresholdDef {
            scope_kind: crate::config::hedge_policy::HedgeScopeKind::Book,
            metric: HedgeMetric::Dv01,
            cap: 100_000.0,
            amber: 0.8,
            red: 0.9,
            target_fraction: 0.8, // edge at 80k
            min_clip: 0.0,
            max_clip: f64::INFINITY,
            ramped: false,
            ramp_k: 1.0,
        }
    }

    fn ctx(net_dv01: f64, breached: bool, offset: f64, desk: &str) -> HedgeContext {
        HedgeContext {
            instrument_id: "EURUSD".into(),
            book: "RATES-EUR".into(),
            desk: desk.into(),
            net_dv01,
            breached,
            internal_offset_available: offset,
            ..HedgeContext::default()
        }
    }

    /// The known-LP registry the engine resolves the effective panel against.
    fn known() -> BTreeSet<String> {
        ["LP-1", "LP-2", "LP-3", "LP-4"]
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }

    #[test]
    fn green_band_warehouses_with_no_provenance() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(50_000.0, false, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(
            out.intent.action.unwrap().kind,
            ExitActionKind::ExitActionWarehouse as i32
        );
        assert!(out.provenance.is_none(), "a hold stamps no provenance");
        assert!(e.provenance(None, None).is_empty());
    }

    #[test]
    fn external_action_is_advisory_by_default_and_records_provenance() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        // net 95k DV01, breached, threshold edge 80k → overflow 15k.
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            42,
        );
        let prov = out.provenance.expect("an action stamps provenance");
        assert!(out.intent.advisory, "external default is advisory-only");
        assert!(prov.advisory);
        assert_eq!(out.intent.external_hedged, 15_000.0);
        assert_eq!(prov.internal_crossed, 0.0);
        assert_eq!(e.provenance(None, None).len(), 1);
        assert_eq!(e.provenance(Some("RATES-EUR"), None).len(), 1);
        assert!(e.provenance(Some("OTHER"), None).is_empty());
    }

    #[test]
    fn cross_internal_nets_against_offset_and_is_not_advisory() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::CrossInternal {
            instrument: "EURUSD".into(),
            max_size: HedgeSize::Overflow,
        });
        // overflow 15k, offset 6k → 6k crossed, 9k residual (warehoused), 0 external.
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 6_000.0, "RATES"),
            &known(),
            7,
        );
        let prov = out.provenance.unwrap();
        assert!(
            !out.intent.advisory,
            "an internal cross is not advisory-gated"
        );
        assert_eq!(prov.internal_crossed, 6_000.0);
        assert_eq!(prov.external_hedged, 0.0);
        assert_eq!(prov.residual, 9_000.0);
    }

    #[test]
    fn kill_switch_halts_to_warehouse() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            kill_switch: true,
            ..HedgeConfigDef::default()
        };
        let out = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(
            out.intent.action.unwrap().kind,
            ExitActionKind::ExitActionWarehouse as i32
        );
        assert!(out.provenance.is_none());
    }

    #[test]
    fn disarmed_advisory_fires_live_until_rate_cap() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            advisory_only: false, // disarmed → live
            max_hedges_per_interval: 1,
            ..HedgeConfigDef::default()
        };
        let first = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert!(!first.intent.advisory, "first live external fire");
        // Second fire trips the per-interval rate cap → downgraded to advisory.
        let second = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            2,
        );
        assert!(second.intent.advisory, "rate cap forces advisory");
        assert!(second.intent.reason.contains("rate cap"));
        // Rolling the interval re-enables a live fire.
        e.roll_rate_interval();
        let third = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            3,
        );
        assert!(
            !third.intent.advisory,
            "interval roll re-arms the live fire"
        );
    }

    #[test]
    fn daily_external_cap_forces_advisory() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Full,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            advisory_only: false,
            daily_external_notional_cap: 10_000.0,
            ..HedgeConfigDef::default()
        };
        // Full flatten of 95k > 10k cap → advisory.
        let out = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert!(out.intent.advisory);
        assert!(out.intent.reason.contains("daily external"));
    }

    #[test]
    fn max_clip_config_bounds_the_shed() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Full,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            max_clip: 5_000.0,
            ..HedgeConfigDef::default()
        };
        let out = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(out.intent.size, 5_000.0, "config max_clip caps the shed");
    }

    #[test]
    fn provenance_ring_is_bounded() {
        let e = AutoHedgeEngine::new(2);
        let g = graph(ExitAction::CrossInternal {
            instrument: "EURUSD".into(),
            max_size: HedgeSize::Overflow,
        });
        for t in 0..5 {
            e.evaluate(
                &g,
                &thr(),
                &HedgeConfigDef::default(),
                &ctx(95_000.0, true, 0.0, "RATES"),
                &known(),
                t,
            );
        }
        assert_eq!(e.provenance(None, None).len(), 2, "ring caps at capacity");
    }

    // --- hedging LP panel: include / exclude resolution on external actions -----

    use crate::config::hedge_policy::{HedgeScopeKind, ScopedLpPanel};
    use celnet_hedge_routing::HedgeLpPanel;

    fn panel_cfg(
        scope_kind: HedgeScopeKind,
        id: &str,
        include: &[&str],
        exclude: &[&str],
    ) -> HedgeConfigDef {
        HedgeConfigDef {
            lp_panels: vec![ScopedLpPanel {
                scope_kind,
                scope_id: id.into(),
                panel: HedgeLpPanel {
                    include: include.iter().map(|s| (*s).to_string()).collect(),
                    exclude: exclude.iter().map(|s| (*s).to_string()).collect(),
                },
            }],
            ..HedgeConfigDef::default()
        }
    }

    #[test]
    fn submit_market_order_defaults_to_full_known_panel() {
        // No scope panel, no per-rule list → SUBMIT_MARKET_ORDER targets every known LP.
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(out.intent.lps, vec!["LP-1", "LP-2", "LP-3", "LP-4"]);
        assert_eq!(
            out.provenance.unwrap().lps,
            vec!["LP-1", "LP-2", "LP-3", "LP-4"]
        );
    }

    #[test]
    fn per_rule_rfq_include_list_is_honoured_without_a_scope_panel() {
        // The shipped back-compat behaviour: RFQ_OUT's own include list wins when no
        // scope panel is configured.
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::RfqOut {
            lps: vec!["LP-2".into(), "LP-3".into()],
            size: HedgeSize::Overflow,
        });
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(out.intent.lps, vec!["LP-2", "LP-3"]);
    }

    #[test]
    fn scope_panel_exclude_removes_an_lp_for_both_external_actions() {
        // A book-scoped exclude panel: hedge on all known LPs EXCEPT LP-2. It applies to
        // SUBMIT_MARKET_ORDER (no per-rule list) AND overrides an RFQ per-rule list.
        let cfg = panel_cfg(HedgeScopeKind::Book, "RATES-EUR", &[], &["LP-2"]);
        let e = AutoHedgeEngine::default();

        let smo = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let out = e.evaluate(
            &smo,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(
            out.intent.lps,
            vec!["LP-1", "LP-3", "LP-4"],
            "exclude drops LP-2"
        );

        // The standing scope panel supersedes the per-rule include list on RFQ_OUT.
        let rfq = graph(ExitAction::RfqOut {
            lps: vec!["LP-1".into(), "LP-2".into()],
            size: HedgeSize::Overflow,
        });
        let out2 = e.evaluate(
            &rfq,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            2,
        );
        assert_eq!(
            out2.intent.lps,
            vec!["LP-1", "LP-3", "LP-4"],
            "panel wins over per-rule list"
        );
    }

    #[test]
    fn scope_panel_include_narrows_the_targeted_set() {
        // An instrument-scoped include panel: hedge only on LP-4.
        let cfg = panel_cfg(HedgeScopeKind::Instrument, "EURUSD", &["LP-4"], &[]);
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let out = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(out.intent.lps, vec!["LP-4"]);
    }

    #[test]
    fn internal_and_no_trade_actions_target_no_lps() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::CrossInternal {
            instrument: "EURUSD".into(),
            max_size: HedgeSize::Overflow,
        });
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 6_000.0, "RATES"),
            &known(),
            1,
        );
        assert!(out.intent.lps.is_empty(), "an internal cross targets no LP");
        assert!(out.provenance.unwrap().lps.is_empty());
    }

    #[test]
    fn stale_panel_id_degrades_to_full_known_set() {
        // A panel naming an LP the registry no longer knows resolves safely to the full
        // known set (never drops the hedge).
        let cfg = panel_cfg(HedgeScopeKind::Book, "RATES-EUR", &["GHOST"], &[]);
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let out = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(out.intent.lps, vec!["LP-1", "LP-2", "LP-3", "LP-4"]);
    }
}
