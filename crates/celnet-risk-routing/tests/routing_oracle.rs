//! Independent oracle for `celnet-risk-routing`.
//!
//! The engine is validated NOT by re-running itself, but against a hand-computed
//! truth table of fills → books over a small, realistic graph, plus a serde
//! round-trip and two totality properties (any graph that `validate`s is total:
//! every context routes to `Ok(book)`, never panicking).

use std::collections::{BTreeMap, BTreeSet};

use celnet_risk_routing::{
    FieldKind, NodeId, RiskRouter, RiskRoutingGraph, RouteField, RouteOp, RouteValue,
    RoutingContext, RoutingNode,
};
use proptest::prelude::*;

// ---- shared builders --------------------------------------------------------

fn cond(field: RouteField, op: RouteOp, value: RouteValue, t: NodeId, f: NodeId) -> RoutingNode {
    RoutingNode::Condition {
        field,
        op,
        value,
        on_true: t,
        on_false: f,
    }
}

fn book(id: &str) -> RoutingNode {
    RoutingNode::Book {
        risk_book_id: id.to_string(),
    }
}

fn known(books: &[&str]) -> BTreeSet<String> {
    books.iter().map(|s| s.to_string()).collect()
}

/// The oracle graph, encoding (in priority order):
/// 1. `ccy == EUR` AND `notional > 50_000_000` → `BOOK-A`
/// 2. else `product == swap` AND `tenor >= 10` → `BOOK-B`
/// 3. else `counterparty in [HF-1, HF-2]` → `BOOK-C`
/// 4. else → `DEFAULT`
///
/// Node layout:
/// - 0: ccy == EUR      ? 1 : 3
/// - 1: notional > 50m  ? 2 : 3      (BOOK-A on true; small EUR falls to product test)
/// - 2: Book BOOK-A
/// - 3: product == swap ? 4 : 6
/// - 4: tenor >= 10     ? 5 : 6      (BOOK-B on true)
/// - 5: Book BOOK-B
/// - 6: cp in [HF-1,HF-2]? 7 : 8     (BOOK-C on true)
/// - 7: Book BOOK-C
/// - 8: Book DEFAULT
fn oracle_graph() -> RiskRoutingGraph {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        0,
        cond(
            RouteField::Ccy,
            RouteOp::Eq,
            RouteValue::Text("EUR".into()),
            1,
            3,
        ),
    );
    nodes.insert(
        1,
        cond(
            RouteField::Notional,
            RouteOp::Gt,
            RouteValue::Num(50_000_000.0),
            2,
            3,
        ),
    );
    nodes.insert(2, book("BOOK-A"));
    nodes.insert(
        3,
        cond(
            RouteField::Product,
            RouteOp::Eq,
            RouteValue::Text("swap".into()),
            4,
            6,
        ),
    );
    nodes.insert(
        4,
        cond(RouteField::Tenor, RouteOp::Ge, RouteValue::Num(10.0), 5, 6),
    );
    nodes.insert(5, book("BOOK-B"));
    nodes.insert(
        6,
        cond(
            RouteField::Counterparty,
            RouteOp::In,
            RouteValue::List(vec!["HF-1".into(), "HF-2".into()]),
            7,
            8,
        ),
    );
    nodes.insert(7, book("BOOK-C"));
    nodes.insert(8, book("DEFAULT"));
    RiskRoutingGraph { entry: 0, nodes }
}

fn oracle_books() -> BTreeSet<String> {
    known(&["BOOK-A", "BOOK-B", "BOOK-C", "DEFAULT"])
}

/// A fill with the oracle's relevant fields set; others defaulted.
fn fill(ccy: &str, notional: f64, product: &str, tenor: f64, cp: &str) -> RoutingContext {
    RoutingContext {
        ccy: ccy.into(),
        notional,
        product: product.into(),
        tenor,
        counterparty: cp.into(),
        ..Default::default()
    }
}

// ---- truth table ------------------------------------------------------------

#[test]
fn oracle_graph_is_valid() {
    oracle_graph()
        .validate(&oracle_books())
        .expect("oracle graph must be well-formed");
}

