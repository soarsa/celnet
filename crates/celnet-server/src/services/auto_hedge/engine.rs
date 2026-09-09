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
use std::sync::{Arc, OnceLock, RwLock};

use tokio::sync::broadcast;

use celnet_hedge_routing::{
    ExitAction, HedgeContext, HedgeGraph, HedgeRouter, HedgeSize, LimitMetric, RagStatus,
    netting_split,
};
use celnet_proto::{
    DecisionEngineEnum, DecisionOutcomeEnum, DecisionRecord, ExitActionDesc, HedgeExitModeEnum,
    HedgeIntent, HedgeProvenance,
};

use super::wire::{band_label, exit_action_to_wire, exit_action_with_vehicle_to_wire};
use crate::config::hedge_policy::{HedgeConfigDef, HedgeMetric, HedgeThresholdDef};
use crate::services::decision_journal::{DecisionJournal, decision};

/// The default bounded depth of the fired-hedge provenance ring (the audit trail the
/// `ListHedgeProvenance` RPC reads). Old records fall off the back — a control-plane
/// history, not an unbounded log.
pub const PROVENANCE_RING_CAPACITY: usize = 4_096;

/// The depth of the live **intent broadcast** every connected client drains. Bounded: a
/// client that falls this far behind is LAGGED (it drops the backlog and resumes at the
/// newest state) rather than back-pressuring the booking tier — one slow socket must never
/// stall risk decisions (`CLAUDE.md` §11, the same discipline as the WS RFS channel).
const INTENT_CHANNEL_DEPTH: usize = 256;

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
    /// The **decision-journal** row for this evaluation — present on EVERY outcome,
    /// including the ones that fired nothing, carrying the walked path and the stated
    /// reason. When a journal is attached ([`AutoHedgeEngine::set_journal`]) this row has
    /// already been appended to it and its `seq` is assigned; otherwise `seq` is 0 and
    /// the row is informational only.
    pub decision: DecisionRecord,
}

/// The call-site context a journal row needs that the pure decision inputs do not carry:
/// which scoped policy was resolved, and the keys that join this decision back to the
/// fill and the lift trace it came from.
///
/// Defaulted (all-absent) by [`AutoHedgeEngine::evaluate`]; the live booking path supplies
/// a real one through [`AutoHedgeEngine::evaluate_with_meta`].
#[derive(Debug, Clone, Default)]
pub struct DecisionMeta {
    /// Which policy actually governed — `"book:rates-usd"`, `"bucket:emea"`, `"firm"`, or
    /// [`SCOPE_NONE`] when no policy was authored at any scope and the built-in default
    /// graph stood in.
    pub scope: String,
    /// The end-to-end lift trace this decision belongs to.
    pub trace_id: Option<u64>,
    /// The booked fill that provoked the evaluation.
    pub position_id: Option<u64>,
    /// The counterparty whose fill provoked it.
    pub counterparty: Option<String>,
    /// The traded security behind the family label.
    pub symbol: Option<String>,
}

/// The width of one **rate-limit interval** — the window `max_hedges_per_interval` counts
/// against.
///
/// The cap exists so "a misconfigured policy or a market gap can't machine-gun the LP
/// panel" (`docs/HEDGING-CONFIGURATION-GUIDE.md` §Guardrails), i.e. it is a **burst**
/// guard. One minute is the window that reading implies: it bounds the panel to
/// `max_hedges_per_interval` clips a minute while leaving a normally-active desk (a few
/// hedges a minute) entirely untouched.
///
/// This is a constant rather than a config field because the tunable the trader was given
/// is the *count*; a desk changes how many hedges it tolerates per burst, not what a burst
/// means. Widening it to a config field is a wire change with no requested use.
const RATE_LIMIT_INTERVAL_NANOS: i64 = 60 * 1_000_000_000;

/// Nanoseconds in a UTC day — the width of the `daily_external_notional_cap` window.
const NANOS_PER_DAY: i64 = 24 * 60 * 60 * 1_000_000_000;

/// The [`DecisionMeta::scope`] value meaning "the firm authored no hedge policy at any
/// scope governing this book; the built-in default graph stood in".
///
/// This is the string the [rule advisor](crate::services::rule_advisor) matches to derive
/// the "this book has no hedge policy" suggestion, so it is a constant rather than a
/// literal typed twice.
pub const SCOPE_NONE: &str = "none: no hedge policy configured for scope";

/// The mutable engine state behind one lock: the provenance ring + the rate-guard counters.
#[derive(Debug)]
struct Inner {
    ring: VecDeque<HedgeProvenance>,
    next_hedge: u64,
    /// Hedges fired live (non-advisory external) in the current rate-limit interval.
    hedges_in_interval: u32,
    /// When the current rate-limit interval opened (the injected fire clock, not wall
    /// time). [`AutoHedgeEngine::roll_windows`] advances this; `0` means "no interval has
    /// opened yet", which the first evaluation rolls.
    interval_opened_at_nanos: i64,
    /// Externalised notional booked live in the current daily window.
    daily_external: f64,
    /// The UTC day index (`now_nanos / NANOS_PER_DAY`) the `daily_external` running total
    /// belongs to. A fire on a later day rolls the total back to zero.
    daily_window_day: i64,
}

