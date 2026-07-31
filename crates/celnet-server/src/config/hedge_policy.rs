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

use celnet_hedge_routing::{HedgeLpPanel, LimitMetric, WarehouseThreshold};
use serde::{Deserialize, Serialize};

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
}

impl HedgeMetric {
    /// Every metric, in stable order (the canonical iteration set).
    pub const ALL: [HedgeMetric; 4] = [
        HedgeMetric::Dv01,
        HedgeMetric::NetNotional,
        HedgeMetric::NetDelta,
        HedgeMetric::NetVega,
    ];

    /// Stable snake_case label for audit / logging.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            HedgeMetric::Dv01 => "dv01",
            HedgeMetric::NetNotional => "net_notional",
            HedgeMetric::NetDelta => "net_delta",
            HedgeMetric::NetVega => "net_vega",
        }
    }

    /// Map to the pure-crate [`LimitMetric`] used to reconstruct a [`WarehouseThreshold`].
    /// `NetNotional`/`NetDelta` both map to the additive net base-currency delta metric
    /// (there is no distinct "net notional" `LimitMetric`); the RAG band is metric-agnostic.
    #[must_use]
    pub fn to_limit_metric(self) -> LimitMetric {
        match self {
            HedgeMetric::Dv01 => LimitMetric::Dv01,
            HedgeMetric::NetNotional | HedgeMetric::NetDelta => LimitMetric::Delta,
            HedgeMetric::NetVega => LimitMetric::Vega,
        }
    }

    /// The proto `HedgeMetricEnum` ordinal (DV01=0, NET_NOTIONAL=1, NET_DELTA=2, NET_VEGA=3).
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            HedgeMetric::Dv01 => 0,
            HedgeMetric::NetNotional => 1,
            HedgeMetric::NetDelta => 2,
            HedgeMetric::NetVega => 3,
        }
    }

    /// Parse from the proto `HedgeMetricEnum` ordinal; out-of-range folds to `Dv01`.
    #[must_use]
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => HedgeMetric::NetNotional,
            2 => HedgeMetric::NetDelta,
            3 => HedgeMetric::NetVega,
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
    pub cap: f64,
    /// Amber utilisation fraction (start skewing) in `[0, red]`.
    pub amber: f64,
    /// Red utilisation fraction (start hedging the overflow) in `[amber, 1]`.
    pub red: f64,
    /// Band-edge target as a fraction of `cap` (default = `amber`).
    pub target_fraction: f64,
    /// Minimum hedge clip (fixed-cost / minimum-ticket floor).
    pub min_clip: f64,
    /// Maximum single hedge clip; a larger overflow is worked.
    pub max_clip: f64,
    /// Whether to ramp the hedged fraction with utilisation.
    pub ramped: bool,
    /// The ramp gain `k`.
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
    /// Advisory-only: compute + emit intents / provenance but never trade externally.
    /// **Defaults ON** — a desk must explicitly disarm advisory to allow live external
    /// orders (the mandatory shadow-run stance, §8.4).
    #[serde(default = "default_true")]
    pub advisory_only: bool,
    /// Per-desk enable overrides (a desk absent from the list is enabled by default).
    #[serde(default)]
    pub desk_enabled: Vec<HedgeDeskToggle>,
    /// A hard ceiling on any single hedge clip (native metric units); `0` ⇒ unbounded.
    #[serde(default)]
    pub max_clip: f64,
    /// Max hedges fired per rate-limit interval; `0` ⇒ unbounded.
    #[serde(default)]
    pub max_hedges_per_interval: u32,
    /// A daily externalised-notional cap; `0` ⇒ unbounded.
    #[serde(default)]
    pub daily_external_notional_cap: f64,
    /// The standing **hedging LP panels** — per-scope include/exclude LP selection
    /// every external exit action inherits (§4/§6.2). Resolves most-specific-wins
    /// (instrument > book > desk); an empty roster ⇒ no restriction (all known LPs).
    /// An additive serde-default list, so an existing `identity.json` loads unchanged.
    #[serde(default)]
    pub lp_panels: Vec<ScopedLpPanel>,
}

/// serde default for [`HedgeConfigDef::advisory_only`].
const fn default_true() -> bool {
    true
}

impl Default for HedgeConfigDef {
    fn default() -> Self {
        Self {
            kill_switch: false,
            advisory_only: true,
            desk_enabled: Vec::new(),
            max_clip: 0.0,
            max_hedges_per_interval: 0,
            daily_external_notional_cap: 0.0,
            lp_panels: Vec::new(),
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
    fn config_default_is_advisory_only_kill_switch_off() {
        let c = HedgeConfigDef::default();
        assert!(c.advisory_only, "advisory-only must default ON");
        assert!(!c.kill_switch);
        assert!(c.desk_active("RATES"), "no per-desk override ⇒ active");
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
}
