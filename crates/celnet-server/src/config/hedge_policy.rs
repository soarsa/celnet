//! Persisted **auto-hedge configuration** — the storage-side value types the
//! [`IdentityStore`](super::identity::IdentityStore) holds alongside the hedge
//! policy graph: the warehouse thresholds (the configurable "100" per scope) and
//! the engine controls (kill-switch, advisory-only, per-desk toggles, rate guards).
//!
//! These are **plain serde value types** so an `identity.json` round-trips them,
//! decoupled from `celnet-hedge-routing`'s [`WarehouseThreshold`] (which is `Copy`
//! and deliberately carries no serde dependency on `celnet-limits`). The pure engine
//! type is *reconstructed* from a [`HedgeThresholdDef`] via [`HedgeThresholdDef::to_threshold`],
//! so the RAG-band / sizing logic stays the single source of truth in the pure crate.
//!
//! The hedge policy graph itself ([`celnet_hedge_routing::HedgeGraph`]) already
//! derives `Serialize`/`Deserialize`, so it is stored directly on the identity store
//! exactly as `risk_routing_graph` is — no mirror type needed for the graph.
//!
//! `docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §4 / §8.4.

use celnet_hedge_routing::{
    HedgeExitMode, HedgeLpPanel, HedgeVehicleRegistry, HedgeVehicleRule, HedgingModel, LimitMetric,
    WarehouseThreshold,
};
use serde::{Deserialize, Serialize};

/// Persistence-serde helper for `f64` fields that can legitimately hold a **non-finite**
/// sentinel — chiefly `f64::INFINITY`, the "uncapped" [`HedgeThresholdDef::max_clip`] default
/// seeded on first boot ([`WarehouseThreshold::new`] uses `max_clip = ∞`), and any threshold
/// field a future config could set unbounded.
///
/// `serde_json` renders a non-finite `f64` as JSON `null`; the stock `f64` deserializer then
/// REJECTS that `null` (`invalid type: null, expected f64`), so the very next server boot after
/// such a value reaches `identity.json` fails to load and the process dies. This module makes
/// the round-trip total while keeping finite values byte-identical:
/// - **serialize**: a *finite* value is written as a bare JSON number (byte-identical to a plain
///   `f64`, so untouched configs are unchanged); a non-finite value is written as a
///   round-trippable string sentinel (`"inf"` / `"-inf"` / `"nan"`).
/// - **deserialize**: accepts a JSON number, one of those sentinel strings, OR a legacy `null`
///   (what the old serializer wrote for an uncapped `∞`) — the last heals an already-corrupted
///   `identity.json` on load instead of crashing.
///
/// Contained to the persisted config structs only: the in-memory `f64` type and the wire/proto
/// contract are unchanged.
mod nonfinite_f64 {
    use serde::de::{self, Unexpected, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    /// Serialize a possibly non-finite `f64`: finite ⇒ bare number, else a sentinel string.
    pub(super) fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() {
            serializer.serialize_f64(*value)
        } else if value.is_nan() {
            serializer.serialize_str("nan")
        } else if value.is_sign_positive() {
            serializer.serialize_str("inf")
        } else {
            serializer.serialize_str("-inf")
        }
    }

    /// Deserialize an `f64` that may arrive as a number, a non-finite sentinel string, or a
    /// legacy `null` (⇒ `f64::INFINITY`, the only non-finite value the old path ever wrote).
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        struct NonFiniteF64;

        impl<'de> Visitor<'de> for NonFiniteF64 {
            type Value = f64;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number, a non-finite sentinel (\"inf\"/\"-inf\"/\"nan\"), or null")
            }

            fn visit_f64<E>(self, v: f64) -> Result<f64, E> {
                Ok(v)
            }
            fn visit_i64<E>(self, v: i64) -> Result<f64, E> {
                Ok(v as f64)
            }
            fn visit_u64<E>(self, v: u64) -> Result<f64, E> {
                Ok(v as f64)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<f64, E> {
                match v.trim().to_ascii_lowercase().as_str() {
                    "inf" | "+inf" | "infinity" | "+infinity" => Ok(f64::INFINITY),
                    "-inf" | "-infinity" => Ok(f64::NEG_INFINITY),
                    "nan" => Ok(f64::NAN),
                    other => other
                        .parse::<f64>()
                        .map_err(|_| de::Error::invalid_value(Unexpected::Str(v), &self)),
                }
            }
            // Legacy `null` — what `serde_json` emitted for a non-finite `f64` before this fix —
            // heals to the "uncapped" `f64::INFINITY` the old serializer meant.
            fn visit_unit<E>(self) -> Result<f64, E> {
                Ok(f64::INFINITY)
            }
        }

        deserializer.deserialize_any(NonFiniteF64)
    }
}

/// The budget metric a warehouse threshold caps. A stored, serde-stable enum decoupled
/// from `celnet_limits::LimitMetric` (which is not `Serialize`); [`Self::to_limit_metric`]
/// maps it onto the pure-crate metric when a [`WarehouseThreshold`] is reconstructed.
///
/// Because RAG classification / utilisation depend only on `|exposure| / cap` (not on
/// *which* metric), the mapping is faithful for display while the pure band logic is
/// metric-agnostic. This enum is the display / wire source of truth for the budget kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeMetric {
    /// Net parallel DV01 (FI first-order rate risk).
    Dv01,
    /// Net base-currency notional.
    NetNotional,
    /// Net base-currency delta.
    NetDelta,
    /// Net vega.
    NetVega,
    /// **Gross** base-currency notional — `Σ|notional|` over the scope, which never
    /// nets down. A turnover brake rather than a risk budget: unlike every other
    /// metric here it only grows with activity, so a gross cap stays breached until
    /// the positions themselves roll off. Offered because desks manage a book by
    /// gross as well as by net/DV01, but it behaves fundamentally differently.
    GrossNotional,
}

impl HedgeMetric {
    /// Every metric, in stable order (the canonical iteration set).
    pub const ALL: [HedgeMetric; 5] = [
        HedgeMetric::Dv01,
        HedgeMetric::NetNotional,
        HedgeMetric::NetDelta,
        HedgeMetric::NetVega,
        HedgeMetric::GrossNotional,
    ];

    /// Stable snake_case label for audit / logging.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            HedgeMetric::Dv01 => "dv01",
            HedgeMetric::NetNotional => "net_notional",
            HedgeMetric::NetDelta => "net_delta",
            HedgeMetric::NetVega => "net_vega",
            HedgeMetric::GrossNotional => "gross_notional",
        }
    }

    /// Map to the pure-crate [`LimitMetric`] used to reconstruct a [`WarehouseThreshold`].
    /// `NetNotional`/`NetDelta` both map to the additive net base-currency delta metric
    /// (there is no distinct "net notional" `LimitMetric`); the RAG band is metric-agnostic.
    #[must_use]
    pub fn to_limit_metric(self) -> LimitMetric {
        match self {
            HedgeMetric::Dv01 => LimitMetric::Dv01,
            // `GrossNotional` shares the additive base-currency amount metric: the RAG
            // band is metric-agnostic (`|exposure| / cap`), and the gross-vs-net
            // distinction lives in WHICH roll-up is fed in, not in the band maths.
            HedgeMetric::NetNotional | HedgeMetric::NetDelta | HedgeMetric::GrossNotional => {
                LimitMetric::Delta
            }
            HedgeMetric::NetVega => LimitMetric::Vega,
        }
    }

    /// The proto `HedgeMetricEnum` ordinal (DV01=0, NET_NOTIONAL=1, NET_DELTA=2,
    /// NET_VEGA=3, GROSS_NOTIONAL=4).
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            HedgeMetric::Dv01 => 0,
            HedgeMetric::NetNotional => 1,
            HedgeMetric::NetDelta => 2,
            HedgeMetric::NetVega => 3,
            HedgeMetric::GrossNotional => 4,
        }
    }

    /// Parse from the proto `HedgeMetricEnum` ordinal; out-of-range folds to `Dv01`.
    #[must_use]
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => HedgeMetric::NetNotional,
            2 => HedgeMetric::NetDelta,
            3 => HedgeMetric::NetVega,
            4 => HedgeMetric::GrossNotional,
            _ => HedgeMetric::Dv01,
        }
    }
}

