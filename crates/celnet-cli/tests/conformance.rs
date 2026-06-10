//! Cross-client conformance gate — the **CLI** half of the executable oracle.
//!
//! The `celnet` CLI is the third local-compute client of the platform's analytics
//! (it prices vanillas/exotics/baskets directly via the same `celnet-exotics` /
//! `celnet-vanilla` libraries the server uses). This test drives the **real CLI
//! binary** out-of-process for the local-compute families it supports, parses the
//! price (or fair strike / std-error) out of its stdout, and asserts it matches
//! the frozen golden-vector corpus' independent-oracle `expected` — within the
//! vector's tolerance for closed-form families, or within `k · stderr` for the
//! Monte-Carlo families.
//!
//! Scope: the CLI's local-compute surface — `price` (vanilla), `exotic`
//! (digital / one-touch / single-barrier / var-swap / vol-swap / asian /
//! forward-start / quanto / cliquet / tarf / accumulator / lookback / american),
//! `basket`, and the linear book `forward` / `swap` / `ndf`. Networked subcommands
//! are out of scope for the vector corpus: `risk` / `stream` are gated by the
//! four-client parity test, and the `rfq` multi-dealer panel is gated below
//! against an in-process multi-dealer edge (CLI ladder == SDK panel, bit for
//! bit). Every family covered here is asserted reachable through the CLI.

use std::process::Command;

use celnet_golden::{GoldenVector, load_vectors};

/// The standard-error band multiplier for the Monte-Carlo families (mirrors the
/// SDK conformance harness).
const K_STDERR: f64 = 4.0;

/// The families the CLI prices locally (and therefore this test covers). The other
/// oneof arms (strategy, double-barrier, touch corridors, window-barrier) either
/// have no single-flag CLI surface or are LSV-only; they are gated by the SDK
/// conformance harness, which exercises every one of the 21 families.
const CLI_FAMILIES: [&str; 12] = [
    "vanilla",
    "digital",
    "touch", // only the single ONE_TOUCH (the CLI's `one-touch`)
    "single_barrier",
    "variance_swap",
    "volatility_swap",
    "forward_start",
    "quanto",
    "american",
    "fx_forward",
    "fx_swap",
    "ndf",
];

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_celnet")
}

/// Run the CLI with `args`, returning its stdout (panicking on a non-zero exit).
fn run(args: &[String]) -> String {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn celnet CLI");
    assert!(
        out.status.success(),
        "CLI failed for args {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("CLI stdout is UTF-8")
}

/// Parse a labelled numeric line (`  <label>   <value>`) out of the CLI report.
fn field(stdout: &str, label: &str) -> Option<f64> {
    stdout.lines().find_map(|line| {
        let t = line.trim();
        t.strip_prefix(label)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|tok| tok.parse::<f64>().ok())
    })
}

/// The CLI's headline price for a report: `price`, or the swap `fair_vol` /
/// `fair_variance`.
fn cli_price(stdout: &str) -> f64 {
    field(stdout, "price")
        .or_else(|| field(stdout, "fair_variance"))
        .or_else(|| field(stdout, "fair_vol"))
        .unwrap_or_else(|| panic!("no price line in CLI output:\n{stdout}"))
}

fn s(x: f64) -> String {
    // Full-precision so the CLI receives the exact corpus market.
    format!("{x:?}")
}

/// The shared `--spot --vol --t --r-dom --r-for [--strike]` market flags.
fn market_flags(v: &GoldenVector, strike: Option<f64>) -> Vec<String> {
    let m = &v.market;
    let t = v.term_f64("expiry_years");
    let mut a = vec![
        "--spot".into(),
        s(m.spot),
        "--vol".into(),
        s(m.vol),
        "--t".into(),
        s(t),
        "--r-dom".into(),
        s(m.r_dom),
        "--r-for".into(),
        s(m.r_for),
    ];
    if let Some(k) = strike {
        a.push("--strike".into());
        a.push(s(k));
    }
    a
}

