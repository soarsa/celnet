//! The **hedging model** — *how* a desk manages risk, as a first-class choice.
//!
//! A desk has a posture before it has a rule set. Two postures cover almost every
//! real mandate, and they are economically opposite:
//!
//! - **Back-to-back.** Every fill is immediately hedged out on the street. Nothing is
//!   warehoused, so there is no DV01 budget to breach and no opposing flow to net
//!   against; the desk earns the spread it quoted and nothing else. The classic
//!   posture for an agency-like or risk-averse mandate, and for a book whose flow is
//!   too toxic or too illiquid to hold.
//! - **Internalise to a DV01 budget.** Client flow is warehoused against a budget,
//!   opposing flow nets off inside the book for free, and only the *overflow* above
//!   the band edge is paid away to the street. This is the dealer's actual edge —
//!   and it is Celnet's historical default behaviour.
//!
//! # This is a control, not a second engine
//!
//! Both postures were **already expressible** in the exit-policy graph
//! (`docs/HEDGING-AND-RISK-EXIT.md` §6). Back-to-back is a single catch-all leaf
//! `SubmitMarketOrder { size: Full }`; internalisation is the two-node
//! warehouse-vs-overflow graph. What was missing was not the capability — it was a
//! *control*: expressing either one meant hand-authoring a decision graph, which is
//! not a "tune how risk is managed" knob a trader will ever reach for.
//!
//! So [`HedgingModel`] **resolves to** those existing primitives via
//! [`HedgingModel::derived_graph`] rather than introducing a parallel decision path.
//! There is exactly one engine, one [`HedgeGraph`](crate::HedgeGraph) evaluator, one
//! sizing routine and one provenance ring; picking a model only decides *which graph*
//! is handed to them. Everything downstream — the RAG band, the vehicle, the exit
//! mode, the LP panel, the internalise-vs-shed split, the offsetting leg — is
//! untouched and behaves identically.
//!
//! # The escape hatch stays open
//!
//! [`HedgingModel::Custom`] is the **default**, and it means "the desk's own authored
//! graph governs" — i.e. exactly today's behaviour. A firm that has configured no
//! model anywhere is bit-for-bit unchanged, and a desk that wants a bespoke graph
//! (per-counterparty back-to-back, toxicity rules, escalation leaves, vehicle
//! hedges) simply leaves its scope on `Custom` and authors it as before. A model is
//! a *shortcut to* the graph vocabulary, never a replacement for it.

use crate::graph::{ExecStyle, ExitAction, HedgeGraph, HedgeNode, HedgeSize};
use crate::vehicle::HedgeVehicle;
use celnet_risk_routing::{RouteOp, RouteValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// How a scope manages risk — the trader-facing posture that resolves to an
/// exit-policy graph.
///
/// [`Custom`](Self::Custom) is the `Default` so an unbound scope, and a firm that has
/// configured nothing, keeps today's behaviour exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgingModel {
    /// **The desk's own authored exit-policy graph governs.** The default, and the
    /// escape hatch: no derivation happens, the scope resolves its `Book` / `Bucket` /
    /// `Firm` graph exactly as it always did.
    #[default]
    Custom,
    /// **Back-to-back.** Every fill is hedged straight out on the street; nothing is
    /// warehoused. Resolves to a single catch-all
    /// `SubmitMarketOrder { size: Full, style: Immediate }` leaf.
    BackToBack,
    /// **Warehouse to a DV01 budget.** Hold client flow against the budget, let
    /// opposing flow net off, shed only the overflow above the band edge. Resolves to
    /// the two-node warehouse-vs-overflow graph (`breached == false ? Warehouse :
    /// SubmitMarketOrder { size: Overflow }`).
    InternaliseToDv01,
}

impl HedgingModel {
    /// Every model, in stable (proto-ordinal) order — the canonical iteration set.
    pub const ALL: [HedgingModel; 3] = [
        HedgingModel::Custom,
        HedgingModel::BackToBack,
        HedgingModel::InternaliseToDv01,
    ];