/// What a warehouse threshold's `scope_id` names (most-specific-wins resolution,
/// instrument > book > desk — §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeScopeKind {
    /// A whole desk's aggregate risk.
    Desk,
    /// A single risk book / portfolio.
    Book,
    /// A single instrument.
    Instrument,
}

impl HedgeScopeKind {
    /// The three scopes, in specificity order (least → most specific).
    pub const ALL: [HedgeScopeKind; 3] = [
        HedgeScopeKind::Desk,
        HedgeScopeKind::Book,
        HedgeScopeKind::Instrument,
    ];

    /// Stable snake_case label for audit / logging.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            HedgeScopeKind::Desk => "desk",
            HedgeScopeKind::Book => "book",
            HedgeScopeKind::Instrument => "instrument",
        }
    }

    /// The proto `HedgeScopeKindEnum` ordinal (DESK=0, BOOK=1, INSTRUMENT=2).
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            HedgeScopeKind::Desk => 0,
            HedgeScopeKind::Book => 1,
            HedgeScopeKind::Instrument => 2,
        }
    }

    /// Parse from the proto `HedgeScopeKindEnum` ordinal; out-of-range folds to `Desk`.
    #[must_use]
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => HedgeScopeKind::Book,
            2 => HedgeScopeKind::Instrument,
            _ => HedgeScopeKind::Desk,
        }
    }
}

/// How a live hedge decision is **executed** once the policy resolves an external exit
/// action — the "Both — config per policy" control
/// **When** a live external hedge runs relative to the fill that triggered it.
///
/// Distinct from [`HedgeExecutionMode`], which says WHERE it executes. This says whether
/// the client's booking commit waits for it.
///
/// The distinction is worth a config field because it is a genuine trade-off, not an
/// optimisation. `route()` blocks on a real venue round trip, so under
/// [`Sync`](HedgeDispatch::Sync) that wait sits on the client's critical path — measured
/// at ~6ms p99 on UAT, against a hedge DECISION of ~26us. Under
/// [`Async`](HedgeDispatch::Async) the fill is acknowledged first and the hedge follows
/// immediately after, off that path.
///
/// What you give up asynchronously is simultaneity: the offsetting leg and the
/// provenance record land a moment AFTER the fill returns rather than with it. A desk
/// that reconciles the two in the same breath — or a test that reads provenance straight
/// after booking — wants `Sync`. A desk that cares about acknowledgement latency wants
/// `Async`. Neither is universally right, which is exactly why it is configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HedgeDispatch {
    /// Run the external hedge inside the booking commit and wait for the venue.
    /// The historical behaviour, and the default.
    #[default]
    Sync,
    /// Book the fill first; run the external hedge immediately after, off the path.
    Async,
}

impl HedgeDispatch {
    /// Every dispatch, in stable (proto-ordinal) order.
    pub const ALL: [HedgeDispatch; 2] = [HedgeDispatch::Sync, HedgeDispatch::Async];

    /// Stable snake_case label for audit / logging.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            HedgeDispatch::Sync => "sync",
            HedgeDispatch::Async => "async",
        }
    }

    /// Whether the fill is acknowledged without waiting for the venue.
    #[must_use]
    pub const fn is_async(self) -> bool {
        matches!(self, HedgeDispatch::Async)
    }

    /// Wire ordinal → dispatch. An unknown ordinal reads as `Sync`: the safe direction,
    /// since a fill that waits is never wrong, only slower.
    #[must_use]
    pub const fn from_i32(v: i32) -> Self {
        match v {
            1 => HedgeDispatch::Async,
            _ => HedgeDispatch::Sync,
        }
    }

    /// Dispatch → wire ordinal.
    #[must_use]
    pub const fn to_i32(self) -> i32 {
        match self {
            HedgeDispatch::Sync => 0,
            HedgeDispatch::Async => 1,
        }
    }
}

/// (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §6). Replaces the former
/// boolean `advisory_only`: [`Advisory`](HedgeExecutionMode::Advisory) is the old dry-run
/// posture; the three live modes each name a concrete venue path the executor drives.
///
/// The mode is consulted only for **external** actions; internal crosses / no-trade
/// actions ignore it. `Advisory` in any mode means the engine stamps a real intent +
/// provenance but books nothing (the shadow run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeExecutionMode {
    /// Shadow-run: compute + stamp the intent / provenance but never trade.
    Advisory,
    /// Externalise onto the LP-sim RFQ/FIX panel; an unfilled clip records an honest miss.
    LpPanel,
    /// Externalise against the Agg Book COMPOSITE mid (spread applied); always fills.
    Composite,
    /// Try the LP panel first; on no fill within the bounded timeout, fall back to Composite.
    LpPanelThenComposite,
}

impl HedgeExecutionMode {
    /// Every mode, in stable (proto-ordinal) order.
    pub const ALL: [HedgeExecutionMode; 4] = [
        HedgeExecutionMode::Advisory,
        HedgeExecutionMode::LpPanel,
        HedgeExecutionMode::Composite,
        HedgeExecutionMode::LpPanelThenComposite,
    ];

    /// Stable snake_case label for audit / logging.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            HedgeExecutionMode::Advisory => "advisory",
            HedgeExecutionMode::LpPanel => "lp_panel",
            HedgeExecutionMode::Composite => "composite",
            HedgeExecutionMode::LpPanelThenComposite => "lp_panel_then_composite",
        }
    }

    /// Whether this mode books nothing (the dry-run shadow posture).
    #[must_use]
    pub const fn is_advisory(self) -> bool {
        matches!(self, HedgeExecutionMode::Advisory)
    }

    /// Whether an LP-panel attempt is made before any composite fallback.
    #[must_use]
    pub const fn tries_lp_panel(self) -> bool {
        matches!(
            self,
            HedgeExecutionMode::LpPanel | HedgeExecutionMode::LpPanelThenComposite
        )
    }

    /// Whether the composite venue is an allowed fill path (as primary or fallback).
    #[must_use]
    pub const fn allows_composite(self) -> bool {
        matches!(
            self,
            HedgeExecutionMode::Composite | HedgeExecutionMode::LpPanelThenComposite
        )
    }

    /// The proto `HedgeExecutionModeEnum` ordinal
    /// (ADVISORY=0, LP_PANEL=1, COMPOSITE=2, LP_PANEL_THEN_COMPOSITE=3).
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            HedgeExecutionMode::Advisory => 0,
            HedgeExecutionMode::LpPanel => 1,
            HedgeExecutionMode::Composite => 2,
            HedgeExecutionMode::LpPanelThenComposite => 3,
        }
    }

    /// Parse from the proto ordinal; out-of-range folds to the live default
    /// (`LpPanelThenComposite`).
    #[must_use]
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => HedgeExecutionMode::Advisory,
            1 => HedgeExecutionMode::LpPanel,
            2 => HedgeExecutionMode::Composite,
            _ => HedgeExecutionMode::LpPanelThenComposite,
        }
    }
}

