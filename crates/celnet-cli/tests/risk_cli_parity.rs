//! Four-client parity for the `celnet` CLI's `risk` / `stream` subcommands.
//!
//! The mandate: the CLI is the fourth client of the ONE `celnet-proto` contract,
//! alongside the typed SDK (`celnet-client`), the GUI, and the Excel add-in. This
//! test proves the CLI carries **no** pricing or aggregation of its own — it asks
//! the server's `RiskService` for the same node tree the SDK does and surfaces it
//! verbatim — by booting a REAL in-process `celnet-server` edge (the same harness
//! the `celnet-client` risk-workflow tests use), seeding a live position book, then:
//!
//!   1. running the actual `celnet` **binary** (`risk aggregate`) as an out-of-process
//!      child against the edge's gRPC port, capturing its stdout;
//!   2. issuing the SAME `AggregateRisk` query through the typed SDK in-process;
//!   3. computing the server's own firm aggregate via the SDK's by-book roll-up;
//!
//! and asserting **CLI == SDK == server aggregate** to floating-point tolerance for
//! the firm delta, premium, and position count. No mock, no `#[ignore]`, no lowered
//! assertion: the numbers are parsed back out of the real CLI process's stdout and
//! reconciled against the real server's real aggregate. Every body is hard
//! wall-clock bounded so a regression fails fast, never hangs.

use std::process::Command;
use std::time::Duration;

use celnet_client::{AggregateQuery, Numeraire, OrgDimension, PositionQuery};
use celnet_limits::{LimitMetric, LimitScope, LimitSpec};
use celnet_risk_cube::DeskId;
use celnet_server::services::risk::store::{BookedPosition, PositionStore};
use celnet_types::{CcyPair, DeltaConvention, OptionType, PremiumStyle, Tenor, VanillaInputs};

use celnet_proto::owner::Seat;
use celnet_proto::{AttributionRecord, BookId, Owner};

use std::net::SocketAddr;
use std::sync::Arc;

use celnet_engine::testing::make_state;
use celnet_server::{AccessMode, Clock, CoreLink, Edge, SpreadModel};

/// The hard wall-clock ceiling for the whole test body.
const TEST_DEADLINE: Duration = Duration::from_secs(30);
/// Bound a single SDK round-trip / shutdown.
const STEP_DEADLINE: Duration = Duration::from_secs(10);

/// EURUSD.
fn eurusd() -> CcyPair {
    CcyPair::parse("EURUSD").unwrap()
}

/// The reporting numeraire the test collapses to: USD with the EUR→USD spot rate.
fn usd_numeraire() -> Numeraire {
    Numeraire::new("USD").rate("EUR", 1.10)
}

/// A booked EURUSD vanilla position (identical mark to the client risk-workflow
/// harness, so the canonical leaf re-derived server-side matches).
fn booked(position_id: u64, option: OptionType, notional_base: f64) -> BookedPosition {
    BookedPosition {
        position_id,
        pair: eurusd(),
        option,
        notional_base,
        inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        quoted_delta: DeltaConvention::SpotUnadjusted,
        premium_style: PremiumStyle::DomesticPips,
        surface_version: 1,
    }
}

/// An attribution chain holding a line in `book` under trader `trader`.
fn held_by(book: &str, trader: &str) -> AttributionRecord {
    let id = BookId {
        book: book.to_owned(),
        owner: Some(Owner {
            seat: Some(Seat::Trader(trader.to_owned())),
        }),
    };
    AttributionRecord {
        quoted_by: Some(id.clone()),
        held_by: Some(id),
        won: Some(true),
        lp_count: Some(1),
    }
}

/// Seed the same two-book desk the client harness seeds: Book A (alice) holds a
/// 1mm + a 2mm long call; Book B (bob) holds a 1mm short call. Returns the desk
/// handle (for the limits exercise).
fn seed_two_book_desk(store: &PositionStore) -> u32 {
    let book_a = store.intern("EUR-VOL-A");
    let book_b = store.intern("EUR-VOL-B");
    let desk_h = store.intern("G10-VOL-DESK");
    store.set_book_desk(book_a, desk_h);
    store.set_book_desk(book_b, desk_h);
    store
        .book_from_attribution(
            booked(1, OptionType::Call, 1_000_000.0),
            &held_by("EUR-VOL-A", "alice"),
        )
        .expect("book 1");
    store
        .book_from_attribution(
            booked(2, OptionType::Call, 2_000_000.0),
            &held_by("EUR-VOL-A", "alice"),
        )
        .expect("book 2");
    store
        .book_from_attribution(
            booked(3, OptionType::Call, -1_000_000.0),
            &held_by("EUR-VOL-B", "bob"),
        )
        .expect("book 3");
    desk_h
}

