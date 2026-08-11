//! Independent oracle for `celnet-hedge-routing`.
//!
//! The engine is validated NOT by re-running itself, but against hand-computed
//! truth tables:
//!
//! 1. `(risk-state, policy) → exit action` over a realistic exit-policy graph
//!    (the §5.4 worked example), ≥ 9 cases.
//! 2. `(net_risk, threshold) → (band, overflow, hedge_size)` over the banded
//!    warehouse model, ≥ 11 cases, each arithmetic worked by hand.
//! 3. the internalise-then-hedge netting decomposition.
//!
//! Plus a serde round-trip and property tests (overflow/size monotone in the
//! breach magnitude; resolution deterministic; every validated graph is total).

use std::collections::{BTreeMap, BTreeSet};

use celnet_hedge_routing::{
    ExecStyle, ExitAction, HedgeContext, HedgeField, HedgeGraph, HedgeNode, HedgeRouter, HedgeSize,
    LimitMetric, RagStatus, RouteOp, RouteValue, WarehouseThreshold, netting_split,
};
use proptest::prelude::*;

// ---- shared builders --------------------------------------------------------

fn cond(field: HedgeField, op: RouteOp, value: RouteValue, t: u32, f: u32) -> HedgeNode {
    HedgeNode::Condition {
        field,
        op,
        value,
        on_true: t,
        on_false: f,
    }
}

fn act(exit: ExitAction) -> HedgeNode {
    HedgeNode::action(exit)
}

/// The hedge-instrument ids the firm's vehicle registry knows (no leaf in these
/// fixtures names a vehicle, so the set only has to exist).
fn known_vehicles() -> BTreeSet<String> {
    BTreeSet::new()
}

fn known(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// ---- 1. exit-policy truth table ---------------------------------------------

/// The §5.4 worked example, expressed with literal thresholds (a graph condition's
/// RHS is a literal, so the "internal_offset > overflow" prose becomes a fixed
/// offset floor):
///
/// - 0: `breached == false`               ? WAREHOUSE (1)          : 2
/// - 2: `counterparty_toxicity > 0.6`     ? SUBMIT_MARKET_ORDER (3): 4   (toxic → back-to-back)
/// - 4: `internal_offset_available > 10k` ? CROSS_INTERNAL (5)     : 6   (offset exists → net internally)
/// - 6: `overflow > 50k`                  ? RFQ_OUT (7)            : SPLIT (8)
fn policy_graph() -> HedgeGraph {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        0,
        cond(
            HedgeField::Breached,
            RouteOp::Eq,
            RouteValue::Text("false".into()),
            1,
            2,
        ),
    );
    nodes.insert(1, act(ExitAction::Warehouse));
    nodes.insert(
        2,
        cond(
            HedgeField::CounterpartyToxicity,
            RouteOp::Gt,
            RouteValue::Num(0.6),
            3,
            4,
        ),
    );
    nodes.insert(
        3,
        act(ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        }),
    );
    nodes.insert(
        4,
        cond(
            HedgeField::InternalOffsetAvailable,
            RouteOp::Gt,
            RouteValue::Num(10_000.0),
            5,
            6,
        ),
    );
    nodes.insert(
        5,
        act(ExitAction::CrossInternal {
            instrument: "EURUSD".into(),
            max_size: HedgeSize::Overflow,
        }),
    );
    nodes.insert(
        6,
        cond(
            HedgeField::Overflow,
            RouteOp::Gt,
            RouteValue::Num(50_000.0),
            7,
            8,
        ),
    );
    nodes.insert(
        7,
        act(ExitAction::RfqOut {
            lps: vec!["LP-1".into(), "LP-2".into(), "LP-3".into()],
            size: HedgeSize::Overflow,
        }),
    );
    nodes.insert(
        8,
        act(ExitAction::Split {
            internal_first: true,
            style: ExecStyle::Immediate,
        }),
    );
    HedgeGraph { entry: 0, nodes }
}