/// One persisted warehouse threshold bound to a scope — the configurable "100" for a
/// desk / book / instrument (§4). A plain serde mirror of the pure
/// [`WarehouseThreshold`] plus its scope binding.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HedgeThresholdDef {
    /// What the bound `scope_id` names (the id itself lives on [`ScopedThreshold`], so
    /// this inner band-config struct stays `Copy`).
    pub scope_kind: HedgeScopeKind,
    /// The budget metric.
    pub metric: HedgeMetric,
    /// The budget magnitude — the "100", in the metric's native units.
    #[serde(with = "nonfinite_f64")]
    pub cap: f64,
    /// Amber utilisation fraction (start skewing) in `[0, red]`.
    #[serde(with = "nonfinite_f64")]
    pub amber: f64,
    /// Red utilisation fraction (start hedging the overflow) in `[amber, 1]`.
    #[serde(with = "nonfinite_f64")]
    pub red: f64,
    /// Band-edge target as a fraction of `cap` (default = `amber`).
    #[serde(with = "nonfinite_f64")]
    pub target_fraction: f64,
    /// Minimum hedge clip (fixed-cost / minimum-ticket floor).
    #[serde(with = "nonfinite_f64")]
    pub min_clip: f64,
    /// Maximum single hedge clip; a larger overflow is worked. Defaults to `f64::INFINITY`
    /// ("uncapped") — persisted through [`nonfinite_f64`] so an uncapped clip survives an
    /// `identity.json` round-trip instead of serialising to `null` and bricking the next boot.
    #[serde(with = "nonfinite_f64")]
    pub max_clip: f64,
    /// Whether to ramp the hedged fraction with utilisation.
    pub ramped: bool,
    /// The ramp gain `k`.
    #[serde(with = "nonfinite_f64")]
    pub ramp_k: f64,
}

impl HedgeThresholdDef {
    /// Reconstruct the pure-crate [`WarehouseThreshold`] from this stored config, so the
    /// RAG-band / overflow-sizing logic stays owned by `celnet-hedge-routing`.
    #[must_use]
    pub fn to_threshold(&self) -> WarehouseThreshold {
        let base = WarehouseThreshold::new(self.metric.to_limit_metric(), self.cap)
            .with_bands(self.amber, self.red)
            .with_target_fraction(self.target_fraction)
            .with_clips(self.min_clip, self.max_clip);
        if self.ramped {
            base.with_ramp(self.ramp_k)
        } else {
            base
        }
    }
}

/// A warehouse threshold together with the scope id it binds to — the persisted list
/// element. The id is held here (a `String`, not on the `Copy` [`HedgeThresholdDef`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedThreshold {
    /// The desk / book / instrument id this threshold binds to.
    pub scope_id: String,
    /// The band configuration (`scope_kind` inside identifies the id's kind).
    pub def: HedgeThresholdDef,
}

impl ScopedThreshold {
    /// Whether two scoped thresholds address the same scope (kind + id) — the upsert key.
    #[must_use]
    pub fn same_scope(&self, other: &ScopedThreshold) -> bool {
        self.def.scope_kind == other.def.scope_kind && self.scope_id == other.scope_id
    }
}

/// What a hedge **policy graph** binds to — the scope of an exit-policy decision graph
/// (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §4/§5). A fill in book `B`
/// is governed by the **most-specific** applicable policy: `B`'s own [`Book`](Self::Book)
/// policy, else the nearest ancestor [`Bucket`](Self::Bucket) policy (a Bucket names a
/// risk-book subtree ROOT id and governs the whole subtree), else the [`Firm`](Self::Firm)
/// default. The Firm graph is the singleton kept on the identity store's existing
/// `hedge_policy_graph` field; Book / Bucket graphs live in a scope→graph list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgePolicyScope {
    /// The firm-wide default policy (the singleton fallback).
    Firm,
    /// A single risk book / portfolio (matches exactly that book).
    Book(String),
    /// A risk-book subtree ROOT id (matches that book and every descendant book).
    Bucket(String),
}

impl HedgePolicyScope {
    /// Stable snake_case kind label for audit / logging (id excluded).
    #[must_use]
    pub const fn kind_label(&self) -> &'static str {
        match self {
            HedgePolicyScope::Firm => "firm",
            HedgePolicyScope::Book(_) => "book",
            HedgePolicyScope::Bucket(_) => "bucket",
        }
    }

    /// The proto `HedgePolicyScopeKindEnum` ordinal (FIRM=0, BOOK=1, BUCKET=2).
    #[must_use]
    pub const fn kind_as_i32(&self) -> i32 {
        match self {
            HedgePolicyScope::Firm => 0,
            HedgePolicyScope::Book(_) => 1,
            HedgePolicyScope::Bucket(_) => 2,
        }
    }

    /// The bound scope id (empty for [`Firm`](Self::Firm)).
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            HedgePolicyScope::Firm => "",
            HedgePolicyScope::Book(id) | HedgePolicyScope::Bucket(id) => id,
        }
    }

    /// Reconstruct from the proto `(kind_ordinal, id)` pair — the wire encoding.
    /// Out-of-range kind folds to `Firm`.
    #[must_use]
    pub fn from_wire(kind: i32, id: String) -> Self {
        match kind {
            1 => HedgePolicyScope::Book(id),
            2 => HedgePolicyScope::Bucket(id),
            _ => HedgePolicyScope::Firm,
        }
    }
}

/// A hedge policy graph together with the scope it binds to — the persisted list
/// element for the Book / Bucket scoped graphs (the Firm graph is the identity store's
/// singleton `hedge_policy_graph`). Mirrors [`ScopedThreshold`] / [`ScopedLpPanel`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedHedgeGraph {
    /// The scope this graph governs (`Book(id)` or `Bucket(id)`; never `Firm` here).
    pub scope: HedgePolicyScope,
    /// The exit-policy decision graph.
    pub graph: celnet_hedge_routing::HedgeGraph,
}

impl ScopedHedgeGraph {
    /// Whether two scoped graphs address the same scope — the upsert key.
    #[must_use]
    pub fn same_scope(&self, other: &ScopedHedgeGraph) -> bool {
        self.scope == other.scope
    }
}

/// A **standing hedging LP panel** bound to a scope — the include/exclude
/// liquidity-provider selection every external exit action inherits for that
/// desk / book / instrument (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`
/// §4/§6.2). Resolves **most-specific-wins** (instrument > book > desk), the same
/// precedence style as [`ScopedThreshold`].
///
/// The include/exclude selection is the pure-crate [`HedgeLpPanel`] (whose resolver
/// [`HedgeLpPanel::effective_lps`] owns the semantics — one source of truth); this
/// server type adds only the scope binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedLpPanel {
    /// What the bound `scope_id` names (desk / book / instrument).
    pub scope_kind: HedgeScopeKind,
    /// The desk / book / instrument id this panel binds to.
    pub scope_id: String,
    /// The include/exclude selection (resolved to an effective LP set by the engine).
    #[serde(default)]
    pub panel: HedgeLpPanel,
}

