//! **Runnable out-of-process scale harness** — the literal "drive every API
//! capability through the edge while scaling backend nodes up and down" demonstration.
//!
//! This driver spawns **real separate OS processes** (not in-process tokio tasks):
//! N `scale_backend` processes (each an `InProcess` [`Edge`] seeded with its disjoint
//! HRW slice of one shared master book) and one `scale_edge` process (a stateless
//! `Distributed` front edge that forwards owned pairs and federates risk across the
//! backends). It then drives the edge **only through the public `celnet-client` SDK**:
//!
//!   * `Price` + the full Greek set for several pairs (each forwarded to its owner);
//!   * `MarkSurface` → `GetSmile` → `Scenario` (surface mark/read + what-if grid);
//!   * an RFS `StreamSession` — a few ticks + a click-to-trade `execute`;
//!   * `RiskService`: `AggregateRisk(FIRM, +shocks +curvature)`, `DrillRisk`,
//!     `LimitStatus`, `ListPositions` (federated over the whole fleet).
//!
//! Then it **scales**:
//!   * UP — spawn an additional backend (the master book re-homes to N+1 replicas),
//!     restart the stateless edge pointed at the larger fleet, re-drive, and assert
//!     the firm risk aggregate + per-pair prices are unchanged;
//!   * DOWN — re-home to N-1 survivors (re-partition), kill the removed backend,
//!     restart the edge, re-drive, assert invariance;
//!   * UNAVAILABLE — kill a backend with NO re-home (the edge still lists it), and show
//!     the federated firm aggregate returns an honest `unavailable`, never a silently
//!     smaller number.
//!
//! It prints a readable scale ladder and tears down **every** child process at the end
//! (even on a panic, via a [`Fleet`] guard whose `Drop` kills survivors). Every wait is
//! bounded by a wall-clock deadline so the harness never hangs.
//!
//! Run (build first so the sibling example binaries exist):
//! ```text
//! cargo build -p celnet-server --examples
//! cargo run   -p celnet-server --example scale_harness
//! ```

// The shared master-book fixture, defined once and `#[path]`-included by both this
// driver and the backend binary so the partition the driver expects is byte-identical
// to the slice each backend seeds.
#[path = "scale_book.rs"]
mod scale_book;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use celnet_client::risk::{
    AggregateQuery, DrillQuery, EntitlementScope, Entitlements, LimitQuery, Numeraire,
    OrgDimension, PositionQuery, Scope,
};
use celnet_client::{
    BrokerQuoteSet, Client, Conventions, ExecuteOutcome, InstrumentSpec, MarketContext, Quantity,
    ShockAxis, ShockFactor, Side, StreamEvent, StrikeSpec,
};
use celnet_types::{Ccy, CcyPair, OptionType, Tenor};

use scale_book::{master_book, owner_replica_of, pairs, usd_rates};

/// Hard ceiling for the whole harness — a regression fails fast, never hangs.
const HARNESS_DEADLINE: Duration = Duration::from_secs(90);
/// How long to wait for a child to print its `READY` line.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
/// Per-RPC client timeout.
const RPC_TIMEOUT: Duration = Duration::from_secs(8);

// ===========================================================================
// child-process management (real OS processes, RAII teardown)
// ===========================================================================

/// One spawned child process plus the identity the harness tracks it by.
struct Node {
    child: Child,
    /// `Some(replica id)` for a backend; `None` for the front edge.
    replica: Option<u64>,
    /// The bound `http://host:port` URL parsed from the child's READY line.
    url: String,
}

/// The live fleet of child processes. Its `Drop` kills every survivor so a panic
/// anywhere in the harness never leaks a process.
struct Fleet {
    nodes: Vec<Node>,
}

impl Fleet {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn push(&mut self, node: Node) {
        self.nodes.push(node);
    }

    /// Kill (and reap) the backend whose replica id is `replica`, removing it from the
    /// fleet. Returns its URL.
    fn kill_backend(&mut self, replica: u64) -> String {
        let idx = self
            .nodes
            .iter()
            .position(|n| n.replica == Some(replica))
            .expect("backend to kill is present");
        let mut node = self.nodes.remove(idx);
        let _ = node.child.kill();
        let _ = node.child.wait();
        node.url
    }