fn policy_instruments() -> BTreeSet<String> {
    known(&["EURUSD", "GBPUSD"])
}
fn policy_lps() -> BTreeSet<String> {
    known(&["LP-1", "LP-2", "LP-3"])
}

/// A risk state with the policy's relevant fields set; others defaulted.
fn state(breached: bool, toxicity: f64, offset: f64, overflow: f64) -> HedgeContext {
    HedgeContext {
        breached,
        counterparty_toxicity: toxicity,
        internal_offset_available: offset,
        overflow,
        ..Default::default()
    }
}

#[test]
fn policy_graph_is_valid() {
    policy_graph()
        .validate(&policy_instruments(), &policy_lps(), &known_vehicles())
        .expect("policy graph must be well-formed");
}

#[test]
fn exit_policy_truth_table() {
    let g = policy_graph();
    // (breached, toxicity, offset, overflow) -> expected action kind, with why.
    let cases: &[(bool, f64, f64, f64, &str, &str)] = &[
        (
            false,
            0.9,
            0.0,
            99_999.0,
            "WAREHOUSE",
            "not breached → warehouse",
        ),
        (
            true,
            0.7,
            0.0,
            10_000.0,
            "SUBMIT_MARKET_ORDER",
            "toxic (0.7>0.6) → back-to-back",
        ),
        (
            true,
            0.61,
            0.0,
            10_000.0,
            "SUBMIT_MARKET_ORDER",
            "toxic just over 0.6",
        ),
        (
            true,
            0.6,
            20_000.0,
            10_000.0,
            "CROSS_INTERNAL",
            "toxicity == 0.6 (Gt strict false) & offset 20k>10k → cross",
        ),
        (
            true,
            0.5,
            15_000.0,
            10_000.0,
            "CROSS_INTERNAL",
            "benign & offset 15k → cross",
        ),
        (
            true,
            0.5,
            10_000.0,
            60_000.0,
            "RFQ_OUT",
            "offset == 10k (Gt strict false) & overflow 60k>50k → rfq",
        ),
        (
            true,
            0.5,
            5_000.0,
            60_000.0,
            "RFQ_OUT",
            "small offset & big overflow → rfq",
        ),
        (
            true,
            0.5,
            5_000.0,
            40_000.0,
            "SPLIT",
            "small offset & overflow 40k (≤50k) → split",
        ),
        (
            true,
            0.0,
            0.0,
            0.0,
            "SPLIT",
            "breached, nothing else → split",
        ),
    ];

    for (breached, tox, offset, overflow, expected, why) in cases {
        let ctx = state(*breached, *tox, *offset, *overflow);
        let r = HedgeRouter::resolve(&g, &ctx).expect("validated graph is total");
        assert_eq!(r.action.kind(), *expected, "case: {why}");
    }
}

#[test]
fn resolution_records_the_policy_path() {
    let g = policy_graph();
    // Toxic breach walks 0 → 2 → 3.
    let r = HedgeRouter::resolve(&g, &state(true, 0.9, 0.0, 1.0)).unwrap();
    assert_eq!(r.path, vec![0, 2, 3]);
    // Benign, offset present walks 0 → 2 → 4 → 5.
    let r2 = HedgeRouter::resolve(&g, &state(true, 0.1, 20_000.0, 1.0)).unwrap();
    assert_eq!(r2.path, vec![0, 2, 4, 5]);
}

// ---- 2. band / sizing truth table -------------------------------------------