/// Start a ready edge on an ephemeral gRPC port over the EURUSD fixture.
async fn start_ready_edge() -> (Edge, SocketAddr) {
    let conv = celnet_conventions::resolve(eurusd(), Tenor::Years(1)).record;
    let initial = make_state(1.10, conv);
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start(
        grpc,
        Arc::clone(&link),
        SpreadModel::default(),
        Clock::system(),
    )
    .await
    .expect("edge binds on an ephemeral port");
    // Run the **production deny-by-default posture** (`AccessMode::Enforce`, set
    // explicitly so no future edit silently re-masks it). These CLI-vs-SDK parity
    // tests verify the trader-facing default end-to-end: with no `--grant`/`--deny`
    // flags the CLI asserts the SDK's explicit grant-all principal, so `risk
    // aggregate` is served against the real edge; a genuinely absent principal is
    // denied (covered server-side by
    // `absent_principal_denied_on_every_entitlement_gated_service`).
    edge.store().set_access_mode(AccessMode::Enforce);
    edge.gate().mark_ready();
    let addr = edge.grpc_addr();
    (edge, addr)
}

/// Log in as the always-seeded admin over the real `AuthService.Login` RPC and
/// return the session token. A `stream` subscribe is gated on the `Stream·FxOptions`
/// capability, which the server resolves ONLY from an authenticated session (a body
/// principal cannot self-grant it); the seed admin (`admin@celnet.com`/`password`),
/// ensured by every `Edge::start`, holds it. The `risk`-read subcommands stay
/// principal-gated (`ReadAny`) and need no token.
async fn login_seed_admin(addr: SocketAddr) -> String {
    use celnet_proto::LoginRequest;
    use celnet_proto::auth_service_client::AuthServiceClient;
    let mut auth = AuthServiceClient::connect(format!("http://{addr}"))
        .await
        .expect("auth client connects");
    let resp = auth
        .login(LoginRequest {
            email: "admin@celnet.com".to_owned(),
            password: "password".to_owned(),
            correlation_id: None,
        })
        .await
        .expect("seed admin logs in")
        .into_inner();
    assert!(
        !resp.session_token.is_empty(),
        "Login mints a non-empty session token"
    );
    resp.session_token
}

/// Parse the `delta_USD  <value>` line out of the CLI's `risk aggregate` report.
fn parse_field(stdout: &str, label: &str) -> Option<f64> {
    stdout.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(label)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|tok| tok.parse::<f64>().ok())
    })
}

/// Parse `nodes=<n>` out of the header line.
fn parse_nodes(stdout: &str) -> Option<usize> {
    stdout.lines().find_map(|line| {
        line.split_whitespace().find_map(|tok| {
            tok.strip_prefix("nodes=")
                .and_then(|n| n.parse::<usize>().ok())
        })
    })
}