    /// A short, stable snake_case label for provenance / logging / UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            HedgingModel::Custom => "custom",
            HedgingModel::BackToBack => "back_to_back",
            HedgingModel::InternaliseToDv01 => "internalise_to_dv01",
        }
    }

    /// Whether this model **derives** its policy graph (so the scope's authored graph
    /// is bypassed). `false` for [`Custom`](Self::Custom) — the single predicate the
    /// booking path branches on.
    #[must_use]
    pub const fn derives_policy(self) -> bool {
        !matches!(self, HedgingModel::Custom)
    }

    /// Whether this model reads a **DV01 budget** (the warehouse cap). Only
    /// [`InternaliseToDv01`](Self::InternaliseToDv01) warehouses, so only it has a
    /// budget to size; back-to-back holds nothing and `Custom` takes its budget from
    /// the configured warehouse threshold exactly as before.
    #[must_use]
    pub const fn uses_budget(self) -> bool {
        matches!(self, HedgingModel::InternaliseToDv01)
    }

    /// The proto `HedgingModelEnum` ordinal (CUSTOM=0, BACK_TO_BACK=1,
    /// INTERNALISE_TO_DV01=2).
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            HedgingModel::Custom => 0,
            HedgingModel::BackToBack => 1,
            HedgingModel::InternaliseToDv01 => 2,
        }
    }

    /// Parse from the proto ordinal; out-of-range folds to
    /// [`Custom`](Self::Custom) — the no-derivation default, so an unknown ordinal can
    /// never silently change how a book hedges.
    #[must_use]
    pub const fn from_i32(v: i32) -> Self {
        match v {
            1 => HedgingModel::BackToBack,
            2 => HedgingModel::InternaliseToDv01,
            _ => HedgingModel::Custom,
        }
    }

    /// **The resolution to existing primitives.** The exit-policy graph this model
    /// stands for, or `None` for [`Custom`](Self::Custom) (whose graph is the desk's
    /// own authored one).
    ///
    /// This is the whole mechanism: a model never gets its own decision path, it only
    /// names a graph built from the same [`HedgeNode`] / [`ExitAction`] vocabulary a
    /// trader would have authored by hand. Every leaf carries
    /// [`HedgeVehicle::SelfInstrument`], the historical default (the same security
    /// sold back, DV01 ratio identically 1).
    ///
    /// | Model | Graph |
    /// | --- | --- |
    /// | [`Custom`](Self::Custom) | `None` — the authored graph governs |
    /// | [`BackToBack`](Self::BackToBack) | one leaf: `SUBMIT_MARKET_ORDER { size: FULL, style: IMMEDIATE }` |
    /// | [`InternaliseToDv01`](Self::InternaliseToDv01) | `IF breached == false THEN WAREHOUSE ELSE SUBMIT_MARKET_ORDER { size: OVERFLOW, style: IMMEDIATE }` |
    ///
    /// Both derived graphs name **no** instrument and **no** LP, so each validates
    /// against any registry, is acyclic, and every path terminates — the same
    /// well-formedness [`HedgeGraph::validate`](crate::HedgeGraph::validate) proves of
    /// a hand-authored graph.
    #[must_use]
    pub fn derived_graph(self) -> Option<HedgeGraph> {
        match self {
            HedgingModel::Custom => None,
            HedgingModel::BackToBack => {
                let mut nodes = BTreeMap::new();
                // A single unconditional leaf. There is deliberately NO `breached`
                // test: back-to-back does not consult a budget, because it never
                // warehouses anything for a budget to measure. `HedgeSize::Full`
                // sizes the whole net (the engine caps every size at `|net_risk|`),
                // which at a back-to-back desk's steady state IS the fill just booked.
                nodes.insert(
                    0u32,
                    HedgeNode::Action {
                        exit: ExitAction::SubmitMarketOrder {
                            size: HedgeSize::Full,
                            style: ExecStyle::Immediate,
                        },
                        vehicle: HedgeVehicle::SelfInstrument,
                    },
                );
                Some(HedgeGraph { entry: 0, nodes })
            }
            HedgingModel::InternaliseToDv01 => {
                let mut nodes = BTreeMap::new();
                nodes.insert(
                    0u32,
                    HedgeNode::Condition {
                        field: crate::field::HedgeField::Breached,
                        op: RouteOp::Eq,
                        value: RouteValue::Text("false".to_owned()),
                        on_true: 1,
                        on_false: 2,
                    },
                );
                nodes.insert(
                    1u32,
                    HedgeNode::Action {
                        exit: ExitAction::Warehouse,
                        vehicle: HedgeVehicle::SelfInstrument,
                    },
                );
                nodes.insert(
                    2u32,
                    HedgeNode::Action {
                        exit: ExitAction::SubmitMarketOrder {
                            size: HedgeSize::Overflow,
                            style: ExecStyle::Immediate,
                        },
                        vehicle: HedgeVehicle::SelfInstrument,
                    },
                );
                Some(HedgeGraph { entry: 0, nodes })
            }
        }
    }

    /// A one-line, trader-readable statement of **what this model resolves to** — the
    /// honesty string the configuration surface renders next to the choice, so a desk
    /// can always see the graph it is really running.
    #[must_use]
    pub const fn resolution(self) -> &'static str {
        match self {
            HedgingModel::Custom => "the scope's own authored exit-policy graph",
            HedgingModel::BackToBack => "SUBMIT_MARKET_ORDER { size: FULL } on every fill",
            HedgingModel::InternaliseToDv01 => {
                "IF breached == false THEN WAREHOUSE ELSE SUBMIT_MARKET_ORDER { size: OVERFLOW }"
            }
        }
    }
}