/// The `--spot --t --r-dom --r-for` market flags for the linear book (no `--vol`:
/// a linear DCF has no volatility input). `--t` is the settlement / far-leg tenor.
fn linear_market_flags(v: &GoldenVector) -> Vec<String> {
    let m = &v.market;
    vec![
        "--spot".into(),
        s(m.spot),
        "--t".into(),
        s(v.term_f64("expiry_years")),
        "--r-dom".into(),
        s(m.r_dom),
        "--r-for".into(),
        s(m.r_for),
    ]
}

/// The CLI `--side`/`--near-side` token (`buy`/`sell`) for a linear vector field.
fn side_token(v: &GoldenVector, key: &str) -> &'static str {
    match v.term_str(key) {
        "BUY" => "buy",
        "SELL" => "sell",
        other => panic!("unknown side `{other}`"),
    }
}

/// The CLI `--fixing` value-enum token for an NDF vector's fixing identity (the
/// vector string matches the `celnet_types::FixingSource` variant name; clap's
/// value-enum spells it kebab-case).
fn fixing_token(v: &GoldenVector) -> &'static str {
    match v.term_str("fixing") {
        "KrwKftc18" => "krw-kftc18",
        "TwdTaipei" => "twd-taipei",
        "InrRbiRef" => "inr-rbi-ref",
        "BrlPtax" => "brl-ptax",
        "ClpDolarObs" => "clp-dolar-obs",
        "CopTrm" => "cop-trm",
        other => panic!("unknown fixing `{other}`"),
    }
}

fn opt_token(v: &GoldenVector, key: &str) -> &'static str {
    match v.term_str(key) {
        "CALL" => "call",
        "PUT" => "put",
        other => panic!("unknown option type `{other}`"),
    }
}