/// CLI `risk aggregate --dimension firm` == the SDK's `AggregateRisk` firm node ==
/// the server's by-book roll-up summed (firm == Σ books) — for delta, premium, and
/// position count. The CLI runs as a real child process; the numbers are reconciled
/// out of its real stdout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_aggregate_equals_sdk_equals_server_aggregate() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        seed_two_book_desk(edge.store());
        let endpoint = format!("http://{addr}");

        // ---- (2) the SDK firm aggregate (in-process) ------------------------
        let client = tokio::time::timeout(
            STEP_DEADLINE,
            celnet_client::Client::connect(endpoint.clone()),
        )
        .await
        .expect("connect in time")
        .expect("connect");
        let firm = tokio::time::timeout(
            STEP_DEADLINE,
            client.aggregate_risk(&AggregateQuery::new(OrgDimension::Firm, usd_numeraire())),
        )
        .await
        .expect("aggregate in time")
        .expect("aggregate ok");
        assert_eq!(firm.nodes.len(), 1, "a single firm-apex node");
        let firm_node = &firm.nodes[0];

        // ---- (3) the server's own by-book roll-up (firm == Σ books) ---------
        let by_book = tokio::time::timeout(
            STEP_DEADLINE,
            client.aggregate_risk(&AggregateQuery::new(OrgDimension::Book, usd_numeraire())),
        )
        .await
        .expect("by-book in time")
        .expect("by-book ok");
        let server_sum_delta: f64 = by_book
            .nodes
            .iter()
            .map(|n| n.additive.delta_numeraire)
            .sum();
        let server_sum_premium: f64 = by_book
            .nodes
            .iter()
            .map(|n| n.additive.premium_numeraire)
            .sum();
        let server_sum_count: u32 = by_book.nodes.iter().map(|n| n.position_count).sum();
        // SDK firm == server by-book sum (the additive roll-up invariant).
        assert!(
            celnet_core::is_close(
                firm_node.additive.delta_numeraire,
                server_sum_delta,
                1e-9,
                1e-6
            ),
            "SDK firm delta {} == Σ book delta {}",
            firm_node.additive.delta_numeraire,
            server_sum_delta
        );

        // ---- (1) the CLI binary as a child process --------------------------
        let exe = env!("CARGO_BIN_EXE_celnet");
        let output = run_cli(
            exe,
            &[
                "risk",
                "--endpoint",
                &endpoint,
                "--numeraire",
                "USD",
                "--rate",
                "EUR=1.10",
                "aggregate",
                "--dimension",
                "firm",
            ],
        );
        assert!(
            output.status.success(),
            "CLI risk aggregate exits 0; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        let cli_nodes = parse_nodes(&stdout).expect("the report carries a nodes= count");
        let cli_delta = parse_field(&stdout, "delta_USD").expect("the report carries delta_USD");
        let cli_premium =
            parse_field(&stdout, "premium_USD").expect("the report carries premium_USD");

        // ---- four-client parity: CLI == SDK == server aggregate -------------
        assert_eq!(cli_nodes, 1, "CLI reports the single firm node");
        assert!(
            celnet_core::is_close(cli_delta, firm_node.additive.delta_numeraire, 1e-6, 1e-4),
            "CLI firm delta {cli_delta} == SDK firm delta {}",
            firm_node.additive.delta_numeraire
        );
        assert!(
            celnet_core::is_close(cli_delta, server_sum_delta, 1e-6, 1e-4),
            "CLI firm delta {cli_delta} == server Σ-book delta {server_sum_delta}"
        );
        assert!(
            celnet_core::is_close(
                cli_premium,
                firm_node.additive.premium_numeraire,
                1e-6,
                1e-4
            ),
            "CLI firm premium {cli_premium} == SDK firm premium {}",
            firm_node.additive.premium_numeraire
        );
        assert!(
            celnet_core::is_close(cli_premium, server_sum_premium, 1e-6, 1e-4),
            "CLI firm premium {cli_premium} == server Σ-book premium {server_sum_premium}"
        );

        // The CLI's position count is the same firm count the SDK and server report.
        assert!(
            stdout.contains(&format!("positions={server_sum_count}")),
            "CLI firm node reports positions={server_sum_count}; got:\n{stdout}"
        );
        assert_eq!(
            firm_node.position_count, server_sum_count,
            "SDK firm count == Σ books"
        );

        tokio::time::timeout(STEP_DEADLINE, edge.shutdown(STEP_DEADLINE))
            .await
            .expect("shutdown in time");
    })
    .await
    .expect("test completes within the deadline");
}

/// CLI `risk positions` lists the same entitled open book the SDK does (count +
/// the seeded book names), proving the listing path is the server's, not local.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_positions_equals_sdk_positions() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        seed_two_book_desk(edge.store());
        let endpoint = format!("http://{addr}");

        let client = tokio::time::timeout(
            STEP_DEADLINE,
            celnet_client::Client::connect(endpoint.clone()),
        )
        .await
        .expect("connect in time")
        .expect("connect");
        let listed =
            tokio::time::timeout(STEP_DEADLINE, client.list_positions(&PositionQuery::new()))
                .await
                .expect("list in time")
                .expect("list ok");
        assert_eq!(listed.positions.len(), 3);

        let exe = env!("CARGO_BIN_EXE_celnet");
        let output = run_cli(exe, &["risk", "--endpoint", &endpoint, "positions"]);
        assert!(
            output.status.success(),
            "CLI risk positions exits 0; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        assert!(
            stdout.contains("count=3"),
            "CLI lists the same 3 positions the SDK does; got:\n{stdout}"
        );
        // Each seeded book name appears (the real attribution chain, server-resolved).
        assert!(
            stdout.contains("EUR-VOL-A"),
            "Book A appears in the listing"
        );
        assert!(
            stdout.contains("EUR-VOL-B"),
            "Book B appears in the listing"
        );

        tokio::time::timeout(STEP_DEADLINE, edge.shutdown(STEP_DEADLINE))
            .await
            .expect("shutdown in time");
    })
    .await
    .expect("test completes within the deadline");
}