#[test]
fn truth_table_routes_as_hand_computed() {
    let g = oracle_graph();
    // (ccy, notional, product, tenor, counterparty) -> expected book, with why.
    let cases: &[(&str, f64, &str, f64, &str, &str, &str)] = &[
        // EUR & notional strictly > 50m → BOOK-A.
        (
            "EUR",
            60_000_000.0,
            "vanilla",
            1.0,
            "CLIENT",
            "BOOK-A",
            "EUR & >50m",
        ),
        // EUR but notional == 50m (Gt is strict → false); vanilla, cp not HF → DEFAULT.
        (
            "EUR",
            50_000_000.0,
            "vanilla",
            1.0,
            "CLIENT",
            "DEFAULT",
            "EUR but notional == 50m (Gt strict)",
        ),
        // EUR & just over 50m → BOOK-A (Gt boundary just above).
        (
            "EUR",
            50_000_000.01,
            "vanilla",
            1.0,
            "CLIENT",
            "BOOK-A",
            "EUR & just over 50m",
        ),
        // swap & tenor == 10 → BOOK-B (Ge inclusive boundary).
        (
            "USD",
            10_000_000.0,
            "swap",
            10.0,
            "CLIENT",
            "BOOK-B",
            "swap & tenor == 10 (Ge boundary)",
        ),
        // swap & tenor just under 10 → not BOOK-B; cp not HF → DEFAULT.
        (
            "USD",
            10_000_000.0,
            "swap",
            9.99,
            "CLIENT",
            "DEFAULT",
            "swap & tenor 9.99 (< 10)",
        ),
        // swap & long tenor → BOOK-B.
        (
            "USD",
            10_000_000.0,
            "swap",
            30.0,
            "CLIENT",
            "BOOK-B",
            "swap & tenor 30",
        ),
        // Not EUR, not swap, cp HF-1 → BOOK-C.
        (
            "GBP",
            5_000_000.0,
            "bond",
            5.0,
            "HF-1",
            "BOOK-C",
            "cp in [HF-1,HF-2] (HF-1)",
        ),
        // cp HF-2 → BOOK-C.
        (
            "GBP",
            5_000_000.0,
            "bond",
            5.0,
            "HF-2",
            "BOOK-C",
            "cp in [HF-1,HF-2] (HF-2)",
        ),
        // cp HF-3 (not in list) → DEFAULT.
        (
            "GBP",
            5_000_000.0,
            "bond",
            5.0,
            "HF-3",
            "DEFAULT",
            "cp HF-3 not in list",
        ),
        // Empty counterparty, nothing matches → DEFAULT.
        (
            "USD",
            1_000_000.0,
            "vanilla",
            1.0,
            "",
            "DEFAULT",
            "empty cp, unmatched",
        ),
        // EUR & small notional, but product swap & long tenor → BOOK-B via the product branch.
        (
            "EUR",
            10_000_000.0,
            "swap",
            20.0,
            "CLIENT",
            "BOOK-B",
            "small EUR falls through to swap branch",
        ),
    ];

    for (ccy, notional, product, tenor, cp, expected, why) in cases {
        let ctx = fill(ccy, *notional, product, *tenor, cp);
        let got = RiskRouter::route(&g, &ctx).expect("validated graph is total");
        assert_eq!(got, *expected, "case: {why}  (ctx={ctx:?})");
    }
}

// ---- serde round-trip -------------------------------------------------------

#[test]
fn graph_json_round_trips() {
    let g = oracle_graph();
    let json = serde_json::to_string(&g).expect("serialize");
    let back: RiskRoutingGraph = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        g, back,
        "graph must survive a JSON round-trip byte-for-value"
    );
    // And the rehydrated graph routes identically.
    let ctx = fill("EUR", 60_000_000.0, "vanilla", 1.0, "CLIENT");
    assert_eq!(
        RiskRouter::route(&back, &ctx).unwrap(),
        RiskRouter::route(&g, &ctx).unwrap()
    );
}

// ---- totality properties ----------------------------------------------------

const GEN_BOOKS: &[&str] = &["RB-0", "RB-1", "RB-2", "RB-3"];

const FIELDS: &[RouteField] = &[
    RouteField::InstrumentId,
    RouteField::Ccy,
    RouteField::Product,
    RouteField::Side,
    RouteField::Notional,
    RouteField::Tenor,
    RouteField::Strike,
    RouteField::Counterparty,
    RouteField::User,
    RouteField::Desk,
    RouteField::Price,
];

/// Operators known-valid for a field kind (mirrors [`RouteOp::valid_for`]).
fn valid_ops(kind: FieldKind) -> &'static [RouteOp] {
    use RouteOp::*;
    match kind {
        FieldKind::Numeric => &[Eq, Ne, Gt, Ge, Lt, Le, Between],
        FieldKind::String => &[Eq, Ne, Contains, In],
        FieldKind::Enum => &[Eq, Ne, In],
    }
}