/// Build the CLI argv for a vector, or `None` if this vector is not a CLI-covered
/// shape (e.g. a double / corridor touch, which the `exotic` subcommand prices
/// only via the SDK path).
fn argv_for(v: &GoldenVector) -> Option<Vec<String>> {
    match v.family.as_str() {
        "vanilla" => {
            let mut a = vec![
                "price".into(),
                "--option".into(),
                opt_token(v, "option_type").into(),
            ];
            a.extend(market_flags(v, None));
            a.push("--strike".into());
            a.push(s(v.term_f64("strike")));
            Some(a)
        }
        "digital" => {
            // The CLI digital pays one unit of domestic cash; corpus digital
            // vectors are all payout = 1.0, so they line up.
            let kind = match v.term_str("option_type") {
                "CALL" => "digital-call",
                _ => "digital-put",
            };
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, Some(v.term_f64("strike"))));
            a.extend(["digital".into(), "--kind".into(), kind.into()]);
            Some(a)
        }
        "touch" => {
            // Only the single one-touch maps to the CLI `one-touch` (at-hit by
            // default — matching the corpus at-hit oracle). The corridor touches
            // are not exposed as a single CLI shape here.
            if v.term_str("kind") != "ONE_TOUCH" {
                return None;
            }
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, None));
            a.extend([
                "one-touch".into(),
                "--barrier".into(),
                s(v.term_f64("lower_barrier")),
                "--rebate".into(),
                s(v.term_f64("rebate")),
            ]);
            Some(a)
        }
        "single_barrier" => {
            // The CLI single-barrier covers all four knock topologies.
            let topology = match (v.term_str("kind"), v.term_str("side")) {
                ("KNOCK_OUT", "LOWER") => "down-and-out",
                ("KNOCK_OUT", "UPPER") => "up-and-out",
                ("KNOCK_IN", "LOWER") => "down-and-in",
                ("KNOCK_IN", "UPPER") => "up-and-in",
                _ => return None,
            };
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, Some(v.term_f64("strike"))));
            a.extend([
                "barrier".into(),
                "--option".into(),
                opt_token(v, "option_type").into(),
                "--topology".into(),
                topology.into(),
                "--barrier".into(),
                s(v.term_f64("barrier")),
                "--rebate".into(),
                s(v.term_f64("rebate")),
            ]);
            Some(a)
        }
        "variance_swap" => {
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, None));
            a.push("var-swap".into());
            Some(a)
        }
        "volatility_swap" => {
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, None));
            a.push("vol-swap".into());
            Some(a)
        }
        "forward_start" => {
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, None));
            a.extend([
                "forward-start".into(),
                "--option".into(),
                opt_token(v, "option_type").into(),
                "--moneyness".into(),
                s(v.term_f64("moneyness")),
                "--reset".into(),
                s(v.term_f64("reset")),
            ]);
            Some(a)
        }
        "quanto" => {
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, Some(v.term_f64("strike"))));
            a.extend([
                "quanto".into(),
                "--option".into(),
                opt_token(v, "option_type").into(),
                "--conversion-vol".into(),
                s(v.term_f64("conversion_vol")),
                // `=` form so a negative correlation is not parsed as a flag.
                format!("--correlation={}", s(v.term_f64("correlation"))),
            ]);
            if v.term_str("payoff") == "DIGITAL" {
                a.push("--digital".into());
            }
            Some(a)
        }
        "american" => {
            // FD engine (lsm_paths == 0). The CLI `american` exotic with no
            // bermudan/LSM flags prices on the projected-SOR free-boundary FD grid.
            if v.term_u64("lsm_paths") != 0 {
                return None;
            }
            let mut a = vec!["exotic".into()];
            a.extend(market_flags(v, Some(v.term_f64("strike"))));
            a.extend([
                "american".into(),
                "--option".into(),
                opt_token(v, "option_type").into(),
                "--strike".into(),
                s(v.term_f64("strike")),
            ]);
            Some(a)
        }
        "fx_forward" => {
            let mut a = vec!["forward".into(), "--pair".into(), v.underlying.clone()];
            a.extend(linear_market_flags(v));
            a.extend([
                "--rate".into(),
                s(v.term_f64("contract_rate")),
                "--notional".into(),
                s(v.term_f64("notional")),
                "--side".into(),
                side_token(v, "side").into(),
            ]);
            Some(a)
        }
        "fx_swap" => {
            let mut a = vec!["swap".into(), "--pair".into(), v.underlying.clone()];
            a.extend(linear_market_flags(v));
            a.extend([
                "--rate".into(),
                s(v.term_f64("contract_rate")),
                "--notional".into(),
                s(v.term_f64("notional")),
                "--near-side".into(),
                side_token(v, "near_side").into(),
            ]);
            Some(a)
        }
        "ndf" => {
            let mut a = vec!["ndf".into(), "--pair".into(), v.underlying.clone()];
            a.extend(linear_market_flags(v));
            a.extend([
                "--rate".into(),
                s(v.term_f64("contract_rate")),
                "--notional".into(),
                s(v.term_f64("notional")),
                "--side".into(),
                side_token(v, "side").into(),
                "--fixing".into(),
                fixing_token(v).into(),
            ]);
            Some(a)
        }
        _ => None,
    }
}

/// Assert a CLI-priced vector against its independent-oracle expectation.
fn assert_cli(v: &GoldenVector, got: f64) {
    let want = v.expected.price;
    if let Some(oracle_se) = v.expected.price_std_error {
        // CLI MC families would report their own std-error too, but the families
        // covered here are all closed-form on the CLI; keep the band for safety.
        let band = K_STDERR * oracle_se.max(1e-12);
        assert!(
            (got - want).abs() <= band,
            "CLI MC vector {} : price {got} vs oracle {want} band {band:e}",
            v.id
        );
    } else {
        let scale = got.abs().max(want.abs());
        assert!(
            (got - want).abs() <= v.tolerance.abs + v.tolerance.rel * scale,
            "CLI vector {} : price {got} vs oracle {want} (rel {}, abs {})",
            v.id,
            v.tolerance.rel,
            v.tolerance.abs
        );
    }
}