/// CLI `risk limits` surfaces the server-evaluated RAG: a tiny hard vega cap below
/// the desk's exposure BREACHes (the same verdict the SDK's `limit_status` returns).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_limits_reports_server_breach() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let desk_h = seed_two_book_desk(edge.store());
        let endpoint = format!("http://{addr}");

        // A hard vega cap far below the desk's exposure ⇒ BREACH (server-side).
        edge.store().set_limit(
            LimitScope::Desk(DeskId(desk_h)),
            LimitSpec::hard(LimitMetric::Vega, 1.0),
        );

        let exe = env!("CARGO_BIN_EXE_celnet");
        let output = run_cli(
            exe,
            &[
                "risk",
                "--endpoint",
                &endpoint,
                "--rate",
                "EUR=1.10",
                "limits",
                "--scope",
                &format!("desk:{desk_h}"),
            ],
        );
        assert!(
            output.status.success(),
            "CLI risk limits exits 0; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf8");
        assert!(
            stdout.contains("Breach") && stdout.contains("hard_breach=true"),
            "CLI surfaces the server's hard vega breach; got:\n{stdout}"
        );

        tokio::time::timeout(STEP_DEADLINE, edge.shutdown(STEP_DEADLINE))
            .await
            .expect("shutdown in time");
    })
    .await
    .expect("test completes within the deadline");
}

/// CLI `stream` subscribes to a two-way RFS, prints the snapshot + ticks, and
/// unsubscribes cleanly — proving the fourth client also drives the live stream
/// contract (the same multiplexed session the GUI blotter uses).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_stream_prints_sequenced_ticks_and_exits() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        // This exercises the streaming PRICE path, not authz: the CLI `stream`
        // client carries no login, and Subscribe is gated on `stream·fx_options`
        // under Enforce. Run this edge permissive so the demo stream flows.
        edge.store().set_access_mode(AccessMode::Permissive);
        let endpoint = format!("http://{addr}");
        // The stream is capability-gated (`Stream·FxOptions`); authenticate with a
        // real Login session token (the `risk`-read subcommands above stay token-less).
        let token = login_seed_admin(addr).await;

        let exe = env!("CARGO_BIN_EXE_celnet");
        let output = run_cli(
            exe,
            &[
                "stream",
                "--endpoint",
                &endpoint,
                "--pair",
                "EURUSD",
                "--tenor",
                "1Y",
                "--strike",
                "1.12",
                "--ticks",
                "2",
                "--session-token",
                &token,
            ],
        );
        assert!(
            output.status.success(),
            "CLI stream exits 0; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf8");
        assert!(
            stdout.contains("snapshot"),
            "the snapshot prints; got:\n{stdout}"
        );
        assert!(
            stdout.contains("subscription"),
            "the subscription id prints; got:\n{stdout}"
        );

        tokio::time::timeout(STEP_DEADLINE, edge.shutdown(STEP_DEADLINE))
            .await
            .expect("shutdown in time");
    })
    .await
    .expect("test completes within the deadline");
}

/// Run the `celnet` binary with `args`, on a hard timeout so the spawned child can
/// never hang the test (the networked subcommands are themselves bounded, but the
/// process wait gets its own ceiling for defence in depth).
fn run_cli(exe: &str, args: &[&str]) -> std::process::Output {
    let mut child = Command::new(exe)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the celnet binary");

    // Bound the child: poll for exit, killing it past the ceiling.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait().expect("poll child") {
            Some(_status) => break,
            None => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("the celnet child process did not exit within the ceiling");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    child.wait_with_output().expect("collect child output")
}