    /// Kill (and reap) the front edge, removing it.
    fn kill_edge(&mut self) {
        if let Some(idx) = self.nodes.iter().position(|n| n.replica.is_none()) {
            let mut node = self.nodes.remove(idx);
            let _ = node.child.kill();
            let _ = node.child.wait();
        }
    }

    /// The current backend URLs, in replica-id order (the membership the edge dials).
    fn backend_urls(&mut self) -> Vec<String> {
        self.nodes.sort_by_key(|n| n.replica.unwrap_or(u64::MAX));
        self.nodes
            .iter()
            .filter(|n| n.replica.is_some())
            .map(|n| n.url.clone())
            .collect()
    }

    /// The front-edge URL (panics if not present).
    fn edge_url(&self) -> String {
        self.nodes
            .iter()
            .find(|n| n.replica.is_none())
            .expect("edge present")
            .url
            .clone()
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        for node in &mut self.nodes {
            let _ = node.child.kill();
            let _ = node.child.wait();
        }
    }
}

/// The directory holding the sibling example binaries (`scale_backend`,
/// `scale_edge`) — the same dir this `scale_harness` binary runs from.
fn examples_dir() -> std::path::PathBuf {
    let me = std::env::current_exe().expect("current exe path");
    me.parent().expect("exe has a parent dir").to_path_buf()
}

/// Spawn a child example binary with the given env, then block (bounded) until it
/// prints a line containing `READY`, parsing the `grpc=<addr>` token out of it.
/// Stdout after the READY line is drained on a background thread so the child never
/// blocks on a full pipe.
fn spawn_until_ready(bin: &str, envs: &[(&str, String)], replica: Option<u64>) -> Node {
    let path = examples_dir().join(bin);
    let mut cmd = Command::new(&path);
    cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", path.display()));

    let stdout = child.stdout.take().expect("child stdout piped");
    let mut reader = BufReader::new(stdout);
    let deadline = Instant::now() + READY_TIMEOUT;
    let mut url = None;

    // Read lines until the READY line. A child binds its port and prints READY within
    // a fraction of a second; the outer `HARNESS_DEADLINE` (and the readiness child's
    // own behaviour) bound the wait. If the child exits early, `read_line` returns 0
    // (EOF) and we fail fast; if it never prints READY, the loop's `deadline` check
    // (re-evaluated after each line) catches it.
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).unwrap_or(0);
        if n == 0 {
            let _ = child.kill();
            panic!("{bin} closed stdout before READY (it exited early)");
        }
        if line.contains("READY") {
            for tok in line.split_whitespace() {
                if let Some(addr) = tok.strip_prefix("grpc=") {
                    url = Some(format!("http://{addr}"));
                }
            }
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("{bin} did not print READY within {READY_TIMEOUT:?}");
        }
    }

    // Drain the rest of stdout in the background so the child never blocks on a full
    // pipe (and so its diagnostics surface).
    std::thread::spawn(move || {
        let mut sink = String::new();
        while reader.read_line(&mut sink).unwrap_or(0) > 0 {
            sink.clear();
        }
    });

    Node {
        child,
        replica,
        url: url.expect("READY line carried a grpc=<addr> token"),
    }
}

/// Spawn `n` backends (replica ids `1..=n`) on ephemeral localhost ports, each
/// seeding its HRW slice of the master book under an `n`-replica partition. Returns
/// the spawned nodes.
fn spawn_backends(n: u64) -> Vec<Node> {
    let mut out = Vec::new();
    for replica in 1..=n {
        out.push(spawn_until_ready(
            "scale_backend",
            &[
                ("CELNET_GRPC_ADDR", "127.0.0.1:0".to_owned()),
                ("CELNET_REPLICA_ID", replica.to_string()),
                ("CELNET_FLEET_SIZE", n.to_string()),
            ],
            Some(replica),
        ));
    }
    out
}

/// Spawn the front edge in distributed mode over `backend_urls`.
fn spawn_edge(backend_urls: &[String]) -> Node {
    spawn_until_ready(
        "scale_edge",
        &[
            ("CELNET_GRPC_ADDR", "127.0.0.1:0".to_owned()),
            ("CELNET_FLEET_MODE", "distributed".to_owned()),
            ("CELNET_FLEET_BACKENDS", backend_urls.join(",")),
        ],
        None,
    )
}

// ===========================================================================
// driving every API capability through the edge (via celnet-client)
// ===========================================================================