#[test]
fn cli_prices_match_the_golden_corpus() {
    let vectors = load_vectors().expect("golden corpus loads");
    let mut exercised = std::collections::HashSet::new();
    let mut count = 0usize;
    for v in &vectors {
        let Some(argv) = argv_for(v) else { continue };
        let stdout = run(&argv);
        let got = cli_price(&stdout);
        assert_cli(v, got);
        exercised.insert(v.family.clone());
        count += 1;
    }
    assert!(
        count >= CLI_FAMILIES.len(),
        "too few CLI vectors exercised: {count}"
    );
    for fam in CLI_FAMILIES {
        assert!(
            exercised.contains(fam),
            "CLI family `{fam}` was never exercised against the corpus"
        );
    }
}

// ---- Cross-asset vanilla: CLI == server == oracle --------------------------
//
// The golden corpus is FX-only, so the cross-asset families (equity / commodity /
// crypto) are reconciled here as a dedicated three-way gate rather than off the
// corpus: for each asset class we (1) run the real `celnet` binary's `price`
// command with `--asset <class>`, parsing its headline price; (2) compute the
// INDEPENDENT generalized-BSM / Garman-Kohlhagen closed form via `celnet-vanilla`
// (the oracle — a code path the CLI/server share but compute separately here); and
// (3) price the SAME instrument through the typed SDK against a real in-process
// `celnet-server` edge. We assert CLI == server == oracle. The vanilla payoff is
// the asset-class-agnostic closed form over the carry-producing market (ADR-0008),
// so the underlying is contract identity only and every class matches the FX price
// on the same market — which the CLI test also asserts.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_client::{
    Ccy, Client, Conventions, InstrumentSpec, MarketContext, Quantity, Side, StrikeSpec,
};
use celnet_engine::testing::make_state;
use celnet_server::{Clock, CoreLink, Edge, LpPanelConfig, SpreadModel};
use celnet_types::{CcyPair, OptionType, Tenor, VanillaInputs};

/// Boot a ready in-process edge on an ephemeral port over the EURUSD fixture.
async fn start_ready_edge() -> (Edge, SocketAddr) {
    let eurusd = CcyPair::parse("EURUSD").unwrap();
    let conv = celnet_conventions::resolve(eurusd, Tenor::Years(1)).record;
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
    edge.gate().mark_ready();
    let addr = edge.grpc_addr();
    (edge, addr)
}