/// A value that matches `op` for `kind` — so the constructed condition always
/// passes validation's type check.
fn value_for(
    kind: FieldKind,
    op: RouteOp,
    num: f64,
    num2: f64,
    text: String,
    list: Vec<String>,
) -> RouteValue {
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

/// One raw node seed. A `Condition`'s edges are clamped forward (`> i`) at build
/// time, guaranteeing an acyclic, always-terminating DAG.
type NodeSeed = (
    bool,        // is_book
    u8,          // book selector
    u8,          // field selector
    u8,          // op selector
    f64,         // num
    f64,         // num2
    String,      // text
    Vec<String>, // list
    u32,         // on_true raw
    u32,         // on_false raw
);

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

/// Build a guaranteed-valid layered graph from raw seeds: node `i`'s edges point
/// strictly forward into `(i, len)`, the last node is forced to a `Book`, and
/// every book id is drawn from `GEN_BOOKS`.
fn build_valid_graph(seeds: &[NodeSeed]) -> RiskRoutingGraph {
    let len = seeds.len();
    let mut nodes = BTreeMap::new();
    for (i, seed) in seeds.iter().enumerate() {
        let (is_book, book_sel, field_sel, op_sel, num, num2, text, list, t_raw, f_raw) = seed;
        let is_last = i + 1 == len;
        let node = if *is_book || is_last {
            book(GEN_BOOKS[(*book_sel as usize) % GEN_BOOKS.len()])
        } else {
            let field = FIELDS[(*field_sel as usize) % FIELDS.len()];
            let ops = valid_ops(field.kind());
            let op = ops[(*op_sel as usize) % ops.len()];
            let value = value_for(field.kind(), op, *num, *num2, text.clone(), list.clone());
            // Forward edges into (i, len): span = len - 1 - i >= 1 here.
            let span = (len - 1 - i) as u32;
            let t = (i as u32) + 1 + (t_raw % span);
            let f = (i as u32) + 1 + (f_raw % span);
            cond(field, op, value, t, f)
        };
        nodes.insert(i as u32, node);
    }
    RiskRoutingGraph { entry: 0, nodes }
}

fn arb_context() -> impl Strategy<Value = RoutingContext> {
    (
        "[A-Za-z0-9-]{0,8}",
        "[A-Za-z0-9-]{0,8}",
        "[A-Za-z0-9-]{0,8}",
        "[A-Za-z0-9-]{0,8}",
        any::<f64>(),
        any::<f64>(),
        any::<f64>(),
        "[A-Za-z0-9-]{0,8}",
        "[A-Za-z0-9-]{0,8}",
        "[A-Za-z0-9-]{0,8}",
        any::<f64>(),
    )
        .prop_map(
            |(
                instrument_id,
                ccy,
                product,
                side,
                notional,
                tenor,
                strike,
                counterparty,
                user,
                desk,
                price,
            )| {
                RoutingContext {
                    instrument_id,
                    ccy,
                    product,
                    side,
                    notional,
                    tenor,
                    strike,
                    counterparty,
                    user,
                    desk,
                    price,
                }
            },
        )
}

proptest! {
    /// Constructive: a layered DAG always validates, and then routes ANY context
    /// to a known book — total, never panics.
    #[test]
    fn valid_graph_is_total(
        seeds in prop::collection::vec(node_seed(), 1..14),
        ctx in arb_context(),
    ) {
        let g = build_valid_graph(&seeds);
        let books = known(GEN_BOOKS);
        prop_assert!(g.validate(&books).is_ok(), "layered DAG must validate: {:?}", g.validate(&books));
        let landed = RiskRouter::route(&g, &ctx).expect("validated graph is total");
        prop_assert!(books.contains(landed), "routed to unknown book {landed}");
    }

    /// The implication holds for arbitrary (mostly-invalid) graphs too: whenever
    /// validate succeeds, route succeeds for any context. Invalid graphs make the
    /// implication vacuously true — but neither call may ever panic.
    #[test]
    fn validate_ok_implies_route_ok(
        seeds in prop::collection::vec(node_seed(), 1..14),
        entry in any::<u32>(),
        ctx in arb_context(),
    ) {
        // Build a graph WITHOUT the forward-edge clamp so cycles/dangles occur.
        let len = seeds.len() as u32;
        let mut nodes = BTreeMap::new();
        for (i, seed) in seeds.iter().enumerate() {
            let (is_book, book_sel, field_sel, op_sel, num, num2, text, list, t_raw, f_raw) = seed;
            let node = if *is_book {
                book(GEN_BOOKS[(*book_sel as usize) % GEN_BOOKS.len()])
            } else {
                let field = FIELDS[(*field_sel as usize) % FIELDS.len()];
                let ops = valid_ops(field.kind());
                let op = ops[(*op_sel as usize) % ops.len()];
                let value = value_for(field.kind(), op, *num, *num2, text.clone(), list.clone());
                cond(field, op, value, t_raw % len, f_raw % len)
            };
            nodes.insert(i as u32, node);
        }
        let g = RiskRoutingGraph { entry: entry % len, nodes };
        let books = known(GEN_BOOKS);
        // Never panics regardless of validity.
        if g.validate(&books).is_ok() {
            let landed = RiskRouter::route(&g, &ctx).expect("validated graph is total");
            prop_assert!(books.contains(landed));
        } else {
            // Even an invalid graph must route without panicking (Ok or Err).
            let _ = RiskRouter::route(&g, &ctx);
        }
    }
}