/// The off-core auto-hedge decision + provenance engine. Shared behind an `Arc` between
/// the control loop (which calls [`Self::evaluate`]) and the `ListHedgeProvenance` handler
/// (which calls [`Self::provenance`]).
#[derive(Debug)]
pub struct AutoHedgeEngine {
    inner: RwLock<Inner>,
    capacity: usize,
    /// The live **intent broadcast** — every resolved decision, published for the WS
    /// per-connection forwarder that renders the risk surface's RAG board.
    ///
    /// Historically the intent was computed and then dropped on the floor: nothing ever
    /// published it, so the GUI's per-book risk panel derived its board from a stream that
    /// never produced a frame and read "Waiting for the first risk-state tick…" forever.
    /// This channel is the missing publication. Sending is non-blocking and ignores the
    /// no-subscriber case, so an unwatched server pays nothing.
    intent_tx: broadcast::Sender<HedgeIntent>,
    /// The shared **decision journal** every evaluation lands in — fired and no-action
    /// alike. Attached once at boot ([`Self::set_journal`]) because the same ring is
    /// shared with the acceptance and routing producers; an engine with none attached
    /// still BUILDS its journal row (returned on [`HedgeOutcome::decision`]) and simply
    /// does not persist it, which is what the unit tests exercise.
    journal: OnceLock<Arc<DecisionJournal>>,
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
                interval_opened_at_nanos: 0,
                daily_external: 0.0,
                daily_window_day: 0,
            }),
            capacity: capacity.max(1),
            intent_tx: broadcast::channel(INTENT_CHANNEL_DEPTH).0,
            journal: OnceLock::new(),
        }
    }

    /// Attach the process-wide decision journal. Called once at boot, after the engine is
    /// already behind its `Arc` (the journal is shared with the acceptance / routing
    /// producers, so it is constructed alongside rather than owned here). A second call is
    /// ignored — the journal is a single boot-time binding, never re-pointed at runtime.
    pub fn set_journal(&self, journal: Arc<DecisionJournal>) {
        let _ = self.journal.set(journal);
    }

    /// The attached decision journal, if any.
    #[must_use]
    pub fn journal(&self) -> Option<&Arc<DecisionJournal>> {
        self.journal.get()
    }

    /// Persist one journal row (assigning its `seq`) when a journal is attached; return it
    /// either way so the outcome always carries the row that describes it.
    fn journal_row(&self, rec: DecisionRecord) -> DecisionRecord {
        match self.journal.get() {
            Some(j) => j.record(rec),
            None => rec,
        }
    }

    /// Subscribe to the live intent stream. Each WS connection holds one receiver and
    /// forwards frames onto its outbound sink; a receiver that lags is dropped-and-resumed
    /// by the broadcast channel rather than blocking the publisher.
    #[must_use]
    pub fn subscribe_intents(&self) -> broadcast::Receiver<HedgeIntent> {
        self.intent_tx.subscribe()
    }

    /// Publish one resolved intent to every connected subscriber. A send with no
    /// subscribers is not an error — it is the ordinary state of an unwatched server — so
    /// the result is deliberately discarded.
    fn publish(&self, intent: &HedgeIntent) {
        let _ = self.intent_tx.send(intent.clone());
    }

    /// Reset the per-interval rate counter — max-hedges-per-interval is a *rate*, not a
    /// lifetime cap.
    ///
    /// Correctness no longer depends on anyone calling this: [`Self::roll_windows`] rolls
    /// the window from the fire clock inside the guard itself. It stays public as the
    /// explicit "clear the burst counter now" operator/test lever.
    pub fn roll_rate_interval(&self) {
        let mut g = self.lock();
        g.hedges_in_interval = 0;
    }

    /// Reset the daily externalised-notional counter. As with [`Self::roll_rate_interval`],
    /// the day boundary is now rolled from the fire clock; this is the explicit lever.
    pub fn roll_daily_window(&self) {
        let mut g = self.lock();
        g.daily_external = 0.0;
    }

    /// Record a **per-fill external hedge-execution** record into the shared provenance ring
    /// the `ListHedgeProvenance` RPC serves, assigning it a fresh `hedge_id`; returns the
    /// stored record. Unlike the book-level advisory-intent records [`Self::evaluate`] stamps
    /// on a threshold breach, this is keyed to a single fill (`parent_position_id`) and carries
    /// the real shed price/mid, so the Hedge Desk populates and reconciles to the originating
    /// B2B deal even when the book-level band did not itself breach (a below-min-edge fill is
    /// backed-to-back regardless of the book's RAG band). The caller supplies the fully-built
    /// record (all economics computed off the fill); this method only mints the id + rings it.
    pub fn record_execution(&self, mut prov: HedgeProvenance) -> HedgeProvenance {
        let mut g = self.lock();
        let id = g.next_hedge;
        g.next_hedge = g.next_hedge.saturating_add(1);
        prov.hedge_id = format!("HDG-{id}");
        if g.ring.len() >= self.capacity {
            g.ring.pop_front();
        }
        g.ring.push_back(prov.clone());
        prov
    }

    /// **Amend** the book-level DECISION record [`Self::evaluate`] just rang (matched by its
    /// minted `hedge_id`) IN PLACE with the REALISED execution economics, so a fired hedge leaves
    /// EXACTLY ONE ring record — the pre-execution intent superseded by the real fill (its price /
    /// mid / signed slippage / winning LP / `parent_position_id`) rather than a duplicate row.
    /// This is the paired counterpart to `evaluate`'s self-ring on the live rates booking path:
    /// `evaluate` decides + rings the intent, then the executor amends that same record with the
    /// realised fill, keeping the `hedge_id` stable (so the event-trace `HedgeFired` stage, which
    /// carries the decision id, still resolves to the now-realised record).
    ///
    /// `realised.hedge_id` is overwritten with `hedge_id`, so the caller need not carry it. If the
    /// decision record has already aged out of the bounded ring (only under extreme churn between
    /// the decide and the amend), the realised record is appended instead so a real fill is never
    /// dropped — still exactly one record for that fill. Returns the stored record.
    pub fn amend_execution(
        &self,
        hedge_id: &str,
        mut realised: HedgeProvenance,
    ) -> HedgeProvenance {
        realised.hedge_id = hedge_id.to_owned();
        let mut g = self.lock();
        if let Some(slot) = g.ring.iter_mut().find(|p| p.hedge_id == hedge_id) {
            *slot = realised.clone();
        } else {
            if g.ring.len() >= self.capacity {
                g.ring.pop_front();
            }
            g.ring.push_back(realised.clone());
        }
        realised
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
        self.evaluate_with_meta(
            graph,
            threshold,
            config,
            ctx,
            known_lps,
            now_nanos,
            &DecisionMeta::default(),
        )
    }

    /// [`Self::evaluate`], plus the call-site [`DecisionMeta`] the journal row needs
    /// (which scoped policy governed, and the trace / position / counterparty keys that
    /// join this decision to the fill it came from). The live booking path calls this;
    /// `evaluate` is the same decision with an empty meta.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_with_meta(
        &self,
        graph: &HedgeGraph,
        threshold: &HedgeThresholdDef,
        config: &HedgeConfigDef,
        ctx: &HedgeContext,
        known_lps: &BTreeSet<String>,
        now_nanos: i64,
        meta: &DecisionMeta,
    ) -> HedgeOutcome {
        let wh = threshold.to_threshold();
        let net_risk = net_risk_for(threshold.metric, ctx);
        let band = wh.classify(net_risk);
        let utilization = wh.utilization(net_risk);
        let overflow = wh.overflow(net_risk);

        // Guard: a kill-switched or disabled desk simply warehouses — no action fires.
        if !config.desk_active(&ctx.desk) {
            let intent = warehouse_intent(
                ctx,
                band,
                net_risk,
                wh.cap,
                utilization,
                overflow,
                now_nanos,
                "halted: kill-switch / desk disabled",
            );
            self.publish(&intent);
            // The graph never ran, so there is no path to record — but the desk still gets
            // a row that says exactly WHY nothing happened, rather than silence.
            let decision = self.journal_row(hedge_row(
                DecisionOutcomeEnum::DecisionOutcomeNoAction,
                "HALTED",
                "halted: kill-switch / desk disabled — the hedge policy graph was not \
                 evaluated",
                &[],
                ctx,
                threshold.metric,
                &wh,
                net_risk,
                band,
                utilization,
                false,
                now_nanos,
                meta,
            ));
            return HedgeOutcome {
                intent,
                provenance: None,
                decision,
            };
        }

        // Resolve the policy. A validated graph never errors; a structurally broken one
        // (an unvalidated store) degrades safely to a warehouse hold, never a panic.
        let Ok(resolution) = HedgeRouter::resolve(graph, ctx) else {
            let intent = warehouse_intent(
                ctx,
                band,
                net_risk,
                wh.cap,
                utilization,
                overflow,
                now_nanos,
                "policy graph did not resolve",
            );
            self.publish(&intent);
            let decision = self.journal_row(hedge_row(
                DecisionOutcomeEnum::DecisionOutcomeNoAction,
                "UNRESOLVED",
                "policy graph did not resolve — a structurally broken graph degraded to a \
                 warehouse hold",
                &[],
                ctx,
                threshold.metric,
                &wh,
                net_risk,
                band,
                utilization,
                false,
                now_nanos,
                meta,
            ));
            return HedgeOutcome {
                intent,
                provenance: None,
                decision,
            };
        };
        let action = resolution.action.clone();
        // The leaf's hedge VEHICLE rides onto the wire action alongside the action itself, so
        // the booking path can size the DV01 ratio against it. Dropping it here was the
        // difference between "hedge into the 10Y future" and a silent self-hedge.
        let vehicle = resolution.vehicle.clone();
        let path = resolution.path.clone();

        // A green-band Warehouse hold changes nothing — emit the intent, stamp no record.
        if matches!(action, ExitAction::Warehouse) {
            let intent = HedgeIntent {
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
                policy_path: path.clone(),
                reason: format!("{} · WAREHOUSE", band_label(band)),
                lps: Vec::new(),
                // A hold trades nothing, so it names no vehicle and no mode matters.
                vehicle_plan: None,
                exit_mode: HedgeExitModeEnum::HedgeExitModeAuto as i32,
            };
            self.publish(&intent);
            // THE case the audit log exists for: the trader's own graph ran, walked
            // `path`, and chose to hold. No provenance is stamped (nothing traded), but the
            // journal records the walk so "why did my hedge rule not fire" is answerable —
            // and the recorded band says whether holding was the sane call.
            let decision = self.journal_row(hedge_row(
                DecisionOutcomeEnum::DecisionOutcomeNoAction,
                "WAREHOUSE",
                format!(
                    "{} · WAREHOUSE — the policy graph resolved a warehouse hold at node {}",
                    band_label(band),
                    path.last().map_or_else(|| "?".to_owned(), u32::to_string),
                ),
                &path,
                ctx,
                threshold.metric,
                &wh,
                net_risk,
                band,
                utilization,
                false,
                now_nanos,
                meta,
            ));
            return HedgeOutcome {
                intent,
                provenance: None,
                decision,
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

        // Advisory gate: the whole decision is a shadow run when the execution mode is
        // `Advisory` (nothing books — internal OR external); the three live modes execute.
        // A live external fire may still be DOWNGRADED to advisory by the rate/size guards
        // below (kill-switch is handled earlier via `desk_active`).
        let is_external = action.is_external();
        let mut advisory = config.execution.is_advisory();
        let mut guard_reason: Option<&str> = None;

        // Rate / size guards apply only to a live (non-advisory) external fire.
        if is_external && !advisory {
            let mut g = self.lock();
            // Roll the burst / daily windows FIRST, off the same injected fire clock the
            // record is stamped with. Without this the counters only ever climb, so both
            // caps degrade from a rate into a LIFETIME cap and every hedge past the
            // `max_hedges_per_interval`-th is silently downgraded to advisory until the
            // process restarts — the desk goes on stamping intents while nothing reaches
            // the street. Observed live on UAT 2026-08-19: `max_hedges_per_interval = 30`,
            // and hedges HDG-31 onward were all advisory with `external_hedged = 0` while
            // book utilisation climbed to 1.000 and stayed there.
            Self::roll_windows(&mut g, now_nanos);
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
        let action_wire = exit_action_with_vehicle_to_wire(&action, &vehicle);

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
            // The engine is the PURE decision layer: it resolves WHAT and HOW MUCH, but the
            // vehicle DV01 ratio needs the fill's own instrument terms + the firm's vehicle
            // registry, which live on the booking path. The booking path fills these in on
            // the record it amends (`RatesPositionStore::stamp_internalise`), so they are
            // honestly absent here rather than half-computed.
            vehicle_plan: None,
            exit_mode: HedgeExitModeEnum::HedgeExitModeAuto as i32,
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

        self.publish(&intent);
        // A fired decision journals too, so the audit table is ONE stream a trader sorts
        // and filters — the fired rows carry the minted `hedge_id`, which joins straight
        // back to the `HedgeProvenance` record with the realised economics.
        let mut row = hedge_row(
            DecisionOutcomeEnum::DecisionOutcomeFired,
            action.kind(),
            intent.reason.clone(),
            &path,
            ctx,
            threshold.metric,
            &wh,
            net_risk,
            band,
            utilization,
            advisory,
            now_nanos,
            meta,
        );
        row.hedge_id = Some(provenance.hedge_id.clone());
        let decision = self.journal_row(row);
        HedgeOutcome {
            intent,
            provenance: Some(provenance),
            decision,
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
            // A book-level advisory-intent record is not keyed to a single fill — the per-fill
            // execution record (`RatesPositionStore::stamp_internalise` → `record_execution`)
            // carries the parent position id for deal reconciliation.
            parent_position_id: None,
            // Sized on the booking path (see the intent's note above), not in the pure
            // decision layer.
            vehicle_plan: None,
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

    /// Advance the burst / daily guard windows to the interval `now_nanos` falls in,
    /// zeroing whichever counters belong to a window that has since closed.
    ///
    /// Driven by the **injected** fire clock rather than wall time, so the guards need no
    /// background task to stay honest and a test can walk a desk across a boundary by
    /// advancing the timestamp it already passes to [`Self::evaluate`].
    ///
    /// A first-ever fire (`interval_opened_at_nanos == 0`) simply opens the first window;
    /// the counters are already zero, so opening is the whole effect.
    fn roll_windows(g: &mut Inner, now_nanos: i64) {
        if now_nanos.saturating_sub(g.interval_opened_at_nanos) >= RATE_LIMIT_INTERVAL_NANOS {
            g.hedges_in_interval = 0;
            g.interval_opened_at_nanos = now_nanos;
        }
        let day = now_nanos.div_euclid(NANOS_PER_DAY);
        if day != g.daily_window_day {
            g.daily_external = 0.0;
            g.daily_window_day = day;
        }
    }
}

/// Build one hedge-engine [`DecisionRecord`] from the state the evaluation actually saw.
///
/// Every field is read straight off the inputs — the band, the utilisation, the walked
/// path, the resolved cap. `reason` is required by the caller (there is no default), which
/// is how the "a no-action decision must state its reason" rule is enforced structurally
/// rather than by convention.
#[allow(clippy::too_many_arguments)]
fn hedge_row(
    outcome: DecisionOutcomeEnum,
    outcome_label: impl Into<String>,
    reason: impl Into<String>,
    path: &[u32],
    ctx: &HedgeContext,
    metric: HedgeMetric,
    wh: &celnet_hedge_routing::WarehouseThreshold,
    net_risk: f64,
    band: RagStatus,
    utilization: f64,
    advisory: bool,
    now_nanos: i64,
    meta: &DecisionMeta,
) -> DecisionRecord {
    let mut reason = reason.into();
    // A book governed by NO authored policy is the single most actionable audit fact
    // there is, so it rides on the reason string as well as the `scope` field — the rule
    // advisor keys the "this book has no hedge policy" suggestion off it.
    if meta.scope == SCOPE_NONE {
        reason = format!("{reason} · {SCOPE_NONE}");
    }
    DecisionRecord {
        policy_path: path.to_vec(),
        scope: meta.scope.clone(),
        book: ctx.book.clone(),
        instrument: ctx.instrument_id.clone(),
        counterparty: meta.counterparty.clone(),
        symbol: meta.symbol.clone(),
        desk: ctx.desk.clone(),
        position_id: meta.position_id,
        trace_id: meta.trace_id,
        metric: metric.as_i32(),
        net_risk,
        threshold: wh.cap,
        utilization,
        band: band_label(band).to_owned(),
        advisory,
        ..decision(
            DecisionEngineEnum::DecisionEngineHedge,
            outcome,
            outcome_label,
            reason,
            now_nanos,
        )
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
        // Gross is already unsigned; `abs()` is belt-and-braces so a caller that
        // mis-signs the roll-up cannot manufacture a negative "gross".
        HedgeMetric::GrossNotional => ctx.gross_notional.abs(),
    }
}

/// The [`HedgeSize`] a size-bearing action requests (or `None` for a no-shed action).
fn requested_size(action: &ExitAction) -> Option<HedgeSize> {
    match action {
        ExitAction::CrossInternal { max_size, .. } => Some(*max_size),
        ExitAction::SubmitMarketOrder { size, .. } | ExitAction::RfqOut { size, .. } => Some(*size),
        // ClearRisk flattens the whole net position — a full-size external market order.
        ExitAction::ClearRisk => Some(HedgeSize::Full),
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
        // Straight externalisation — all to the street. `ClearRisk` sizes to `Full`
        // (the whole net), so it flattens the book to zero.
        ExitAction::SubmitMarketOrder { .. }
        | ExitAction::RfqOut { .. }
        | ExitAction::ClearRisk => Split {
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
        vehicle_plan: None,
        exit_mode: HedgeExitModeEnum::HedgeExitModeAuto as i32,
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
        nodes.insert(1, HedgeNode::action(ExitAction::Warehouse));
        nodes.insert(2, HedgeNode::action(hedge));
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
    fn advisory_mode_shadow_runs_and_records_provenance() {
        use crate::config::hedge_policy::HedgeExecutionMode;
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            execution: HedgeExecutionMode::Advisory,
            ..HedgeConfigDef::default()
        };
        // net 95k DV01, breached, threshold edge 80k → overflow 15k.
        let out = e.evaluate(
            &g,
            &thr(),
            &cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            42,
        );
        let prov = out.provenance.expect("an action stamps provenance");
        assert!(out.intent.advisory, "Advisory mode never trades");
        assert!(prov.advisory);
        assert_eq!(out.intent.external_hedged, 15_000.0);
        assert_eq!(prov.internal_crossed, 0.0);
        assert_eq!(e.provenance(None, None).len(), 1);
        assert_eq!(e.provenance(Some("RATES-EUR"), None).len(), 1);
        assert!(e.provenance(Some("OTHER"), None).is_empty());
    }

    /// A `counterparty == "X"` rule back-to-backs X's flow externally while any other
    /// counterparty's fill warehouses — driven through the full `evaluate` path.
    #[test]
    fn counterparty_rule_backs_to_back_matching_flow_only() {
        use celnet_hedge_routing::HedgeNode;
        // Graph: `counterparty == "CITADEL" ? SubmitMarketOrder(Full) : Warehouse`.
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            HedgeNode::Condition {
                field: celnet_hedge_routing::HedgeField::Counterparty,
                op: RouteOp::Eq,
                value: RouteValue::Text("CITADEL".into()),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1,
            HedgeNode::action(ExitAction::SubmitMarketOrder {
                size: HedgeSize::Full,
                style: ExecStyle::Immediate,
            }),
        );
        nodes.insert(2, HedgeNode::action(ExitAction::Warehouse));
        let g = HedgeGraph { entry: 0, nodes };
        let e = AutoHedgeEngine::default();

        // A CITADEL fill (breached book) → external back-to-back, provenance stamped.
        let citadel = HedgeContext {
            counterparty: "CITADEL".into(),
            ..ctx(95_000.0, true, 0.0, "RATES")
        };
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &citadel,
            &known(),
            1,
        );
        assert_eq!(
            out.intent.action.unwrap().kind,
            ExitActionKind::ExitActionSubmitMarketOrder as i32,
            "CITADEL flow is backed-to-back"
        );
        assert!(out.provenance.is_some(), "a fired hedge stamps provenance");

        // Any other counterparty's fill → warehouse, no provenance.
        let other = HedgeContext {
            counterparty: "MILLENNIUM".into(),
            ..ctx(95_000.0, true, 0.0, "RATES")
        };
        let out2 = e.evaluate(&g, &thr(), &HedgeConfigDef::default(), &other, &known(), 2);
        assert_eq!(
            out2.intent.action.unwrap().kind,
            ExitActionKind::ExitActionWarehouse as i32,
            "non-CITADEL flow warehouses"
        );
        assert!(out2.provenance.is_none(), "a warehouse hold stamps nothing");
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
            // Default execution is already a live mode; the rate cap gates the second fire.
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

    /// The rate cap is a **rate**, not a lifetime cap: once the interval it counts against
    /// has elapsed, the desk fires live again with nobody having called anything.
    ///
    /// Regression for the UAT report of 2026-08-19. `hedges_in_interval` only ever
    /// incremented — `roll_rate_interval` existed but had **no production caller** — so a
    /// desk configured `max_hedges_per_interval: 30` fired 30 live hedges and then
    /// downgraded every subsequent one to advisory *for the life of the process*. The
    /// symptom is silent and looks like a market problem: intents keep being stamped,
    /// `external_hedged` is 0 on every one of them, and book utilisation climbs to 1.000
    /// and stays pinned there while nothing reaches the street.
    #[test]
    fn the_rate_cap_is_a_rate_not_a_lifetime_cap() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Full,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            max_hedges_per_interval: 1,
            ..HedgeConfigDef::default()
        };
        // A realistic fire clock — the bug is invisible at t≈0, where every timestamp
        // lands in the first window anyway.
        let t0: i64 = 1_787_000_000_000_000_000;
        let fire = |at: i64| {
            e.evaluate(
                &g,
                &thr(),
                &cfg,
                &ctx(95_000.0, true, 0.0, "RATES"),
                &known(),
                at,
            )
        };

        assert!(!fire(t0).intent.advisory, "first live external fire");
        assert!(
            fire(t0 + 1_000_000_000).intent.advisory,
            "a second fire one second later is inside the same interval — capped"
        );

        // One interval later the window has closed. NOTHING calls roll_rate_interval here:
        // that is the whole point — correctness must not depend on an external control
        // loop that does not exist.
        let next = fire(t0 + RATE_LIMIT_INTERVAL_NANOS);
        assert!(
            !next.intent.advisory,
            "the interval elapsed, so the desk fires live again — a cap that never rolls \
             is a lifetime cap and silently retires the desk"
        );
        assert!(
            !next.intent.reason.contains("rate cap"),
            "a rolled window must not still blame the rate cap"
        );
    }

    /// The daily externalised-notional cap rolls at the UTC day boundary, for the same
    /// reason and by the same clock.
    #[test]
    fn the_daily_notional_cap_rolls_at_the_day_boundary() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Full,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            daily_external_notional_cap: 150_000.0,
            ..HedgeConfigDef::default()
        };
        // Day boundary in the middle of the walk: 95k fits, a second 95k would breach.
        let day = 20_680_i64;
        let late = day * NANOS_PER_DAY + 23 * 60 * 60 * 1_000_000_000;

        assert!(
            !fire_at(&e, &g, &cfg, late).intent.advisory,
            "first fire fits"
        );
        assert!(
            fire_at(&e, &g, &cfg, late + 60 * 1_000_000_000)
                .intent
                .advisory,
            "the second 95k breaches the 150k daily cap on the same day"
        );
        assert!(
            !fire_at(&e, &g, &cfg, (day + 1) * NANOS_PER_DAY)
                .intent
                .advisory,
            "the next UTC day starts a fresh notional budget"
        );
    }

    /// Shared driver for the day-boundary walk above — one live external fire at `at`.
    fn fire_at(
        e: &AutoHedgeEngine,
        g: &HedgeGraph,
        cfg: &HedgeConfigDef,
        at: i64,
    ) -> crate::services::auto_hedge::engine::HedgeOutcome {
        e.evaluate(
            g,
            &thr(),
            cfg,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            at,
        )
    }

    #[test]
    fn daily_external_cap_forces_advisory() {
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Full,
            style: ExecStyle::Immediate,
        });
        let cfg = HedgeConfigDef {
            // Default execution is live; the daily cap downgrades the fire to advisory.
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
    fn clear_risk_flattens_the_whole_net_externally() {
        // ClearRisk sizes to Full (the whole net), not the overflow-to-edge — a full
        // external flatten. net 95k → external 95k (not the 15k overflow).
        let e = AutoHedgeEngine::default();
        let g = graph(ExitAction::ClearRisk);
        let out = e.evaluate(
            &g,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            1,
        );
        assert_eq!(
            out.intent.action.as_ref().unwrap().kind,
            ExitActionKind::ExitActionClearRisk as i32
        );
        assert_eq!(
            out.intent.external_hedged, 95_000.0,
            "ClearRisk flattens the whole net, not just the overflow"
        );
        assert_eq!(out.intent.internal_crossed, 0.0);
        let prov = out.provenance.expect("a fired flatten stamps provenance");
        assert_eq!(prov.residual, 0.0, "nothing is warehoused after a flatten");
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

    // ---------------------------------------------------------------------
    // decision-journal emission — the "why did nothing fire" audit trail
    // ---------------------------------------------------------------------

    /// Attach a journal and return it beside the engine.
    fn journaled(capacity: usize) -> (AutoHedgeEngine, Arc<DecisionJournal>) {
        let e = AutoHedgeEngine::new(capacity);
        let j = Arc::new(DecisionJournal::new(64));
        e.set_journal(Arc::clone(&j));
        (e, j)
    }

    /// THE gap this feature closes: a green-band evaluation walks the trader's graph to a
    /// WAREHOUSE leaf, stamps NO provenance (nothing traded) — and must still leave an
    /// audit row carrying the walked path and a stated reason.
    #[test]
    fn a_warehouse_hold_stamps_no_provenance_but_records_the_walked_path_and_reason() {
        let (e, j) = journaled(8);
        let out = e.evaluate(
            &g_market(),
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(10_000.0, false, 0.0, "RATES"),
            &known(),
            77,
        );
        assert!(out.provenance.is_none(), "a hold trades nothing");
        assert!(e.provenance(None, None).is_empty());

        let rows = j.query(&crate::services::decision_journal::JournalQuery::default());
        assert_eq!(rows.records.len(), 1, "the hold IS recorded");
        let r = &rows.records[0];
        assert_eq!(r.engine, DecisionEngineEnum::DecisionEngineHedge as i32);
        assert_eq!(
            r.outcome,
            DecisionOutcomeEnum::DecisionOutcomeNoAction as i32
        );
        assert_eq!(r.outcome_label, "WAREHOUSE");
        assert!(
            r.reason.contains("WAREHOUSE"),
            "the reason states what happened: {}",
            r.reason
        );
        assert_eq!(r.policy_path, vec![0, 1], "the exact walk is recorded");
        assert_eq!(r.band, "green");
        assert_eq!(r.book, "RATES-EUR");
        assert_eq!(r.decided_at, 77);
        assert!((r.threshold - 100_000.0).abs() < 1e-12);
        assert!((r.net_risk - 10_000.0).abs() < 1e-12);
        assert_eq!(out.decision.seq, r.seq, "the outcome carries the same row");
    }

    /// A kill-switched desk suppresses the decision BEFORE the graph runs. There is no
    /// path to record — but there is a reason, and it must be recorded rather than
    /// returning silence.
    #[test]
    fn a_kill_switched_desk_records_an_empty_path_with_a_stated_reason() {
        let (e, j) = journaled(8);
        let config = HedgeConfigDef {
            kill_switch: true,
            ..HedgeConfigDef::default()
        };
        let out = e.evaluate(
            &g_market(),
            &thr(),
            &config,
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            5,
        );
        assert!(out.provenance.is_none());
        let rows = j.query(&crate::services::decision_journal::JournalQuery::default());
        assert_eq!(rows.records.len(), 1);
        let r = &rows.records[0];
        assert_eq!(r.outcome_label, "HALTED");
        assert!(r.policy_path.is_empty(), "the graph never ran");
        assert!(
            r.reason.contains("kill-switch"),
            "the reason names the suppressor: {}",
            r.reason
        );
    }

    /// A structurally broken graph degrades to a warehouse hold. That degradation used to
    /// be invisible; it is now an audit row that says so.
    #[test]
    fn an_unresolvable_graph_records_why_it_could_not_decide() {
        let (e, j) = journaled(8);
        // An entry pointing at a node that does not exist.
        let broken = HedgeGraph {
            entry: 9,
            nodes: BTreeMap::new(),
        };
        let out = e.evaluate(
            &broken,
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            11,
        );
        assert!(out.provenance.is_none());
        let r = &j
            .query(&crate::services::decision_journal::JournalQuery::default())
            .records[0];
        assert_eq!(r.outcome_label, "UNRESOLVED");
        assert!(r.reason.contains("did not resolve"), "{}", r.reason);
    }

    /// A fired decision journals too, and its row carries the minted `hedge_id` so the
    /// audit table joins straight back to the realised `HedgeProvenance`.
    #[test]
    fn a_fired_decision_records_a_row_that_joins_to_its_provenance() {
        let (e, j) = journaled(8);
        let out = e.evaluate(
            &g_market(),
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(95_000.0, true, 0.0, "RATES"),
            &known(),
            21,
        );
        let prov = out.provenance.expect("an external action fires");
        let r = &j
            .query(&crate::services::decision_journal::JournalQuery::default())
            .records[0];
        assert_eq!(r.outcome, DecisionOutcomeEnum::DecisionOutcomeFired as i32);
        assert_eq!(r.hedge_id.as_deref(), Some(prov.hedge_id.as_str()));
        assert_eq!(r.policy_path, prov.policy_path);
        assert_eq!(r.band, prov.band);
    }

    /// The rate guard downgrades a live external fire to advisory. The row records BOTH
    /// the downgrade and the reason, which is what the rule advisor keys off.
    #[test]
    fn a_rate_guard_downgrade_is_recorded_with_its_reason() {
        let (e, j) = journaled(8);
        let config = HedgeConfigDef {
            max_hedges_per_interval: 1,
            ..HedgeConfigDef::default()
        };
        let c = ctx(95_000.0, true, 0.0, "RATES");
        // First fire consumes the interval budget; the second is downgraded.
        e.evaluate(&g_market(), &thr(), &config, &c, &known(), 1);
        e.evaluate(&g_market(), &thr(), &config, &c, &known(), 2);
        let rows = j.query(&crate::services::decision_journal::JournalQuery::default());
        assert_eq!(rows.records.len(), 2);
        let second = &rows.records[0];
        assert!(second.advisory, "the second fire is a shadow run");
        assert!(
            second.reason.contains("rate cap") && second.reason.contains("advisory"),
            "the downgrade states its cause: {}",
            second.reason
        );
    }

    /// The call-site metadata (which policy governed, and the join keys) rides onto the
    /// row — including the "no policy was authored at any scope" marker the advisor keys
    /// its strongest suggestion off.
    #[test]
    fn call_site_metadata_including_the_no_policy_marker_rides_onto_the_row() {
        let (e, j) = journaled(8);
        let meta = DecisionMeta {
            scope: SCOPE_NONE.to_owned(),
            trace_id: Some(4_242),
            position_id: Some(7),
            counterparty: Some("cp-a".to_owned()),
            symbol: Some("US10Y".to_owned()),
        };
        e.evaluate_with_meta(
            &g_market(),
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(10_000.0, false, 0.0, "RATES"),
            &known(),
            9,
            &meta,
        );
        let r = &j
            .query(&crate::services::decision_journal::JournalQuery::default())
            .records[0];
        assert_eq!(r.scope, SCOPE_NONE);
        assert!(
            r.reason.contains("no hedge policy configured for scope"),
            "the no-policy fact is on the reason too: {}",
            r.reason
        );
        assert_eq!(r.trace_id, Some(4_242));
        assert_eq!(r.position_id, Some(7));
        assert_eq!(r.counterparty.as_deref(), Some("cp-a"));
        assert_eq!(r.symbol.as_deref(), Some("US10Y"));
    }

    /// An engine with no journal attached behaves identically and still hands the caller
    /// the row describing the decision (unsequenced) — so the journal is observability,
    /// never a dependency of the decision.
    #[test]
    fn an_engine_without_a_journal_still_decides_and_reports_its_row() {
        let e = AutoHedgeEngine::new(8);
        let out = e.evaluate(
            &g_market(),
            &thr(),
            &HedgeConfigDef::default(),
            &ctx(10_000.0, false, 0.0, "RATES"),
            &known(),
            3,
        );
        assert!(out.provenance.is_none());
        assert_eq!(out.decision.seq, 0, "no journal ⇒ no sequence assigned");
        assert_eq!(out.decision.outcome_label, "WAREHOUSE");
        assert_eq!(out.decision.policy_path, vec![0, 1]);
    }

    #[test]
    fn amend_execution_supersedes_the_decision_record_in_place() {
        // A size-bearing external action self-rings ONE decision record; amending it by id must
        // REPLACE that record (with realised economics) rather than append a second row — the
        // no-double-record invariant the live rates booking path relies on.
        let e = AutoHedgeEngine::new(8);
        let g = graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        });
        let decision = e
            .evaluate(
                &g,
                &thr(),
                &HedgeConfigDef::default(),
                &ctx(95_000.0, true, 0.0, "RATES"),
                &known(),
                1,
            )
            .provenance
            .expect("an external action self-rings a decision record");
        assert_eq!(e.provenance(None, None).len(), 1);
        assert!(
            decision.lp_won.is_none(),
            "the decision carries no realised LP"
        );

        let mut realised = decision.clone();
        realised.lp_won = Some("LP-9".to_owned());
        realised.hedge_price = 1.2345;
        realised.slippage_bp = -1.0;
        realised.parent_position_id = Some(42);
        realised.advisory = false;
        let stored = e.amend_execution(&decision.hedge_id, realised);

        let ring = e.provenance(None, None);
        assert_eq!(ring.len(), 1, "amend REPLACES, never appends");
        assert_eq!(ring[0].hedge_id, decision.hedge_id, "the id is preserved");
        assert_eq!(ring[0].lp_won.as_deref(), Some("LP-9"));
        assert_eq!(ring[0].parent_position_id, Some(42));
        assert!((ring[0].slippage_bp - (-1.0)).abs() < 1e-12);
        assert_eq!(stored.hedge_id, decision.hedge_id);
    }

    #[test]
    fn amend_execution_appends_when_no_matching_decision() {
        // Defensive fallback: if the decision aged out (or was never rung), amend appends so a
        // real fill is never dropped — still exactly one record for that fill.
        let e = AutoHedgeEngine::new(8);
        let template = e
            .evaluate(
                &g_market(),
                &thr(),
                &HedgeConfigDef::default(),
                &ctx(95_000.0, true, 0.0, "RATES"),
                &known(),
                1,
            )
            .provenance
            .expect("template decision");
        assert_eq!(e.provenance(None, None).len(), 1);
        let stored = e.amend_execution("HDG-nonexistent", template);
        assert_eq!(
            e.provenance(None, None).len(),
            2,
            "an unmatched amend appends so the fill is never lost"
        );
        assert_eq!(stored.hedge_id, "HDG-nonexistent");
    }

    fn g_market() -> HedgeGraph {
        graph(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        })
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