impl ScopedLpPanel {
    /// Whether two scoped panels address the same scope (kind + id) — the upsert key.
    #[must_use]
    pub fn same_scope(&self, other: &ScopedLpPanel) -> bool {
        self.scope_kind == other.scope_kind && self.scope_id == other.scope_id
    }
}

/// A **scoped exit-mode binding**: whether exits in this desk / book / instrument fire
/// automatically or publish a standing suggestion a trader fires
/// (`docs/HEDGING-AND-RISK-EXIT.md` §6.5).
///
/// Resolves **most-specific-wins** (instrument > book > desk), the identical precedence
/// [`ScopedLpPanel`] uses — so a firm binds `SUGGEST` to one credit book while the rest of
/// the estate keeps firing automatically. An unbound scope is
/// [`HedgeExitMode::Auto`], which is exactly the historical behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedExitMode {
    /// What the bound `scope_id` names (desk / book / instrument).
    pub scope_kind: HedgeScopeKind,
    /// The desk / book / instrument id this mode binds to.
    pub scope_id: String,
    /// Fire automatically, or suggest and wait.
    #[serde(default)]
    pub mode: HedgeExitMode,
}

impl ScopedExitMode {
    /// Whether two bindings address the same scope (kind + id) — the upsert key.
    #[must_use]
    pub fn same_scope(&self, other: &ScopedExitMode) -> bool {
        self.scope_kind == other.scope_kind && self.scope_id == other.scope_id
    }
}

/// A **scoped hedging-model binding** — *how* this desk / book / instrument manages
/// risk: back-to-back every fill, or warehouse client flow against a DV01 budget
/// (`docs/HEDGING-AND-RISK-EXIT.md` §6.0).
///
/// Resolves **most-specific-wins** (instrument > book > desk), the identical precedence
/// [`ScopedLpPanel`] and [`ScopedExitMode`] use — so a firm runs one toxic book
/// back-to-back while the rest of the estate warehouses. An unbound scope is
/// [`HedgingModel::Custom`], which means "the scope's own authored exit-policy graph
/// governs" — i.e. **exactly** the historical behaviour.
///
/// The model is a *control that resolves to the existing primitives*, not a second
/// engine: [`HedgingModel::derived_graph`] yields an ordinary [`HedgeGraph`] built from
/// the same node/action vocabulary a trader would have authored by hand, and the
/// booking path hands it to the same evaluator. Nothing else about the six-stage spine
/// changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedHedgingModel {
    /// What the bound `scope_id` names (desk / book / instrument).
    pub scope_kind: HedgeScopeKind,
    /// The desk / book / instrument id this model binds to.
    pub scope_id: String,
    /// The posture. `Custom` (the serde default) keeps the authored graph.
    #[serde(default)]
    pub model: HedgingModel,
    /// The **DV01 warehouse budget** — the "100" this scope warehouses up to. Read
    /// **only** under [`HedgingModel::InternaliseToDv01`] (back-to-back warehouses
    /// nothing, and `Custom` takes its budget from the configured warehouse threshold).
    ///
    /// A non-positive value (the serde default, and what an operator leaves blank)
    /// means **inherit** the scope's configured [`ScopedThreshold`] exactly as today —
    /// so binding a model never silently invents a cap. When positive it overrides only
    /// the cap and the metric (`Dv01`); the amber / red / target / clip / ramp settings
    /// still come from the resolved threshold, so a desk that tuned its bands keeps them.
    #[serde(default, with = "nonfinite_f64")]
    pub dv01_budget: f64,
}

impl ScopedHedgingModel {
    /// Whether two bindings address the same scope (kind + id) — the upsert key.
    #[must_use]
    pub fn same_scope(&self, other: &ScopedHedgingModel) -> bool {
        self.scope_kind == other.scope_kind && self.scope_id == other.scope_id
    }

    /// The DV01 budget this binding actually imposes, or `None` when it imposes none
    /// (a non-`InternaliseToDv01` model, or a blank / non-positive / non-finite budget).
    /// `None` ⇒ the configured warehouse threshold is used unchanged.
    #[must_use]
    pub fn effective_budget(&self) -> Option<f64> {
        (self.model.uses_budget() && self.dv01_budget.is_finite() && self.dv01_budget > 0.0)
            .then_some(self.dv01_budget)
    }
}

/// One per-desk enable toggle in the engine config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HedgeDeskToggle {
    /// The desk id.
    pub desk: String,
    /// Whether auto-hedging is enabled for the desk (a desk absent from the list ⇒ on).
    pub enabled: bool,
}

/// The auto-hedge engine's global controls (§8.4). All fields `#[serde(default)]` so an
/// `identity.json` written before this contract loads with the safe defaults
/// (advisory-only ON, kill-switch OFF, no rate caps).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HedgeConfigDef {
    /// Global kill-switch: when true, ALL auto-hedging halts (positions warehouse).
    #[serde(default)]
    pub kill_switch: bool,
    /// How external hedge actions are executed — the "Both — config per policy" control
    /// (§6). `Advisory` is the dry-run shadow posture; the three live modes each name a
    /// concrete venue (LP panel, composite, or LP-panel-then-composite fallback).
    /// **Defaults to [`LpPanelThenComposite`](HedgeExecutionMode::LpPanelThenComposite)**
    /// — live, LP-first with a composite backstop. (The global `kill_switch`, per-desk
    /// toggles, and rate/size guards remain the safety envelope.)
    #[serde(default = "default_execution")]
    pub execution: HedgeExecutionMode,
    /// The spread (basis points) applied to the Agg Book COMPOSITE mid when a hedge fills
    /// on the composite venue (`Composite` / the `LpPanelThenComposite` fallback). Applied
    /// on the adverse side (worsening the fill vs mid), so it is the cost of the composite
    /// back-to-back. A small finite default; persisted through [`nonfinite_f64`] for
    /// robustness.
    #[serde(default = "default_composite_spread_bp", with = "nonfinite_f64")]
    pub composite_spread_bp: f64,
    /// Per-desk enable overrides (a desk absent from the list is enabled by default).
    #[serde(default)]
    pub desk_enabled: Vec<HedgeDeskToggle>,
    /// A hard ceiling on any single hedge clip (native metric units); `0` ⇒ unbounded.
    /// Persisted through [`nonfinite_f64`] so a future `∞` ("uncapped") value can never
    /// serialise to `null` and brick the next boot — finite values stay byte-identical.
    #[serde(default, with = "nonfinite_f64")]
    pub max_clip: f64,
    /// Max hedges fired per rate-limit interval; `0` ⇒ unbounded.
    #[serde(default)]
    pub max_hedges_per_interval: u32,
    /// WHEN a live external hedge runs relative to the fill. `#[serde(default)]` ⇒ an
    /// existing `identity.json` loads as `Sync`, the historical behaviour.
    #[serde(default)]
    pub dispatch: HedgeDispatch,
    /// A daily externalised-notional cap; `0` ⇒ unbounded.
    #[serde(default, with = "nonfinite_f64")]
    pub daily_external_notional_cap: f64,
    /// The standing **hedging LP panels** — per-scope include/exclude LP selection
    /// every external exit action inherits (§4/§6.2). Resolves most-specific-wins
    /// (instrument > book > desk); an empty roster ⇒ no restriction (all known LPs).
    /// An additive serde-default list, so an existing `identity.json` loads unchanged.
    #[serde(default)]
    pub lp_panels: Vec<ScopedLpPanel>,
    /// The firm's **hedge-vehicle registry** — which vehicle hedges which risk, resolved
    /// by instrument and **maturity bucket** (`docs/HEDGING-AND-RISK-EXIT.md` §6.4). This
    /// is where a `BENCHMARK` leaf finds its hedge instrument, and where an explicitly
    /// named `INSTRUMENT`/`FUTURE` vehicle finds its **DV01 per unit** — the ratio's
    /// denominator. A vehicle absent from here cannot be sized, and the booking path
    /// falls back to the self-hedge rather than trading a guessed quantity (guardrail 2).
    /// An additive serde-default list, so an existing `identity.json` loads unchanged.
    #[serde(default)]
    pub vehicles: HedgeVehicleRegistry,
    /// The scoped **suggest-vs-auto** exit modes (§6.5). Empty ⇒ `Auto` everywhere, i.e.
    /// the historical fire-on-breach behaviour. Additive serde-default.
    #[serde(default)]
    pub exit_modes: Vec<ScopedExitMode>,
    /// The scoped **hedging models** — *how* each desk / book / instrument manages risk
    /// (`docs/HEDGING-AND-RISK-EXIT.md` §6.0): back-to-back, warehouse-to-a-DV01-budget,
    /// or the desk's own authored graph. Empty ⇒ [`HedgingModel::Custom`] everywhere,
    /// i.e. every scope resolves its authored graph exactly as it always did. Additive
    /// serde-default, so an existing `identity.json` loads bit-for-bit unchanged.
    #[serde(default)]
    pub hedging_models: Vec<ScopedHedgingModel>,
    /// The minimum dealer-captured **edge** (basis points) a booked fill must clear to be
    /// internalised (warehoused) rather than handed back to the street as an advisory
    /// external back-to-back (§6): a fill dealt within this floor of the engine reference
    /// mid is a losing / marginal trade to warehouse. The price-tolerance floor the
    /// [`InternaliseVerdict`](crate::services::internalise::InternaliseVerdict) gates on.
    /// An additive serde-default field (defaults to `0.5`bp, matching
    /// [`DEFAULT_MIN_EDGE_BPS`](crate::services::internalise::DEFAULT_MIN_EDGE_BPS)), so an
    /// existing `identity.json` loads unchanged.
    #[serde(default = "default_min_edge_bps", with = "nonfinite_f64")]
    pub min_edge_bps: f64,
}