#[test]
fn band_and_sizing_truth_table() {
    // cap 100k, amber 0.8 (=80k), red 0.9 (=90k), target = amber edge (80k).
    let t = WarehouseThreshold::new(LimitMetric::Dv01, 100_000.0);
    // (net_risk, expected band, expected overflow, expected size)
    let cases: &[(f64, RagStatus, f64, f64)] = &[
        (40_000.0, RagStatus::Green, 0.0, 0.0), // comfortably inside
        (80_000.0, RagStatus::Amber, 0.0, 0.0), // == amber edge
        (85_000.0, RagStatus::Amber, 5_000.0, 0.0), // amber: overflow exists, no hedge
        (90_000.0, RagStatus::Red, 10_000.0, 10_000.0), // == red edge
        (95_000.0, RagStatus::Red, 15_000.0, 15_000.0),
        (100_000.0, RagStatus::Red, 20_000.0, 20_000.0), // == cap (util 1.0)
        (120_000.0, RagStatus::Breach, 40_000.0, 40_000.0), // over cap
        (-95_000.0, RagStatus::Red, 15_000.0, 15_000.0), // sign-agnostic
    ];
    for (net, band, overflow, size) in cases {
        let s = t.sizing(*net);
        assert_eq!(s.band, *band, "band for net={net}");
        assert!(
            (s.overflow - overflow).abs() < 1e-6,
            "overflow net={net}: {}",
            s.overflow
        );
        assert!((s.size - size).abs() < 1e-6, "size net={net}: {}", s.size);
    }
}

#[test]
fn band_and_sizing_truth_table_with_clips() {
    // min 5k, max 30k clip on the same 100k/amber-edge threshold.
    let t = WarehouseThreshold::new(LimitMetric::Dv01, 100_000.0).with_clips(5_000.0, 30_000.0);
    // net 150k: breach, overflow 70k, capped to max 30k.
    let s = t.sizing(150_000.0);
    assert_eq!(s.band, RagStatus::Breach);
    assert!((s.overflow - 70_000.0).abs() < 1e-6);
    assert!((s.size - 30_000.0).abs() < 1e-6);

    // A sub-min overflow floors up to the minimum ticket: target 90k, net 92k.
    let t2 = WarehouseThreshold::new(LimitMetric::Dv01, 100_000.0)
        .with_target_fraction(0.9)
        .with_clips(5_000.0, 30_000.0);
    let s2 = t2.sizing(92_000.0); // red (util 0.92), overflow 2k < min 5k → 5k
    assert_eq!(s2.band, RagStatus::Red);
    assert!((s2.overflow - 2_000.0).abs() < 1e-6);
    assert!((s2.size - 5_000.0).abs() < 1e-6);

    // Below the red band never hedges even with a min clip: net 85k (amber).
    let s3 = t.sizing(85_000.0);
    assert!(!s3.breached);
    assert_eq!(s3.size, 0.0);
}

// ---- 3. netting decomposition -----------------------------------------------

#[test]
fn netting_waterfall_truth_table() {
    // want 15k, offset 20k, internal-first → all internal.
    let n = netting_split(15_000.0, 20_000.0, true);
    assert_eq!(n.internal_crossed, 15_000.0);
    assert_eq!(n.external_hedged, 0.0);
    // want 15k, offset 6k → 6k internal + 9k external.
    let n2 = netting_split(15_000.0, 6_000.0, true);
    assert_eq!(n2.internal_crossed, 6_000.0);
    assert_eq!(n2.external_hedged, 9_000.0);
    // externalise-first policy crosses nothing.
    let n3 = netting_split(15_000.0, 20_000.0, false);
    assert_eq!(n3.internal_crossed, 0.0);
    assert_eq!(n3.external_hedged, 15_000.0);
    // Conservation: internal + external == want, always.
    for (want, offset) in [(15_000.0, 6_000.0), (100.0, 0.0), (50.0, 200.0)] {
        let s = netting_split(want, offset, true);
        assert!((s.internal_crossed + s.external_hedged - want).abs() < 1e-9);
    }
}

// ---- serde round-trip -------------------------------------------------------

#[test]
fn graph_json_round_trips() {
    let g = policy_graph();
    let json = serde_json::to_string(&g).expect("serialize");
    let back: HedgeGraph = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(g, back, "graph must survive a JSON round-trip");
    let ctx = state(true, 0.9, 0.0, 1.0);
    assert_eq!(
        HedgeRouter::action(&back, &ctx).unwrap().kind(),
        HedgeRouter::action(&g, &ctx).unwrap().kind()
    );
}

// ---- property tests ---------------------------------------------------------