#[tokio::test]
async fn cli_cross_asset_vanilla_equals_server_equals_oracle() {
    // A non-degenerate 1Y market shared across asset classes (full-precision flags).
    let (spot, strike, vol, t, r_dom, r_for) = (100.0_f64, 105.0, 0.22, 1.0, 0.03, 0.012);

    // The independent oracle: the generalized-BSM / GK closed form for this market.
    let oracle = celnet_vanilla::greeks(
        OptionType::Call,
        &VanillaInputs::new(spot, strike, vol, t, r_dom, r_for),
    )
    .price;

    let (edge, addr) = start_ready_edge().await;
    let client = Client::connect(format!("http://{addr}"))
        .await
        .expect("SDK connects to the edge");
    let conv = Conventions::major_default();
    let market = MarketContext {
        spot,
        vol,
        r_dom,
        r_for,
    };
    let tenor = Tenor::Years(1);
    let qty = Quantity::base(1.0);

    // (asset CLI token, the SDK InstrumentSpec for the same underlying).
    let cases: Vec<(&str, InstrumentSpec)> = vec![
        (
            "equity",
            InstrumentSpec::equity_vanilla(
                "AAPL",
                "XNAS",
                Ccy::USD,
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
        (
            "commodity",
            InstrumentSpec::commodity_vanilla(
                "BRENT",
                "",
                Ccy::USD,
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
        (
            "crypto",
            InstrumentSpec::crypto_vanilla(
                "BTC",
                "USDT",
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
    ];

    for (asset, spec) in &cases {
        // (1) The CLI binary's local-compute price with the cross-asset selector.
        let argv = vec![
            "price".to_string(),
            "--option".into(),
            "call".into(),
            "--asset".into(),
            (*asset).into(),
            "--spot".into(),
            s(spot),
            "--vol".into(),
            s(vol),
            "--t".into(),
            s(t),
            "--r-dom".into(),
            s(r_dom),
            "--r-for".into(),
            s(r_for),
            "--strike".into(),
            s(strike),
        ];
        let cli = cli_price(&run(&argv));

        // (2) The SAME instrument priced through the SDK against the real server.
        let server =
            tokio::time::timeout(Duration::from_secs(30), client.price(spec, market, conv))
                .await
                .unwrap_or_else(|_| panic!("{asset} server price timed out"))
                .unwrap_or_else(|e| panic!("{asset} server price failed: {e:?}"))
                .greeks
                .price;

        // CLI == oracle (closed form), and server == oracle — so CLI == server.
        assert!(
            (cli - oracle).abs() <= 1e-9,
            "{asset} CLI price {cli} vs independent oracle {oracle}"
        );
        assert!(
            (server - oracle).abs() <= 1e-9,
            "{asset} server price {server} vs independent oracle {oracle}"
        );
        assert!(
            (cli - server).abs() <= 1e-9,
            "{asset} CLI price {cli} vs server price {server}"
        );
    }

    drop(client);
    drop(edge);
}

// ---- Multi-dealer panel: CLI ladder == SDK panel, bit-identical -------------
//
// The `rfq` subcommand surfaces the server's ranked multi-dealer panel verbatim
// through the typed SDK, printing each price with shortest-round-trip precision.
// This gate boots ONE in-process edge with a deterministic synthetic LP panel
// (native maker + 3 labeled demo/test dealers — live LP connectivity is ENV,
// never claimed here), takes the SDK panel as the reference, then drives the real
// `celnet` binary against the same edge and asserts:
//
//  1. row-per-LP parity — every SDK dealer row prints exactly once;
//  2. the parsed bid/offer/strike columns equal the SDK panel's f64s **to the
//     bit** (the maker prices a pure function of the frozen engine state, so a
//     second RFQ over the same edge reproduces the rows bit-for-bit — the SDK's
//     own multi-dealer suite gates that same property between two SDK requests);
//  3. the `native` / `BEST_BID` / `BEST_OFFER` markers sit exactly where the SDK
//     panel puts them (greeks-bearing maker row / ranked winners);
//  4. every row prints a live last-look countdown at issue time; and
//  5. `--accept <lp_id>` books the pinned row: the printed `traded_premium`
//     equals that row's printed offer — and the SDK row's offer — bit-for-bit
//     (never a re-price), attributed to the winning LP.

/// One parsed `dealer` row of the CLI `rfq` ladder.
#[derive(Debug)]
struct CliPanelRow {
    lp_id: String,
    bid: f64,
    offer: f64,
    last_look: String,
    native: bool,
    best_bid: bool,
    best_offer: bool,
}

/// Parse the `dealer` rows out of the CLI `rfq` report
/// (`  dealer <lp_id>  bid <f64>  offer <f64>  last_look <window>  [markers…]`).
fn parse_panel_rows(stdout: &str) -> Vec<CliPanelRow> {
    stdout
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("dealer ")?;
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            let after = |label: &str| -> Option<&str> {
                let at = tokens.iter().position(|t| *t == label)?;
                tokens.get(at + 1).copied()
            };
            Some(CliPanelRow {
                lp_id: tokens.first()?.to_string(),
                bid: after("bid")?.parse().ok()?,
                offer: after("offer")?.parse().ok()?,
                last_look: after("last_look")?.to_owned(),
                native: tokens.contains(&"native"),
                best_bid: tokens.contains(&"BEST_BID"),
                best_offer: tokens.contains(&"BEST_OFFER"),
            })
        })
        .collect()
}

/// Boot a ready in-process edge on ephemeral ports with a deterministic
/// synthetic LP panel (native maker + `synthetic_lps` labeled demo/test dealers)
/// over the EURUSD fixture, using the explicit-panel boot path so no
/// process-global env is mutated.
async fn start_panel_edge(synthetic_lps: u32) -> (Edge, SocketAddr) {
    let eurusd = CcyPair::parse("EURUSD").unwrap();
    let conv = celnet_conventions::resolve(eurusd, Tenor::Years(1)).record;
    let initial = make_state(1.10, conv);
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start_on_with_panel(
        grpc,
        ws,
        Arc::clone(&link),
        SpreadModel::default(),
        Clock::system(),
        LpPanelConfig { synthetic_lps },
    )
    .await
    .expect("edge binds on ephemeral ports");
    edge.gate().mark_ready();
    let addr = edge.grpc_addr();
    (edge, addr)
}

#[tokio::test]
async fn cli_rfq_panel_matches_the_sdk_panel_bit_for_bit() {
    const SYNTHETIC_LPS: u32 = 3;
    let (edge, addr) = start_panel_edge(SYNTHETIC_LPS).await;

    // (1) The SDK reference panel from the edge — the same instrument the CLI
    // requests below (1Y EURUSD vanilla call @ 1.12, 1mm EUR, two-way).
    let client = Client::connect(format!("http://{addr}"))
        .await
        .expect("SDK connects to the edge");
    let instrument = InstrumentSpec::vanilla(
        CcyPair::parse("EURUSD").unwrap(),
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        OptionType::Call,
        StrikeSpec::Absolute(1.12),
    );
    let md = client.request_multi_dealer_quote(instrument, Conventions::major_default());
    let sdk = tokio::time::timeout(Duration::from_secs(30), md.request())
        .await
        .expect("SDK panel request in time")
        .expect("SDK panel request succeeds");
    assert_eq!(
        sdk.dealers.len(),
        1 + SYNTHETIC_LPS as usize,
        "native maker + {SYNTHETIC_LPS} synthetic demo dealers"
    );

    // (2) The CLI ladder from the SAME edge. The edge serves on THIS test's
    // runtime, so the blocking child-process wait runs on a blocking thread —
    // never on the reactor it is calling back into.
    let argv: Vec<String> = vec![
        "rfq".into(),
        "--endpoint".into(),
        format!("http://{addr}"),
        "--pair".into(),
        "EURUSD".into(),
        "--tenor".into(),
        "1Y".into(),
        "--expiry-years".into(),
        s(1.0),
        "--option".into(),
        "call".into(),
        "--strike".into(),
        s(1.12),
        "--notional".into(),
        s(1_000_000.0),
    ];
    let cli_argv = argv.clone();
    let stdout = tokio::task::spawn_blocking(move || run(&cli_argv))
        .await
        .expect("CLI run completes");
    let rows = parse_panel_rows(&stdout);

    // Row-per-LP parity, then per-row bit-identity + marker parity by lp_id.
    assert_eq!(
        rows.len(),
        sdk.dealers.len(),
        "one printed row per SDK dealer row:\n{stdout}"
    );
    let strike = field(&stdout, "line strike=").expect("the resolved line prints");
    for row in &rows {
        let sdk_row = sdk
            .dealer(&row.lp_id)
            .unwrap_or_else(|| panic!("CLI row `{}` is an SDK panel row", row.lp_id));
        assert_eq!(
            row.bid.to_bits(),
            sdk_row.price.bid.to_bits(),
            "{}: CLI bid {} == SDK bid {} bit-for-bit",
            row.lp_id,
            row.bid,
            sdk_row.price.bid
        );
        assert_eq!(
            row.offer.to_bits(),
            sdk_row.price.offer.to_bits(),
            "{}: CLI offer {} == SDK offer {} bit-for-bit",
            row.lp_id,
            row.offer,
            sdk_row.price.offer
        );
        assert_eq!(
            strike.to_bits(),
            sdk_row.resolved_strike.to_bits(),
            "{}: the printed line strike is the SDK resolved strike",
            row.lp_id
        );
        assert_eq!(
            row.native,
            sdk_row.greeks.is_some(),
            "{}: `native` marks exactly the greeks-bearing maker row",
            row.lp_id
        );
        assert_eq!(
            row.best_bid,
            sdk.best_bid_lp_id.as_deref() == Some(row.lp_id.as_str()),
            "{}: BEST_BID sits on the SDK panel's bid winner",
            row.lp_id
        );
        assert_eq!(
            row.best_offer,
            sdk.best_offer_lp_id.as_deref() == Some(row.lp_id.as_str()),
            "{}: BEST_OFFER sits on the SDK panel's offer winner",
            row.lp_id
        );
        // The 5s last-look window is open at issue time on a system clock, so
        // every row prints a live countdown, never `expired`.
        assert!(
            row.last_look.ends_with('s') && row.last_look != "expired",
            "{}: a live last-look countdown prints, got `{}`",
            row.lp_id,
            row.last_look
        );
    }
    assert_eq!(
        rows.iter().filter(|r| r.native).count(),
        1,
        "exactly one native maker row"
    );

    // (3) `--accept <lp_id>` books the pinned best-offer row: the printed
    // execution's premium equals the same invocation's printed offer — and the
    // SDK reference row's offer — bit-for-bit, attributed to the winning LP.
    let lp = sdk
        .best_offer_lp_id
        .clone()
        .expect("a best offer exists on a ≥3-LP panel");
    let mut accept_argv = argv.clone();
    accept_argv.extend(["--accept".into(), lp.clone(), "--side".into(), "buy".into()]);
    let booked = tokio::task::spawn_blocking(move || run(&accept_argv))
        .await
        .expect("CLI accept run completes");
    let premium = field(&booked, "traded_premium").expect("the execution prints its premium");
    let booked_row = parse_panel_rows(&booked)
        .into_iter()
        .find(|r| r.lp_id == lp)
        .expect("the booked LP is on the printed ladder");
    assert_eq!(
        premium.to_bits(),
        booked_row.offer.to_bits(),
        "BUY books the printed row's offer bit-for-bit: {premium} vs {}",
        booked_row.offer
    );
    assert_eq!(
        premium.to_bits(),
        sdk.dealer(&lp).expect("winner row").price.offer.to_bits(),
        "the booked premium reproduces the SDK reference row's offer"
    );
    assert!(
        booked.contains(&format!("quoted_by {lp}")),
        "the execution is attributed to the winning LP:\n{booked}"
    );

    drop(client);
    drop(edge);
}

#[test]
fn cli_rejects_inverse_coin_for_non_crypto() {
    // The inverse coin-margined settlement style is valid only for a digital asset;
    // requesting it for an equity must fail loudly (no silent wrong-label pricing).
    let out = Command::new(bin())
        .args([
            "price",
            "--option",
            "call",
            "--asset",
            "equity",
            "--settlement-style",
            "inverse-coin",
            "--spot",
            "100",
            "--vol",
            "0.2",
            "--t",
            "1.0",
            "--r-dom",
            "0.03",
            "--r-for",
            "0.0",
            "--strike",
            "105",
        ])
        .output()
        .expect("spawn celnet CLI");
    assert!(
        !out.status.success(),
        "inverse-coin on a non-crypto asset must be rejected"
    );
}