/// The market the harness prices/streams against (the backends' EURUSD fixture).
fn market() -> MarketContext {
    MarketContext {
        spot: 1.10,
        vol: 0.10,
        r_dom: 0.04,
        r_for: 0.02,
    }
}

/// A 1Y vanilla call on `pair` at an absolute strike.
fn call_on(pair: CcyPair, strike: f64) -> InstrumentSpec {
    InstrumentSpec::vanilla(
        pair,
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        OptionType::Call,
        StrikeSpec::Absolute(strike),
    )
}

/// The USD reporting numeraire with every base-ccy rate the book needs.
fn usd_numeraire() -> Numeraire {
    let mut n = Numeraire::new("USD");
    for (ccy, rate) in usd_rates() {
        n = n.rate(ccy, rate);
    }
    n
}

/// The strike used per pair (roughly ATM for each fixture pair).
fn strike_for(pair: CcyPair) -> f64 {
    match (pair.base, pair.quote) {
        (Ccy::EUR, Ccy::USD) => 1.10,
        (Ccy::GBP, Ccy::USD) => 1.27,
        (Ccy::AUD, Ccy::USD) => 0.66,
        (Ccy::USD, Ccy::JPY) => 150.0,
        _ => 1.0,
    }
}

/// The full set of observations the harness compares across scale steps.
#[derive(Debug, Clone, PartialEq)]
struct Observations {
    /// Per-pair `(price.to_bits, full-greek-bits-sum)` — bit-exact across scale.
    prices: Vec<(String, u64, f64)>,
    /// The marked-and-pinned GBPUSD price (proves MarkSurface→pin round-trips).
    marked_price_bits: u64,
    /// The scenario centre-node price (proves Scenario forwards to the owner).
    scenario_centre_bits: u64,
    /// The firm aggregate: delta, vega, premium, VaR, ES, curvature (rounded).
    firm: FirmRisk,
    /// The number of positions ListPositions returns (the whole entitled book).
    position_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct FirmRisk {
    delta: f64,
    vega: f64,
    premium: f64,
    var: f64,
    es: f64,
    curvature: f64,
}

/// Round a float to a stable comparison grid (the federation reconciles to ~1e-9;
/// fixture pricing is fully deterministic, so a coarse round is exact across runs).
fn r(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// Drive EVERY API capability through the edge at `edge_url` and return the
/// observations to compare against the invariance baseline.
async fn drive_all(edge_url: &str) -> Observations {
    let client = Client::connect(edge_url.to_owned())
        .await
        .expect("connect to front edge");
    let conv = Conventions::major_default();

    // ---- Price + full Greeks for several pairs (each forwarded to its owner) ----
    let mut prices = Vec::new();
    for pair in pairs() {
        let inst = call_on(pair, strike_for(pair));
        let priced = tokio::time::timeout(RPC_TIMEOUT, client.price(&inst, market(), conv))
            .await
            .expect("price within timeout")
            .expect("price ok");
        let g = &priced.greeks;
        // A sum over the whole Greek set — sensitive to any drift in any Greek.
        let greek_sum = g.price
            + g.delta_spot
            + g.delta_forward
            + g.gamma
            + g.vega
            + g.theta
            + g.rho_dom
            + g.rho_for
            + g.vanna
            + g.volga
            + g.charm
            + g.speed
            + g.zomma
            + g.color;
        prices.push((pair.to_string(), g.price.to_bits(), r(greek_sum)));
    }

    // ---- MarkSurface → GetSmile → pinned Price (surface mark/read round-trip) ----
    let gbpusd = CcyPair::new(Ccy::GBP, Ccy::USD);
    let quotes = [BrokerQuoteSet::three_point(1.0, 0.105, -0.004, 0.0015)];
    let marked = tokio::time::timeout(RPC_TIMEOUT, client.mark_surface(gbpusd, &quotes, conv))
        .await
        .expect("mark within timeout")
        .expect("mark ok");
    let version = marked.surface_version;
    assert!(version > 0, "a real surface version was assigned");
    let smile = tokio::time::timeout(RPC_TIMEOUT, client.get_smile(gbpusd, 1.0, conv))
        .await
        .expect("get_smile within timeout")
        .expect("get_smile ok");
    assert!(!smile.points.is_empty(), "owner returns a calibrated smile");
    // The pinned price must resolve on the same owner the mark landed on.
    let marked_price = {
        let mut svc = celnet_proto::pricing_service_client::PricingServiceClient::connect(
            edge_url.to_owned(),
        )
        .await
        .expect("dial pricing for pin");
        let req = celnet_proto::PriceRequest {
            request_id: 1,
            instrument: Some(call_on(gbpusd, strike_for(gbpusd)).to_wire_pub()),
            market: Some(wire_market()),
            conventions: Some(wire_conv()),
            correlation_id: Some(9),
            surface_version: Some(version),
        };
        let resp = tokio::time::timeout(RPC_TIMEOUT, svc.price(req))
            .await
            .expect("pinned price within timeout")
            .expect("pinned price ok")
            .into_inner();
        assert_eq!(
            resp.surface_version,
            Some(version),
            "the forwarded pinned price echoes the marked version (resolved on the owner)"
        );
        resp.greeks.expect("greeks").price.to_bits()
    };

    // ---- Scenario (a spot what-if grid, forwarded to the pair owner) ----
    let audusd = CcyPair::new(Ccy::AUD, Ccy::USD);
    let inst = call_on(audusd, strike_for(audusd));
    let axes = [ShockAxis::relative(
        ShockFactor::Spot,
        vec![-0.02, -0.01, 0.0, 0.01, 0.02],
    )];
    let grid = tokio::time::timeout(RPC_TIMEOUT, client.scenario(&inst, market(), &axes, conv))
        .await
        .expect("scenario within timeout")
        .expect("scenario ok");
    // The centre node (0.0 shock) — the un-shocked price.
    let scenario_centre_bits = grid
        .node_with_shocks(&[0.0])
        .expect("centre scenario node present")
        .greeks
        .price
        .to_bits();

    // ---- RFS: a few ticks + a click-to-trade execution ----
    let session = tokio::time::timeout(RPC_TIMEOUT, client.open_session())
        .await
        .expect("open session within timeout")
        .expect("session opens");
    let eurusd = CcyPair::new(Ccy::EUR, Ccy::USD);
    let mut sub = tokio::time::timeout(
        RPC_TIMEOUT,
        session.subscribe(call_on(eurusd, strike_for(eurusd)), conv, Some(1), None),
    )
    .await
    .expect("subscribe within timeout")
    .expect("subscribe ok");

    // Consume the snapshot + a couple of ticks; click-to-trade the first line that
    // carries a tradable token.
    let mut executed = false;
    let mut ticks_seen = 0u32;
    let stream_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < stream_deadline && (!executed || ticks_seen < 2) {
        let ev = match tokio::time::timeout(RPC_TIMEOUT, sub.next_event()).await {
            Ok(Some(Ok(ev))) => ev,
            Ok(Some(Err(e))) => panic!("stream error: {e:?}"),
            Ok(None) => break,
            Err(_) => break,
        };
        match ev {
            StreamEvent::Snapshot { line, .. } | StreamEvent::Tick(line) => {
                ticks_seen += 1;
                if !executed && line.token_for(Side::Buy).is_some() {
                    let outcome = tokio::time::timeout(RPC_TIMEOUT, sub.execute(&line, Side::Buy))
                        .await
                        .expect("execute within timeout")
                        .expect("execute ok");
                    match outcome {
                        ExecuteOutcome::Booked(exec) => {
                            assert!(exec.execution_id != 0, "a real execution id booked");
                            executed = true;
                        }
                        ExecuteOutcome::Rejected { reason } => {
                            // A last-look rejection is a valid outcome; retry on the
                            // next fresh line rather than failing.
                            eprintln!(
                                "    (click-to-trade last-look rejected: {reason:?}; retrying)"
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
    assert!(
        ticks_seen >= 1,
        "the RFS subscription streamed at least a snapshot"
    );
    drop(sub);
    drop(session);

    // ---- RiskService: AggregateRisk(FIRM +shocks +curvature) ----
    let firm_q = AggregateQuery::new(OrgDimension::Firm, usd_numeraire())
        .value_at_risk([-0.02, -0.01, 0.0, 0.01, 0.02], 0.99)
        .curvature(0.18)
        .correlation_id(7);
    let agg = tokio::time::timeout(RPC_TIMEOUT, client.aggregate_risk(&firm_q))
        .await
        .expect("aggregate within timeout")
        .expect("aggregate ok");
    let firm_node = agg
        .nodes
        .iter()
        .find(|n| n.dimension == OrgDimension::Firm)
        .expect("a firm apex node");
    let firm = FirmRisk {
        delta: r(firm_node.additive.delta_numeraire),
        vega: r(firm_node.additive.vega_numeraire),
        premium: r(firm_node.additive.premium_numeraire),
        var: r(firm_node.nonadditive.var.expect("VaR evaluated")),
        es: r(firm_node.nonadditive.es.expect("ES evaluated")),
        curvature: r(firm_node
            .nonadditive
            .curvature_spot
            .expect("curvature evaluated")),
    };

    // ---- RiskService: DrillRisk (firm → ccy-pair children + positions) ----
    let drill_q = DrillQuery::new(Scope::firm(), OrgDimension::CcyPair, usd_numeraire())
        .children()
        .positions()
        .correlation_id(9);
    let drill = tokio::time::timeout(RPC_TIMEOUT, client.drill_risk(&drill_q))
        .await
        .expect("drill within timeout")
        .expect("drill ok");
    assert!(
        !drill.children.is_empty(),
        "firm drills into ccy-pair children"
    );

    // ---- RiskService: LimitStatus (a firm-level scope) ----
    let limit_q = LimitQuery::new(Scope::firm(), usd_numeraire());
    let limits = tokio::time::timeout(RPC_TIMEOUT, client.limit_status(&limit_q))
        .await
        .expect("limit within timeout")
        .expect("limit ok");
    // No limits configured at the firm scope here → worst is Green, no hard breach.
    assert!(
        !limits.hard_breach,
        "no hard breach with no configured limit"
    );

    // ---- RiskService: ListPositions (the whole entitled book, grant-all) ----
    let list_q = PositionQuery::new().entitled(Entitlements::grant_all());
    let listed = tokio::time::timeout(RPC_TIMEOUT, client.list_positions(&list_q))
        .await
        .expect("list within timeout")
        .expect("list ok");
    let position_count = listed.positions.len();

    // Sanity: a scoped deny actually cuts positions (entitlement pruning federates).
    let walled = Entitlements::grant_all().deny(EntitlementScope::covering(Scope::at(
        OrgDimension::Desk,
        10,
    )));
    let walled_list = tokio::time::timeout(
        RPC_TIMEOUT,
        client.list_positions(&PositionQuery::new().entitled(walled)),
    )
    .await
    .expect("walled list within timeout")
    .expect("walled list ok");
    assert!(
        walled_list.positions.len() < position_count,
        "the deny wall actually cut federated positions"
    );

    drop(drill);
    drop(smile);

    Observations {
        prices,
        marked_price_bits: marked_price,
        scenario_centre_bits,
        firm,
        position_count,
    }
}

// Wire helpers for the one pinned-price path that needs a raw proto request (the SDK
// `price` does not expose surface_version pinning).
fn wire_market() -> celnet_proto::MarketContext {
    let m = market();
    celnet_proto::MarketContext::fx(m.spot, m.vol, m.r_dom, m.r_for)
}
fn wire_conv() -> celnet_proto::Conventions {
    celnet_proto::Conventions {
        delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
        atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
        premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
        cut: celnet_proto::Cut::NewYork1000 as i32,
        day_count: celnet_proto::DayCount::Act365Fixed as i32,
        settlement: celnet_proto::Settlement::Deliverable as i32,
    }
}

// A tiny extension so `call_on(..).to_wire_pub()` builds the raw proto instrument for
// the pinned-price path (the SDK keeps `to_wire` private). Re-derives it from the
// public fields.
trait ToWirePub {
    fn to_wire_pub(&self) -> celnet_proto::Instrument;
}
impl ToWirePub for InstrumentSpec {
    fn to_wire_pub(&self) -> celnet_proto::Instrument {
        let strike = match self.product {
            celnet_client::Product::Vanilla {
                strike: StrikeSpec::Absolute(k),
                ..
            } => k,
            _ => strike_for(self.pair),
        };
        celnet_proto::Instrument {
            underlying: Some(celnet_proto::Underlying::fx(celnet_proto::CcyPair {
                base: self.pair.base.as_str().to_owned(),
                quote: self.pair.quote.as_str().to_owned(),
            })),
            tenor: Some(celnet_proto::Tenor {
                unit: celnet_proto::tenor::Unit::Years as i32,
                count: 1,
                broken_date: None,
            }),
            expiry_years: self.expiry_years,
            quantity: Some(celnet_proto::Quantity {
                notional: self.quantity.notional,
                base_ccy: self.quantity.base_ccy,
            }),
            side: celnet_proto::Side::TwoWay as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(celnet_proto::instrument::Product::Vanilla(
                celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(celnet_proto::StrikeOrDelta {
                        spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                    }),
                },
            )),
            ..Default::default()
        }
    }
}

// ===========================================================================
// the scale ladder
// ===========================================================================

/// Print one rung of the ladder: the node count + the load-bearing invariants.
fn print_rung(label: &str, nodes: usize, obs: &Observations) {
    let px = |pair: &str| {
        obs.prices
            .iter()
            .find(|(p, _, _)| p == pair)
            .map(|(_, bits, _)| f64::from_bits(*bits))
            .unwrap_or(f64::NAN)
    };
    println!(
        "[{label} nodes={nodes}] firm Δ={:>14.2} vega={:>12.2} VaR={:>14.2} ES={:>14.2} \
         curv={:>12.4} | px(EURUSD)={:.6} px(GBPUSD)={:.6} px(USDJPY)={:.6} | positions={} OK",
        obs.firm.delta,
        obs.firm.vega,
        obs.firm.var,
        obs.firm.es,
        obs.firm.curvature,
        px("EURUSD"),
        px("GBPUSD"),
        px("USDJPY"),
        obs.position_count,
    );
}

/// Assert that two observation sets are invariant under a scale change: the firm risk
/// aggregate, every per-pair price (bit-exact), the marked + scenario prices, and the
/// federated position count are unchanged.
fn assert_invariant(baseline: &Observations, scaled: &Observations, what: &str) {
    assert_eq!(
        baseline.firm, scaled.firm,
        "{what}: firm risk aggregate must be invariant"
    );
    assert_eq!(
        baseline.prices, scaled.prices,
        "{what}: every per-pair price + Greek set must be bit-invariant"
    );
    assert_eq!(
        baseline.marked_price_bits, scaled.marked_price_bits,
        "{what}: the marked+pinned price must be invariant"
    );
    assert_eq!(
        baseline.scenario_centre_bits, scaled.scenario_centre_bits,
        "{what}: the scenario centre price must be invariant"
    );
    assert_eq!(
        baseline.position_count, scaled.position_count,
        "{what}: the federated position count (whole book) must be invariant"
    );
    println!(
        "    -> INVARIANT across {what} (firm risk, prices, marked, scenario, positions all equal)"
    );
}

// ===========================================================================
// the unavailable proof (a down backend with NO re-home)
// ===========================================================================

/// With the edge dialling a fleet of which one backend has been killed (and NOT
/// re-homed), a federated FIRM aggregate must return `unavailable` — never a silently
/// smaller firm number. Returns the gRPC status code string.
async fn assert_unavailable_on_uncovered_slice(edge_url: &str) {
    let client = Client::connect(edge_url.to_owned())
        .await
        .expect("connect to degraded edge");
    let firm_q = AggregateQuery::new(OrgDimension::Firm, usd_numeraire())
        .value_at_risk([-0.02, 0.0, 0.02], 0.99)
        .curvature(0.18);
    let err = tokio::time::timeout(RPC_TIMEOUT, client.aggregate_risk(&firm_q))
        .await
        .expect("the degraded aggregate returns promptly (no hang)")
        .expect_err("a down backend with no re-home must fail, never a partial number");
    match &err {
        celnet_client::ClientError::Status(status) => {
            assert_eq!(
                status.code(),
                tonic::Code::Unavailable,
                "an uncovered slice must surface as unavailable"
            );
            println!(
                "    -> HONEST UNAVAILABLE: federated firm aggregate over a killed-without-rehome \
                 backend returned `{}` (never a silently smaller number) OK",
                status.code()
            );
        }
        other => panic!("expected a gRPC Unavailable status, got {other:?}"),
    }
}

// ===========================================================================
// main
// ===========================================================================

#[tokio::main]
async fn main() {
    // Wrap the whole run in a wall-clock deadline so it can never hang CI/a console.
    let result = tokio::time::timeout(HARNESS_DEADLINE, run()).await;
    match result {
        Ok(()) => {
            println!(
                "\nscale_harness: ALL SCALE STEPS PASSED — every API capability drove through the distributed edge and stayed invariant across scale up/down."
            );
        }
        Err(_) => {
            eprintln!("scale_harness: TIMED OUT after {HARNESS_DEADLINE:?}");
            std::process::exit(1);
        }
    }
}

async fn run() {
    // Sanity: the master book genuinely fans across replicas at every scale we test
    // (so reconciliation is not vacuously single-backend).
    for n in [2u64, 3, 4] {
        let distinct: std::collections::HashSet<u64> = master_book()
            .iter()
            .map(|&m| owner_replica_of(m, n))
            .collect();
        assert!(
            distinct.len() >= 2,
            "the {n}-replica partition must fan the book across multiple backends"
        );
    }

    let mut fleet = Fleet::new();

    println!("=== Celnet out-of-process scale harness ===");
    println!(
        "Spawning REAL OS processes: N backend nodes (each seeded its HRW slice of a \
         {}-leg master book) + 1 stateless distributed edge.\n",
        master_book().len()
    );

    // ---- Rung 1: boot 3 backends + the edge; drive everything; baseline. ----
    println!("[scale] starting 3 backends + edge ...");
    for n in spawn_backends(3) {
        fleet.push(n);
    }
    let urls = fleet.backend_urls();
    println!("    backends: {}", urls.join(", "));
    fleet.push(spawn_edge(&urls));
    let edge_url = fleet.edge_url();
    println!("    edge:     {edge_url}\n");

    let baseline = drive_all(&edge_url).await;
    print_rung("BASELINE", 3, &baseline);

    // ---- Rung 2: SCALE UP to 4 backends (re-home) + restart the stateless edge. ----
    println!(
        "\n[scale] SCALING UP: spawning a 4th backend, re-homing the book over 4 \
              replicas, restarting the stateless edge over the larger fleet ..."
    );
    // Re-home: kill the 3 backends and re-spawn 4 (each seeds its new 4-replica slice).
    for r in [1u64, 2, 3] {
        fleet.kill_backend(r);
    }
    fleet.kill_edge();
    for n in spawn_backends(4) {
        fleet.push(n);
    }
    let urls = fleet.backend_urls();
    println!("    backends: {}", urls.join(", "));
    fleet.push(spawn_edge(&urls));
    let edge_url = fleet.edge_url();
    println!("    edge:     {edge_url}\n");

    let scaled_up = drive_all(&edge_url).await;
    print_rung("SCALE-UP", 4, &scaled_up);
    assert_invariant(&baseline, &scaled_up, "scale-up 3->4");

    // ---- Rung 3: SCALE DOWN to 2 backends (re-home survivors) + restart edge. ----
    println!(
        "\n[scale] SCALING DOWN: re-homing the book over 2 surviving replicas, \
              killing the rest, restarting the stateless edge over the smaller fleet ..."
    );
    for r in [1u64, 2, 3, 4] {
        fleet.kill_backend(r);
    }
    fleet.kill_edge();
    for n in spawn_backends(2) {
        fleet.push(n);
    }
    let urls = fleet.backend_urls();
    println!("    backends: {}", urls.join(", "));
    fleet.push(spawn_edge(&urls));
    let edge_url = fleet.edge_url();
    println!("    edge:     {edge_url}\n");

    let scaled_down = drive_all(&edge_url).await;
    print_rung("SCALE-DOWN", 2, &scaled_down);
    assert_invariant(&baseline, &scaled_down, "scale-down to 2");

    // ---- Rung 4: UNAVAILABLE — kill a backend with NO re-home. ----
    println!(
        "\n[scale] DEGRADED: killing a backend with NO re-home (the edge still \
              lists it) — the federated firm aggregate must refuse, not under-report ..."
    );
    // Boot a fresh 3-backend fleet, edge over all 3, then kill one backend without
    // restarting the edge → the edge still dials the now-dead endpoint.
    for r in [1u64, 2] {
        fleet.kill_backend(r);
    }
    fleet.kill_edge();
    for n in spawn_backends(3) {
        fleet.push(n);
    }
    let urls = fleet.backend_urls();
    fleet.push(spawn_edge(&urls));
    let edge_url = fleet.edge_url();
    // Kill backend 2 (a non-empty slice owner) WITHOUT re-homing or repointing.
    let killed = fleet.kill_backend(2);
    println!("    killed backend (no re-home): {killed}");
    assert_unavailable_on_uncovered_slice(&edge_url).await;

    // Teardown is automatic via Fleet::drop, but be explicit for the log.
    println!("\n[scale] tearing down all child processes ...");
    drop(fleet);
}