/// serde default for [`HedgeConfigDef::execution`] — the live LP-first-then-composite mode.
const fn default_execution() -> HedgeExecutionMode {
    HedgeExecutionMode::LpPanelThenComposite
}

/// serde default for [`HedgeConfigDef::composite_spread_bp`] — a small composite hedge cost.
const fn default_composite_spread_bp() -> f64 {
    0.5
}

/// serde default for [`HedgeConfigDef::min_edge_bps`] — kept in sync with
/// [`crate::services::internalise::DEFAULT_MIN_EDGE_BPS`] (0.5bp).
const fn default_min_edge_bps() -> f64 {
    0.5
}

impl Default for HedgeConfigDef {
    fn default() -> Self {
        Self {
            kill_switch: false,
            execution: default_execution(),
            composite_spread_bp: default_composite_spread_bp(),
            desk_enabled: Vec::new(),
            max_clip: 0.0,
            max_hedges_per_interval: 0,
            dispatch: HedgeDispatch::Sync,
            daily_external_notional_cap: 0.0,
            lp_panels: Vec::new(),
            vehicles: HedgeVehicleRegistry::default(),
            exit_modes: Vec::new(),
            hedging_models: Vec::new(),
            min_edge_bps: default_min_edge_bps(),
        }
    }
}

impl HedgeConfigDef {
    /// Whether the engine may act (trade / book) for `desk` right now: the global
    /// kill-switch is off **and** the desk is not explicitly disabled. (Advisory-only is
    /// a separate gate applied to *external* actions only.)
    #[must_use]
    pub fn desk_active(&self, desk: &str) -> bool {
        if self.kill_switch {
            return false;
        }
        self.desk_enabled
            .iter()
            .find(|t| t.desk == desk)
            .is_none_or(|t| t.enabled)
    }

    /// Resolve the standing hedging LP panel for a `(desk, book, instrument)` risk
    /// state, **most-specific-wins** (instrument > book > desk). Returns `None` when no
    /// panel is configured for any of the three scopes — the caller then falls back to
    /// the per-rule `RfqOut` include list, else the full known-LP set (§6.2).
    #[must_use]
    pub fn resolve_lp_panel(
        &self,
        desk: &str,
        book: &str,
        instrument: &str,
    ) -> Option<&HedgeLpPanel> {
        // Probe most-specific → least-specific; the first hit wins.
        let by = |kind: HedgeScopeKind, id: &str| {
            self.lp_panels
                .iter()
                .find(move |p| p.scope_kind == kind && p.scope_id == id)
        };
        by(HedgeScopeKind::Instrument, instrument)
            .or_else(|| by(HedgeScopeKind::Book, book))
            .or_else(|| by(HedgeScopeKind::Desk, desk))
            .map(|p| &p.panel)
    }

    /// Resolve the **exit mode** for a `(desk, book, instrument)` risk state,
    /// **most-specific-wins** (instrument > book > desk) — the identical precedence
    /// [`Self::resolve_lp_panel`] uses. An unbound scope is
    /// [`HedgeExitMode::Auto`], so a firm that has configured nothing keeps firing
    /// automatically exactly as it always did.
    #[must_use]
    pub fn resolve_exit_mode(&self, desk: &str, book: &str, instrument: &str) -> HedgeExitMode {
        let by = |kind: HedgeScopeKind, id: &str| {
            self.exit_modes
                .iter()
                .find(move |m| m.scope_kind == kind && m.scope_id == id)
        };
        by(HedgeScopeKind::Instrument, instrument)
            .or_else(|| by(HedgeScopeKind::Book, book))
            .or_else(|| by(HedgeScopeKind::Desk, desk))
            .map_or(HedgeExitMode::Auto, |m| m.mode)
    }

    /// Upsert one exit-mode binding by scope (kind + id).
    ///
    /// An explicit [`HedgeExitMode::Auto`] binding is **kept**, not treated as a delete: it
    /// is a real override that lets a book fire automatically underneath a desk-wide
    /// `Suggest`. (Removal is [`Self::remove_exit_mode`] — the operator's explicit "clear
    /// this override" gesture, which restores inheritance from the wider scope.)
    pub fn upsert_exit_mode(&mut self, entry: ScopedExitMode) {
        self.exit_modes.retain(|m| !m.same_scope(&entry));
        self.exit_modes.push(entry);
    }

    /// Clear one scope's exit-mode override, so it inherits from the next-wider scope again.
    pub fn remove_exit_mode(&mut self, scope_kind: HedgeScopeKind, scope_id: &str) {
        self.exit_modes
            .retain(|m| !(m.scope_kind == scope_kind && m.scope_id == scope_id));
    }

    /// Resolve the **hedging-model binding** governing a `(desk, book, instrument)` risk
    /// state, **most-specific-wins** (instrument > book > desk) — the identical precedence
    /// [`Self::resolve_lp_panel`] and [`Self::resolve_exit_mode`] use.
    ///
    /// `None` ⇒ no model is bound at any scope, which is
    /// [`HedgingModel::Custom`]: the scope's own authored exit-policy graph governs and
    /// nothing about the decision changes. This is why an `identity.json` written before
    /// hedging models existed behaves EXACTLY as it did.
    #[must_use]
    pub fn resolve_hedging_model(
        &self,
        desk: &str,
        book: &str,
        instrument: &str,
    ) -> Option<&ScopedHedgingModel> {
        let by = |kind: HedgeScopeKind, id: &str| {
            self.hedging_models
                .iter()
                .find(move |m| m.scope_kind == kind && m.scope_id == id)
        };
        by(HedgeScopeKind::Instrument, instrument)
            .or_else(|| by(HedgeScopeKind::Book, book))
            .or_else(|| by(HedgeScopeKind::Desk, desk))
    }