impl fmt::Display for HedgingModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HedgeContext, HedgeRouter, WarehouseThreshold};
    use celnet_limits::LimitMetric;
    use std::collections::BTreeSet;

    fn empty() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn custom_is_the_default_so_an_unconfigured_firm_is_unchanged() {
        assert_eq!(HedgingModel::default(), HedgingModel::Custom);
        assert!(!HedgingModel::default().derives_policy());
        assert!(
            HedgingModel::default().derived_graph().is_none(),
            "Custom derives NOTHING — the authored graph governs, exactly as today"
        );
    }

    #[test]
    fn ordinals_round_trip_and_unknown_folds_to_custom() {
        for m in HedgingModel::ALL {
            assert_eq!(HedgingModel::from_i32(m.as_i32()), m);
        }
        // An unknown ordinal must never silently change how a book hedges.
        assert_eq!(HedgingModel::from_i32(99), HedgingModel::Custom);
        assert_eq!(HedgingModel::from_i32(-1), HedgingModel::Custom);
    }

    #[test]
    fn json_round_trips_and_a_legacy_config_reloads_as_custom() {
        for m in HedgingModel::ALL {
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<HedgingModel>(&json).unwrap(), m);
        }
        #[derive(Deserialize)]
        struct Scope {
            #[serde(default)]
            model: HedgingModel,
        }
        let s: Scope = serde_json::from_str("{}").unwrap();
        assert_eq!(s.model, HedgingModel::Custom);
    }

    /// Both derived graphs must be **valid** by the same rules a hand-authored graph
    /// is held to — they are ordinary graphs, not a privileged path.
    #[test]
    fn derived_graphs_validate_against_an_empty_registry() {
        for m in [HedgingModel::BackToBack, HedgingModel::InternaliseToDv01] {
            let g = m.derived_graph().expect("a derived model has a graph");
            g.validate(&empty(), &empty(), &empty())
                .unwrap_or_else(|e| panic!("{m} derived an invalid graph: {e:?}"));
        }
    }

    /// Back-to-back resolves to the FULL-size market order on **every** risk state —
    /// green band included. It never consults the budget, because it warehouses
    /// nothing.
    #[test]
    fn back_to_back_resolves_to_full_market_order_in_every_band() {
        let g = HedgingModel::BackToBack.derived_graph().unwrap();
        for (breached, util) in [(false, 0.05), (false, 0.5), (true, 0.95), (true, 4.0)] {
            let ctx = HedgeContext {
                breached,
                utilization: util,
                net_dv01: 1_000.0,
                ..HedgeContext::default()
            };
            let r = HedgeRouter::resolve(&g, &ctx).expect("total");
            assert_eq!(
                r.action,
                &ExitAction::SubmitMarketOrder {
                    size: HedgeSize::Full,
                    style: ExecStyle::Immediate
                },
                "back-to-back hedges out at utilisation {util}"
            );
            assert!(
                r.action.is_external(),
                "back-to-back always pays the street"
            );
        }
    }

    /// Internalisation resolves to WAREHOUSE inside the band and to the OVERFLOW shed
    /// once the red band fires — the historical default behaviour, unchanged.
    #[test]
    fn internalise_warehouses_inside_the_band_and_sheds_the_overflow_outside_it() {
        let g = HedgingModel::InternaliseToDv01.derived_graph().unwrap();

        let held = HedgeContext {
            breached: false,
            ..HedgeContext::default()
        };
        assert_eq!(
            HedgeRouter::resolve(&g, &held).expect("total").action,
            &ExitAction::Warehouse
        );

        let shed = HedgeContext {
            breached: true,
            ..HedgeContext::default()
        };
        assert_eq!(
            HedgeRouter::resolve(&g, &shed).expect("total").action,
            &ExitAction::SubmitMarketOrder {
                size: HedgeSize::Overflow,
                style: ExecStyle::Immediate
            }
        );
    }

    /// The economic difference, measured on one threshold: at the SAME risk state the
    /// two models shed different amounts — back-to-back flattens, internalisation
    /// sheds only the overflow to the band edge. This is the whole point of the
    /// control, so it is pinned numerically rather than described.
    #[test]
    fn the_two_models_shed_materially_different_amounts() {
        // cap 4,000 · target 80% ⇒ edge 3,200. Book holds 5,000.
        let wh = WarehouseThreshold::new(LimitMetric::Dv01, 4_000.0).with_target_fraction(0.8);
        let net = 5_000.0;

        let b2b = wh.resolve_size(net, HedgeSize::Full);
        assert!(
            (b2b - 5_000.0).abs() < 1e-9,
            "back-to-back flattens the whole net, got {b2b}"
        );

        let internalise = wh.resolve_size(net, HedgeSize::Overflow);
        assert!(
            (internalise - 1_800.0).abs() < 1e-9,
            "internalisation sheds only the 1,800 overflow to the 3,200 edge, got {internalise}"
        );
    }

    #[test]
    fn only_internalisation_reads_a_budget() {
        assert!(!HedgingModel::Custom.uses_budget());
        assert!(
            !HedgingModel::BackToBack.uses_budget(),
            "back-to-back warehouses nothing, so it has no budget to size"
        );
        assert!(HedgingModel::InternaliseToDv01.uses_budget());
    }

    #[test]
    fn labels_and_resolution_strings_are_distinct_and_non_empty() {
        for m in HedgingModel::ALL {
            assert!(!m.label().is_empty());
            assert!(!m.resolution().is_empty());
            assert_eq!(m.to_string(), m.label());
        }
        let labels: BTreeSet<&str> = HedgingModel::ALL.iter().map(|m| m.label()).collect();
        assert_eq!(labels.len(), HedgingModel::ALL.len());
    }
}
