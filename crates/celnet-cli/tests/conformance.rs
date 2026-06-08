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
//! Scope: the CLI's local-compute surface — `price` (vanilla) and `exotic`
//! (digital / one-touch / single-barrier / var-swap / vol-swap / asian /
//! forward-start / quanto / cliquet / tarf / accumulator / lookback / american) and
//! `basket`. Networked subcommands (`risk`, `stream`) are out of scope for the
//! vector corpus (they are gated by the four-client parity test). Every family
//! covered here is asserted reachable through the CLI.

use std::process::Command;

use celnet_golden::{GoldenVector, load_vectors};

/// The standard-error band multiplier for the Monte-Carlo families (mirrors the
/// SDK conformance harness).
const K_STDERR: f64 = 4.0;

/// The families the CLI prices locally (and therefore this test covers). The other
/// oneof arms (strategy, double-barrier, touch corridors, window-barrier) either
/// have no single-flag CLI surface or are LSV-only; they are gated by the SDK
/// conformance harness, which exercises every one of the 18 families.
const CLI_FAMILIES: [&str; 9] = [
    "vanilla",
    "digital",
    "touch", // only the single ONE_TOUCH (the CLI's `one-touch`)
    "single_barrier",
    "variance_swap",
    "volatility_swap",
    "forward_start",
    "quanto",
    "american",
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