    /// Upsert one hedging-model binding by scope (kind + id).
    ///
    /// An explicit [`HedgingModel::Custom`] binding is **kept**, not treated as a delete:
    /// it is a real override that lets one book keep its bespoke authored graph underneath
    /// a desk-wide `BackToBack` — the escape hatch, expressible at any scope. (Removal is
    /// [`Self::remove_hedging_model`], which restores inheritance from the wider scope.)
    pub fn upsert_hedging_model(&mut self, entry: ScopedHedgingModel) {
        self.hedging_models.retain(|m| !m.same_scope(&entry));
        self.hedging_models.push(entry);
    }

    /// Clear one scope's hedging-model override, so it inherits from the next-wider scope
    /// again (and, with nothing bound anywhere, returns to the authored graph).
    pub fn remove_hedging_model(&mut self, scope_kind: HedgeScopeKind, scope_id: &str) {
        self.hedging_models
            .retain(|m| !(m.scope_kind == scope_kind && m.scope_id == scope_id));
    }

    /// The hedge-instrument ids the vehicle registry knows — the set
    /// [`HedgeGraph::validate`](celnet_hedge_routing::HedgeGraph::validate) checks a
    /// leaf's explicitly named vehicle against, so a rule can never name a vehicle whose
    /// DV01 per unit the firm has not configured.
    #[must_use]
    pub fn known_vehicles(&self) -> std::collections::BTreeSet<String> {
        self.vehicles
            .rules
            .iter()
            .filter(|r| r.validate().is_ok())
            .map(|r| r.hedge_instrument_id.clone())
            .collect()
    }

    /// Upsert one vehicle-registry row by id; an empty `hedge_instrument_id` **removes**
    /// the row (the operator's delete gesture).
    pub fn upsert_vehicle(&mut self, entry: HedgeVehicleRule) {
        self.vehicles.rules.retain(|r| r.id != entry.id);
        if !entry.hedge_instrument_id.trim().is_empty() {
            self.vehicles.rules.push(entry);
        }
    }