proptest! {
    /// Overflow and (non-ramped) size are monotone non-decreasing in the breach
    /// magnitude |net_risk|: shedding more of a bigger position never shrinks.
    #[test]
    fn overflow_and_size_monotone_in_magnitude(
        a in 0.0f64..500_000.0,
        b in 0.0f64..500_000.0,
    ) {
        let t = WarehouseThreshold::new(LimitMetric::Dv01, 100_000.0)
            .with_clips(1_000.0, 250_000.0);
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let s_lo = t.sizing(lo);
        let s_hi = t.sizing(hi);
        prop_assert!(s_hi.overflow >= s_lo.overflow - 1e-6,
            "overflow not monotone: {} !>= {}", s_hi.overflow, s_lo.overflow);
        prop_assert!(s_hi.size >= s_lo.size - 1e-6,
            "size not monotone: {} !>= {}", s_hi.size, s_lo.size);
    }

    /// Resolution is deterministic and never panics for any numeric risk state.
    #[test]
    fn resolution_is_deterministic(
        breached in any::<bool>(),
        tox in -1.0f64..2.0,
        offset in -1e9f64..1e9,
        overflow in -1e9f64..1e9,
    ) {
        let g = policy_graph();
        let ctx = state(breached, tox, offset, overflow);
        let a = HedgeRouter::resolve(&g, &ctx).expect("validated graph is total");
        let b = HedgeRouter::resolve(&g, &ctx).expect("validated graph is total");
        prop_assert_eq!(a.action.kind(), b.action.kind());
        prop_assert_eq!(a.path, b.path);
    }

    /// Netting always conserves and never externalises more than `want`.
    #[test]
    fn netting_conserves(
        want in 0.0f64..1e9,
        offset in -1e6f64..1e9,
        internal_first in any::<bool>(),
    ) {
        let s = netting_split(want, offset, internal_first);
        prop_assert!(s.internal_crossed >= 0.0);
        prop_assert!(s.external_hedged >= 0.0);
        prop_assert!((s.internal_crossed + s.external_hedged - want).abs() < 1e-3);
        prop_assert!(s.internal_crossed <= want + 1e-6);
    }
}

// ---- totality over generated valid graphs -----------------------------------

const GEN_INSTR: &[&str] = &["EURUSD", "GBPUSD", "USDJPY"];
const GEN_LPS: &[&str] = &["LP-1", "LP-2", "LP-3"];

const FIELDS: &[HedgeField] = &[
    HedgeField::NetDv01,
    HedgeField::NetNotional,
    HedgeField::Utilization,
    HedgeField::Overflow,
    HedgeField::CounterpartyToxicity,
    HedgeField::InternalOffsetAvailable,
    HedgeField::Ccy,
    HedgeField::Breached,
    HedgeField::InstrumentId,
];

fn valid_ops(kind: celnet_hedge_routing::FieldKind) -> &'static [RouteOp] {
    use RouteOp::*;
    use celnet_hedge_routing::FieldKind;
    match kind {
        FieldKind::Numeric => &[Eq, Ne, Gt, Ge, Lt, Le, Between],
        FieldKind::String => &[Eq, Ne, Contains, In],
        FieldKind::Enum => &[Eq, Ne, In],
    }
}

fn value_for(
    kind: celnet_hedge_routing::FieldKind,
    op: RouteOp,
    num: f64,
    num2: f64,
    text: String,
    list: Vec<String>,
) -> RouteValue {
    use celnet_hedge_routing::FieldKind;
    match op {
        RouteOp::Between => RouteValue::Range { lo: num, hi: num2 },
        RouteOp::In => RouteValue::List(list),
        RouteOp::Contains => RouteValue::Text(text),
        RouteOp::Gt | RouteOp::Ge | RouteOp::Lt | RouteOp::Le => RouteValue::Num(num),
        RouteOp::Eq | RouteOp::Ne => match kind {
            FieldKind::Numeric => RouteValue::Num(num),
            _ => RouteValue::Text(text),
        },
    }
}

/// A valid action leaf drawn from a selector, always passing validation.
fn action_for(sel: u8) -> ExitAction {
    match sel % 5 {
        0 => ExitAction::Warehouse,
        1 => ExitAction::SubmitMarketOrder {
            size: HedgeSize::Overflow,
            style: ExecStyle::Immediate,
        },
        2 => ExitAction::CrossInternal {
            instrument: GEN_INSTR[(sel as usize / 5) % GEN_INSTR.len()].into(),
            max_size: HedgeSize::Overflow,
        },
        3 => ExitAction::RfqOut {
            lps: vec![GEN_LPS[(sel as usize) % GEN_LPS.len()].into()],
            size: HedgeSize::Full,
        },
        _ => ExitAction::Split {
            internal_first: true,
            style: ExecStyle::Worked,
        },
    }
}

type NodeSeed = (bool, u8, u8, u8, f64, f64, String, Vec<String>, u32, u32);

fn node_seed() -> impl Strategy<Value = NodeSeed> {
    (
        any::<bool>(),
        any::<u8>(),
        any::<u8>(),
        any::<u8>(),
        any::<f64>(),
        any::<f64>(),
        "[A-Za-z0-9-]{0,6}",
        prop::collection::vec("[A-Za-z0-9-]{0,6}", 0..4),
        any::<u32>(),
        any::<u32>(),
    )
}

/// Build a guaranteed-valid layered graph: node `i`'s edges point strictly forward,
/// the last node is forced to a valid `Action`, every venue/LP target is known.
fn build_valid_graph(seeds: &[NodeSeed]) -> HedgeGraph {
    let len = seeds.len();
    let mut nodes = BTreeMap::new();
    for (i, seed) in seeds.iter().enumerate() {
        let (is_action, sel, field_sel, op_sel, num, num2, text, list, t_raw, f_raw) = seed;
        let is_last = i + 1 == len;
        let node = if *is_action || is_last {
            act(action_for(*sel))
        } else {
            let field = FIELDS[(*field_sel as usize) % FIELDS.len()];
            let ops = valid_ops(field.kind());
            let op = ops[(*op_sel as usize) % ops.len()];
            let value = value_for(field.kind(), op, *num, *num2, text.clone(), list.clone());
            let span = (len - 1 - i) as u32;
            let t = (i as u32) + 1 + (t_raw % span);
            let f = (i as u32) + 1 + (f_raw % span);
            cond(field, op, value, t, f)
        };
        nodes.insert(i as u32, node);
    }
    HedgeGraph { entry: 0, nodes }
}

fn arb_context() -> impl Strategy<Value = HedgeContext> {
    (
        "[A-Za-z0-9-]{0,8}",
        "[A-Za-z0-9-]{0,8}",
        any::<bool>(),
        any::<f64>(),
        any::<f64>(),
        any::<f64>(),
        any::<f64>(),
    )
        .prop_map(
            |(instrument_id, ccy, breached, net_dv01, overflow, toxicity, offset)| HedgeContext {
                instrument_id,
                ccy,
                breached,
                net_dv01,
                overflow,
                counterparty_toxicity: toxicity,
                internal_offset_available: offset,
                ..Default::default()
            },
        )
}

proptest! {
    /// A layered DAG always validates and then resolves ANY context to an action —
    /// total, never panics.
    #[test]
    fn valid_graph_is_total(
        seeds in prop::collection::vec(node_seed(), 1..14),
        ctx in arb_context(),
    ) {
        let g = build_valid_graph(&seeds);
        let instr = known(GEN_INSTR);
        let lps = known(GEN_LPS);
        prop_assert!(g.validate(&instr, &lps, &known_vehicles()).is_ok(),
            "layered DAG must validate: {:?}", g.validate(&instr, &lps, &known_vehicles()));
        let r = HedgeRouter::resolve(&g, &ctx).expect("validated graph is total");
        // The resolved leaf is a real action node.
        let leaf_is_action = matches!(
            g.nodes.get(r.path.last().unwrap()),
            Some(HedgeNode::Action { .. })
        );
        prop_assert!(leaf_is_action);
    }
}