    /// Upsert one LP panel by scope (kind + id). A panel that is **unrestricted** (empty
    /// include *and* exclude) **removes** the matching entry (the operator's "clear the
    /// panel" gesture); otherwise it inserts / replaces the same-scope entry.
    pub fn upsert_lp_panel(&mut self, entry: ScopedLpPanel) {
        self.lp_panels.retain(|p| !p.same_scope(&entry));
        if !entry.panel.is_unrestricted() {
            self.lp_panels.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_hedge_routing::RagStatus;

    #[test]
    fn threshold_reconstructs_pure_bands() {
        let def = HedgeThresholdDef {
            scope_kind: HedgeScopeKind::Book,
            metric: HedgeMetric::Dv01,
            cap: 100_000.0,
            amber: 0.8,
            red: 0.9,
            target_fraction: 0.8,
            min_clip: 0.0,
            max_clip: f64::INFINITY,
            ramped: false,
            ramp_k: 1.0,
        };
        let t = def.to_threshold();
        assert_eq!(t.classify(50_000.0), RagStatus::Green);
        assert_eq!(t.classify(95_000.0), RagStatus::Red);
        assert_eq!(t.overflow(95_000.0), 15_000.0); // to the amber (80k) edge
    }

    #[test]
    fn ramped_flag_round_trips_into_threshold() {
        let def = HedgeThresholdDef {
            scope_kind: HedgeScopeKind::Desk,
            metric: HedgeMetric::NetNotional,
            cap: 100_000.0,
            amber: 0.8,
            red: 0.9,
            target_fraction: 0.0, // hedge to flat so overflow == |risk|
            min_clip: 0.0,
            max_clip: f64::INFINITY,
            ramped: true,
            ramp_k: 2.0,
        };
        let t = def.to_threshold();
        // util 1.1 -> fraction clamp(2*0.1,0,1)=0.2, overflow 110k -> 22k
        let s = t.sizing(110_000.0);
        assert!(s.breached);
        assert!((s.size - 22_000.0).abs() < 1e-6, "got {}", s.size);
    }

    #[test]
    fn metric_and_scope_ordinals_round_trip() {
        for m in HedgeMetric::ALL {
            assert_eq!(HedgeMetric::from_i32(m.as_i32()), m);
        }
        for k in HedgeScopeKind::ALL {
            assert_eq!(HedgeScopeKind::from_i32(k.as_i32()), k);
        }
    }

    #[test]
    fn config_default_is_live_lp_then_composite_kill_switch_off() {
        let c = HedgeConfigDef::default();
        assert_eq!(
            c.execution,
            HedgeExecutionMode::LpPanelThenComposite,
            "execution defaults to the live LP-first-then-composite mode"
        );
        assert!(!c.execution.is_advisory());
        assert_eq!(c.composite_spread_bp, 0.5);
        assert!(!c.kill_switch);
        assert!(c.desk_active("RATES"), "no per-desk override ⇒ active");
    }

    #[test]
    fn execution_mode_ordinals_round_trip() {
        for m in HedgeExecutionMode::ALL {
            assert_eq!(HedgeExecutionMode::from_i32(m.as_i32()), m);
        }
    }

    #[test]
    fn policy_scope_wire_round_trips() {
        for s in [
            HedgePolicyScope::Firm,
            HedgePolicyScope::Book("RATES-EUR".into()),
            HedgePolicyScope::Bucket("RATES".into()),
        ] {
            let back = HedgePolicyScope::from_wire(s.kind_as_i32(), s.id().to_owned());
            assert_eq!(back, s);
        }
    }

    #[test]
    fn kill_switch_and_desk_toggle_gate_activity() {
        let c = HedgeConfigDef {
            kill_switch: false,
            desk_enabled: vec![HedgeDeskToggle {
                desk: "FX".into(),
                enabled: false,
            }],
            ..HedgeConfigDef::default()
        };
        assert!(!c.desk_active("FX"), "explicitly disabled desk is inactive");
        assert!(c.desk_active("RATES"), "other desks stay active");

        let killed = HedgeConfigDef {
            kill_switch: true,
            ..HedgeConfigDef::default()
        };
        assert!(!killed.desk_active("RATES"), "kill-switch halts every desk");
    }

    fn panel(
        scope_kind: HedgeScopeKind,
        id: &str,
        include: &[&str],
        exclude: &[&str],
    ) -> ScopedLpPanel {
        ScopedLpPanel {
            scope_kind,
            scope_id: id.into(),
            panel: HedgeLpPanel {
                include: include.iter().map(|s| (*s).to_string()).collect(),
                exclude: exclude.iter().map(|s| (*s).to_string()).collect(),
            },
        }
    }

    #[test]
    fn lp_panel_resolves_most_specific_wins() {
        let cfg = HedgeConfigDef {
            lp_panels: vec![
                panel(HedgeScopeKind::Desk, "RATES", &[], &["LP-1"]),
                panel(HedgeScopeKind::Book, "RATES-EUR", &["LP-2", "LP-3"], &[]),
                panel(HedgeScopeKind::Instrument, "EURUSD", &[], &["LP-4"]),
            ],
            ..HedgeConfigDef::default()
        };
        // Instrument beats book beats desk.
        assert_eq!(
            cfg.resolve_lp_panel("RATES", "RATES-EUR", "EURUSD")
                .unwrap()
                .exclude,
            vec!["LP-4"]
        );
        // No instrument panel → book wins.
        assert_eq!(
            cfg.resolve_lp_panel("RATES", "RATES-EUR", "GBPUSD")
                .unwrap()
                .include,
            vec!["LP-2", "LP-3"]
        );
        // No instrument/book panel → desk wins.
        assert_eq!(
            cfg.resolve_lp_panel("RATES", "RATES-USD", "USDJPY")
                .unwrap()
                .exclude,
            vec!["LP-1"]
        );
        // No panel at any scope → None (caller falls back).
        assert!(cfg.resolve_lp_panel("FX", "FX-G10", "AUDUSD").is_none());
    }

    #[test]
    fn upsert_lp_panel_replaces_by_scope_and_clears_on_unrestricted() {
        let mut cfg = HedgeConfigDef::default();
        cfg.upsert_lp_panel(panel(HedgeScopeKind::Book, "RATES-EUR", &["LP-1"], &[]));
        assert_eq!(cfg.lp_panels.len(), 1);
        // Same scope replaces (not appends).
        cfg.upsert_lp_panel(panel(HedgeScopeKind::Book, "RATES-EUR", &[], &["LP-2"]));
        assert_eq!(cfg.lp_panels.len(), 1);
        assert_eq!(cfg.lp_panels[0].panel.exclude, vec!["LP-2"]);
        // A different scope appends.
        cfg.upsert_lp_panel(panel(HedgeScopeKind::Desk, "RATES", &["LP-3"], &[]));
        assert_eq!(cfg.lp_panels.len(), 2);
        // An unrestricted (empty/empty) upsert clears the same-scope entry.
        cfg.upsert_lp_panel(panel(HedgeScopeKind::Book, "RATES-EUR", &[], &[]));
        assert_eq!(cfg.lp_panels.len(), 1);
        assert!(cfg.resolve_lp_panel("RATES", "RATES-EUR", "x").is_some()); // desk panel still resolves
    }

    /// A threshold with an **uncapped** (`f64::INFINITY`) `max_clip` — exactly what
    /// [`IdentityStore::ensure_seed_hedge_policy`](crate::config::identity::IdentityStore) seeds
    /// on first boot and what the proto→def mapping produces for `max_clip <= 0` — must survive
    /// the `identity.json` persistence round-trip (`serde_json` String out and back), NOT
    /// serialise to `null` and fail to reload. Regression for the boot-crash
    /// `invalid type: null, expected f64`.
    #[test]
    fn infinite_threshold_fields_survive_json_round_trip() {
        let def = HedgeThresholdDef {
            scope_kind: HedgeScopeKind::Book,
            metric: HedgeMetric::Dv01,
            // Every non-finite-capable field set to a non-finite sentinel at once, so a
            // missed field would surface here rather than only moving the crash.
            cap: f64::INFINITY,
            amber: 0.8,
            red: 0.9,
            target_fraction: f64::NEG_INFINITY,
            min_clip: f64::NAN,
            max_clip: f64::INFINITY,
            ramped: true,
            ramp_k: f64::INFINITY,
        };
        // Nest inside the actual persisted element (ScopedThreshold), mirroring identity.json.
        let scoped = ScopedThreshold {
            scope_id: "RATES-EUR".into(),
            def,
        };
        let json = serde_json::to_string(&scoped).expect("serialize must not fail");
        // Non-finite values round-trip as string sentinels, never as `null`.
        assert!(
            !json.contains("null"),
            "no field may serialise to null (that is the boot-crash trigger): {json}"
        );
        assert!(
            json.contains("\"max_clip\":\"inf\""),
            "max_clip → sentinel: {json}"
        );

        let back: ScopedThreshold = serde_json::from_str(&json).expect("must reload, not crash");
        assert!(back.def.max_clip.is_infinite() && back.def.max_clip > 0.0);
        assert!(back.def.cap.is_infinite() && back.def.cap > 0.0);
        assert_eq!(back.def.target_fraction, f64::NEG_INFINITY);
        assert!(back.def.min_clip.is_nan());
        assert!(back.def.ramp_k.is_infinite() && back.def.ramp_k > 0.0);
        // Finite fields are unchanged.
        assert_eq!(back.def.amber, 0.8);
        assert_eq!(back.def.red, 0.9);
    }

    /// Finite values are **byte-unchanged**: they serialise as bare JSON numbers (identical to a
    /// plain `f64`), so an existing `identity.json` written before this helper loads unchanged.
    #[test]
    fn finite_threshold_json_is_byte_unchanged() {
        let scoped = ScopedThreshold {
            scope_id: "RATES-EUR".into(),
            def: HedgeThresholdDef {
                scope_kind: HedgeScopeKind::Book,
                metric: HedgeMetric::Dv01,
                cap: 100_000.0,
                amber: 0.8,
                red: 0.9,
                target_fraction: 0.8,
                min_clip: 0.0,
                max_clip: 25_000.0,
                ramped: false,
                ramp_k: 1.0,
            },
        };
        let json = serde_json::to_string(&scoped).unwrap();
        // Bare numbers, not quoted sentinels.
        assert!(json.contains("\"cap\":100000.0"), "{json}");
        assert!(json.contains("\"max_clip\":25000.0"), "{json}");
        // The finite clip is a bare number, never a quoted sentinel string.
        assert!(!json.contains("\"max_clip\":\""), "{json}");
        let back: ScopedThreshold = serde_json::from_str(&json).unwrap();
        assert_eq!(back, scoped, "finite config round-trips identically");
    }

    /// An already-corrupted `identity.json` — one written by the *old* serializer, which emitted
    /// `null` for the uncapped `f64::INFINITY` `max_clip` — must **heal** to `f64::INFINITY` on
    /// load instead of failing with `invalid type: null, expected f64`.
    #[test]
    fn legacy_null_max_clip_heals_to_infinity() {
        let legacy = r#"{
            "scope_id": "RATES-EUR",
            "def": {
                "scope_kind": "Book",
                "metric": "Dv01",
                "cap": 100000.0,
                "amber": 0.8,
                "red": 0.9,
                "target_fraction": 0.8,
                "min_clip": 0.0,
                "max_clip": null,
                "ramped": false,
                "ramp_k": 1.0
            }
        }"#;
        let back: ScopedThreshold =
            serde_json::from_str(legacy).expect("legacy null must heal, not crash");
        assert!(
            back.def.max_clip.is_infinite() && back.def.max_clip > 0.0,
            "legacy null max_clip heals to +INFINITY"
        );
        // The healed def still reconstructs the pure engine threshold uncapped.
        let t = back.def.to_threshold();
        assert_eq!(t.classify(50_000.0), RagStatus::Green);
    }

    // --- hedging models (how risk is managed) -------------------------------

    fn model(kind: HedgeScopeKind, id: &str, m: HedgingModel, budget: f64) -> ScopedHedgingModel {
        ScopedHedgingModel {
            scope_kind: kind,
            scope_id: id.into(),
            model: m,
            dv01_budget: budget,
        }
    }

    /// The no-silent-change contract: an `identity.json` written before hedging models
    /// existed carries no `hedging_models` key, must load, and must resolve NOTHING —
    /// which is `Custom`, i.e. the authored graph governs exactly as it always did.
    #[test]
    fn a_config_with_no_model_resolves_nothing_and_is_unchanged() {
        let cfg = HedgeConfigDef::default();
        assert!(cfg.hedging_models.is_empty());
        assert!(
            cfg.resolve_hedging_model("RATES", "RATES-EUR", "BOND")
                .is_none(),
            "an unbound scope resolves NO binding ⇒ Custom ⇒ today's behaviour"
        );

        // A legacy persisted config (no key at all) reloads with an empty roster.
        let legacy = r#"{"kill_switch":false,"execution":"LpPanelThenComposite"}"#;
        let back: HedgeConfigDef = serde_json::from_str(legacy).expect("legacy config must load");
        assert!(back.hedging_models.is_empty());
        assert!(back.resolve_hedging_model("RATES", "wh", "BOND").is_none());
    }

    #[test]
    fn hedging_model_resolves_most_specific_wins() {
        let cfg = HedgeConfigDef {
            hedging_models: vec![
                model(
                    HedgeScopeKind::Desk,
                    "RATES",
                    HedgingModel::InternaliseToDv01,
                    250_000.0,
                ),
                model(
                    HedgeScopeKind::Book,
                    "CREDIT",
                    HedgingModel::BackToBack,
                    0.0,
                ),
                model(
                    HedgeScopeKind::Instrument,
                    "XS-CORP-9Y",
                    HedgingModel::Custom,
                    0.0,
                ),
            ],
            ..HedgeConfigDef::default()
        };
        // Instrument beats book beats desk — and the instrument binding here is the
        // ESCAPE HATCH: one security keeps its bespoke authored graph under a
        // back-to-back book under a warehousing desk.
        assert_eq!(
            cfg.resolve_hedging_model("RATES", "CREDIT", "XS-CORP-9Y")
                .unwrap()
                .model,
            HedgingModel::Custom
        );
        assert_eq!(
            cfg.resolve_hedging_model("RATES", "CREDIT", "US10Y")
                .unwrap()
                .model,
            HedgingModel::BackToBack
        );
        let desk = cfg
            .resolve_hedging_model("RATES", "RATES-EUR", "US10Y")
            .unwrap();
        assert_eq!(desk.model, HedgingModel::InternaliseToDv01);
        assert_eq!(desk.effective_budget(), Some(250_000.0));
        // A scope bound nowhere still resolves nothing.
        assert!(
            cfg.resolve_hedging_model("FX", "FX-G10", "EURUSD")
                .is_none()
        );
    }

    /// A budget is imposed ONLY by an internalisation binding with a real, positive,
    /// finite number. Everything else inherits the configured warehouse threshold, so a
    /// model can never silently invent a cap (guardrail 2).
    #[test]
    fn only_a_positive_internalisation_budget_overrides_the_threshold() {
        assert_eq!(
            model(
                HedgeScopeKind::Book,
                "b",
                HedgingModel::InternaliseToDv01,
                100.0
            )
            .effective_budget(),
            Some(100.0)
        );
        // Blank / zero / negative ⇒ inherit.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                model(
                    HedgeScopeKind::Book,
                    "b",
                    HedgingModel::InternaliseToDv01,
                    bad
                )
                .effective_budget(),
                None,
                "budget {bad} must not override the configured threshold"
            );
        }
        // Back-to-back warehouses nothing, so a stray budget is ignored, not applied.
        assert_eq!(
            model(HedgeScopeKind::Book, "b", HedgingModel::BackToBack, 500.0).effective_budget(),
            None
        );
        assert_eq!(
            model(HedgeScopeKind::Book, "b", HedgingModel::Custom, 500.0).effective_budget(),
            None
        );
    }

    #[test]
    fn upsert_replaces_by_scope_and_keeps_an_explicit_custom_override() {
        let mut cfg = HedgeConfigDef::default();
        cfg.upsert_hedging_model(model(
            HedgeScopeKind::Book,
            "CREDIT",
            HedgingModel::BackToBack,
            0.0,
        ));
        assert_eq!(cfg.hedging_models.len(), 1);
        // Same scope replaces, never appends.
        cfg.upsert_hedging_model(model(
            HedgeScopeKind::Book,
            "CREDIT",
            HedgingModel::InternaliseToDv01,
            80_000.0,
        ));
        assert_eq!(cfg.hedging_models.len(), 1);
        assert_eq!(
            cfg.hedging_models[0].effective_budget(),
            Some(80_000.0),
            "the replacement's budget wins"
        );
        // An explicit `Custom` is a real override, not a delete — it is how a book opts
        // OUT of a wider posture and back into its own authored graph.
        cfg.upsert_hedging_model(model(
            HedgeScopeKind::Desk,
            "RATES",
            HedgingModel::BackToBack,
            0.0,
        ));
        cfg.upsert_hedging_model(model(
            HedgeScopeKind::Book,
            "WASH",
            HedgingModel::Custom,
            0.0,
        ));
        assert_eq!(cfg.hedging_models.len(), 3);
        assert_eq!(
            cfg.resolve_hedging_model("RATES", "WASH", "BOND")
                .unwrap()
                .model,
            HedgingModel::Custom,
            "the book-scoped Custom overrides the desk-wide BackToBack"
        );
        // Removal restores inheritance from the wider scope.
        cfg.remove_hedging_model(HedgeScopeKind::Book, "WASH");
        assert_eq!(
            cfg.resolve_hedging_model("RATES", "WASH", "BOND")
                .unwrap()
                .model,
            HedgingModel::BackToBack
        );
    }

    /// The binding survives an `identity.json` round-trip, including a non-finite budget
    /// (which must never serialise to `null` and brick the next boot).
    #[test]
    fn hedging_model_bindings_survive_a_json_round_trip() {
        let cfg = HedgeConfigDef {
            hedging_models: vec![
                model(
                    HedgeScopeKind::Book,
                    "CREDIT",
                    HedgingModel::BackToBack,
                    0.0,
                ),
                model(
                    HedgeScopeKind::Desk,
                    "RATES",
                    HedgingModel::InternaliseToDv01,
                    f64::INFINITY,
                ),
            ],
            ..HedgeConfigDef::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("null"), "{json}");
        let back: HedgeConfigDef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
        // An `∞` budget is not a real budget — it still inherits the threshold.
        assert_eq!(back.hedging_models[1].effective_budget(), None);
    }

    /// The engine config round-trips with an `∞` clip too (defensive: the field uses `0` for
    /// unbounded today, but a future `∞` must not brick boot).
    #[test]
    fn hedge_config_infinite_clip_round_trips() {
        let cfg = HedgeConfigDef {
            max_clip: f64::INFINITY,
            ..HedgeConfigDef::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("null"), "{json}");
        let back: HedgeConfigDef = serde_json::from_str(&json).unwrap();
        assert!(back.max_clip.is_infinite() && back.max_clip > 0.0);
        // Default (finite) config is byte-unchanged and reloads identically.
        let default_json = serde_json::to_string(&HedgeConfigDef::default()).unwrap();
        assert!(default_json.contains("\"max_clip\":0.0"), "{default_json}");
        let default_back: HedgeConfigDef = serde_json::from_str(&default_json).unwrap();
        assert_eq!(default_back, HedgeConfigDef::default());
    }
}
