//! Independent oracle for `celnet-analytics` (guardrail 5).
//!
//! Every expected value below is computed **by hand / first principles** in the
//! comment beside it — NOT by calling the crate's own rollup path. Inputs are
//! chosen so the expectations are exactly representable in binary IEEE-754
//! (integer notionals; margins/markouts/spreads that are integers or dyadic
//! fractions; hit-rates and $/mm that are exact ratios of small integers), so
//! the metrics are asserted with **exact `==` equality**, not a slackened
//! tolerance.
//!
//! Model recap (see `metrics.rs` / `fishing.rs`):
//!   traded_mm            = Σ traded notional / 1e6
//!   gross_pnl            = Σ margin  (traded records only)
//!   net_pnl              = gross_pnl − Σ markout − Σ hedge_cost
//!   dpm_gross            = gross_pnl / traded_mm          (None if traded_mm = 0)
//!   dpm_net              = net_pnl   / traded_mm          (None if traded_mm = 0)
//!   captured_vs_offered  = gross_pnl / Σ quoted_spread    (None if Σ = 0)
//!   breakeven_spread     = ((Σmarkout+Σhedge)/traded_mm) / captured_vs_offered
//!   quote_to_trade_ratio = quotes / trades                (None if trades = 0)
//!   hit_rate             = trades / quotes                (None if quotes = 0)
//!   fishing_score        = clamp(1−hit_rate,0,1) · (1 − clamp(max(net_dpm,0)/50,0,1))
//!                          (0 if quotes = 0; net treated as 0 if no trades)

use celnet_analytics::{
    ClientFlowMetrics, DPM_VALUE_SCALE, FlowRecord, Side, fishing_score, group_by_asset,
    group_by_client, group_by_counterparty, group_by_instrument, metrics_from,
};

/// A template record: quoted, not (yet) traded, all cash quantities zero. Cases
/// override only the fields they exercise via struct-update syntax — keeping
/// every row a readable truth-table line with no wide positional helper.
fn base(client: &str) -> FlowRecord {
    FlowRecord {
        client: client.to_owned(),
        counterparty: "CPTY".to_owned(),
        instrument: "EURUSD".to_owned(),
        asset: "fxo".to_owned(),
        notional: 0.0,
        side: Side::Buy,
        was_quoted: true,
        was_traded: false,
        margin: 0.0,
        quoted_spread: 0.0,
        cover_distance: None,
        markout: None,
        hedge_cost: None,
    }
}

/// A quoted-and-traded fill with no cover/markout/hedge attributed.
fn fill(client: &str, notional: f64, margin: f64, quoted_spread: f64) -> FlowRecord {
    FlowRecord {
        was_traded: true,
        notional,
        margin,
        quoted_spread,
        ..base(client)
    }
}

/// A pure quote (no trade).
fn quote(client: &str) -> FlowRecord {
    FlowRecord {
        quoted_spread: 2.0,
        ..base(client)
    }
}

// ───────────────────────────── case 1 — single-trade $/mm ───────────────────

#[test]
fn case1_single_trade_dpm() {
    // 1 fill: 5mm @ margin 250, quoted_spread 500, no markout/hedge.
    // traded_mm = 5. dpm_gross = 250/5 = 50. net = 250 ⇒ dpm_net = 50.
    // captured_vs_offered = 250/500 = 0.5. hit_rate = 1/1 = 1. q2t = 1/1 = 1.
    // fishing: fished = 1−1 = 0 ⇒ score 0.
    let m = metrics_from("ACME", &[fill("ACME", 5_000_000.0, 250.0, 500.0)]);
    assert_eq!(m.traded_notional, 5_000_000.0);
    assert_eq!(m.traded_count, 1);
    assert_eq!(m.quote_count, 1);
    assert_eq!(m.gross_pnl, 250.0);
    assert_eq!(m.net_pnl, 250.0);
    assert_eq!(m.dpm_gross, Some(50.0));
    assert_eq!(m.dpm_net, Some(50.0));
    assert_eq!(m.captured_vs_offered, Some(0.5));
    assert_eq!(m.hit_rate, Some(1.0));
    assert_eq!(m.quote_to_trade_ratio, Some(1.0));
    assert_eq!(m.fishing_score, 0.0);
}

// ───────────────────────── case 2 — multi-trade aggregate ───────────────────

#[test]
fn case2_multi_trade_aggregate() {
    // 2 fills: 10mm@400 and 20mm@800. traded_mm = 30. gross = 1200.
    // dpm_gross = 1200/30 = 40. no markout/hedge ⇒ dpm_net = 40.
    let recs = [
        fill("ACME", 10_000_000.0, 400.0, 0.0),
        fill("ACME", 20_000_000.0, 800.0, 0.0),
    ];
    let m = metrics_from("ACME", &recs);
    assert_eq!(m.traded_notional, 30_000_000.0);
    assert_eq!(m.traded_count, 2);
    assert_eq!(m.gross_pnl, 1200.0);
    assert_eq!(m.dpm_gross, Some(40.0));
    assert_eq!(m.dpm_net, Some(40.0));
    // Σ quoted_spread = 0 ⇒ captured_vs_offered None (guarded).
    assert_eq!(m.captured_vs_offered, None);
    assert_eq!(m.breakeven_spread, None);
}

// ─────────────────────────── case 3 — pure fisher (0 trades) ────────────────

#[test]
fn case3_pure_fisher() {
    // 8 quotes, 0 trades. hit_rate = 0/8 = 0. q2t = None (no trades).
    // dpm None. fishing: quotes>0, fished = 1−0 = 1, net→0 ⇒ penalty 1 ⇒ 1.0.
    let recs: Vec<FlowRecord> = (0..8).map(|_| quote("FISH")).collect();
    let m = metrics_from("FISH", &recs);
    assert_eq!(m.quote_count, 8);
    assert_eq!(m.traded_count, 0);
    assert_eq!(m.traded_notional, 0.0);
    assert_eq!(m.dpm_gross, None);
    assert_eq!(m.dpm_net, None);
    assert_eq!(m.hit_rate, Some(0.0));
    assert_eq!(m.quote_to_trade_ratio, None);
    assert_eq!(m.fishing_score, 1.0);
}

// ─────────────── case 4 — fisher with one loss-making conversion ────────────

#[test]
fn case4_fisher_high_ratio_low_hit() {
    // 8 quotes total, exactly 1 of them trades (7 unconverted).
    // hit_rate = 1/8 = 0.125. q2t = 8/1 = 8.
    // fill: 2mm @ margin 4, markout 10 ⇒ net = 4−10 = −6, dpm_net = −6/2 = −3.
    // fishing: fished = 1−0.125 = 0.875; net<0 ⇒ penalty 1 ⇒ score 0.875.
    let mut recs: Vec<FlowRecord> = (0..7).map(|_| quote("FISH2")).collect();
    recs.push(FlowRecord {
        was_traded: true,
        side: Side::Sell,
        notional: 2_000_000.0,
        margin: 4.0,
        quoted_spread: 20.0,
        markout: Some(10.0),
        ..base("FISH2")
    });
    let m = metrics_from("FISH2", &recs);
    assert_eq!(m.quote_count, 8);
    assert_eq!(m.traded_count, 1);
    assert_eq!(m.hit_rate, Some(0.125));
    assert_eq!(m.quote_to_trade_ratio, Some(8.0));
    assert_eq!(m.dpm_gross, Some(2.0));
    assert_eq!(m.dpm_net, Some(-3.0));
    assert_eq!(m.fishing_score, 0.875);
}

// ─────────────────────── case 5 — valuable client (low fishing) ─────────────

#[test]
fn case5_valuable_client() {
    // 4 quotes, all 4 trade (hit_rate = 1). Each 10mm @ margin 1000, cover 2.
    // gross = 4000, traded_mm = 40 ⇒ dpm_gross = 100.
    // markout 100 each (Σ 400), hedge 25 each (Σ 100). net = 4000−400−100 = 3500.
    // dpm_net = 3500/40 = 87.5 (≥ SCALE 50 ⇒ full profit ⇒ penalty 0).
    // fished = 1−1 = 0 ⇒ score 0. mean cover = (2·4)/4 = 2.
    let recs: Vec<FlowRecord> = (0..4)
        .map(|_| FlowRecord {
            cover_distance: Some(2.0),
            markout: Some(100.0),
            hedge_cost: Some(25.0),
            ..fill("GOOD", 10_000_000.0, 1000.0, 2000.0)
        })
        .collect();
    let m = metrics_from("GOOD", &recs);
    assert_eq!(m.gross_pnl, 4000.0);
    assert_eq!(m.total_markout, 400.0);
    assert_eq!(m.total_hedge_cost, 100.0);
    assert_eq!(m.net_pnl, 3500.0);
    assert_eq!(m.dpm_gross, Some(100.0));
    assert_eq!(m.dpm_net, Some(87.5));
    assert_eq!(m.hit_rate, Some(1.0));
    assert_eq!(m.mean_cover_distance, Some(2.0));
    assert_eq!(m.fishing_score, 0.0);
}

// ───────────── case 6 — fat gross / NEGATIVE net (§11.1a sign flip) ─────────

#[test]
fn case6_fat_gross_negative_net() {
    // 1 fill: 10mm @ margin 300 ⇒ dpm_gross = 300/10 = 30 (> 0).
    // markout 500 ⇒ net = 300−500 = −200 ⇒ dpm_net = −200/10 = −20 (< 0).
    // The whole point: gross positive, net negative — assert the SIGN FLIP.
    let m = metrics_from(
        "TOXIC",
        &[FlowRecord {
            markout: Some(500.0),
            ..fill("TOXIC", 10_000_000.0, 300.0, 600.0)
        }],
    );
    assert_eq!(m.dpm_gross, Some(30.0));
    assert_eq!(m.dpm_net, Some(-20.0));
    assert!(m.dpm_gross.unwrap() > 0.0 && m.dpm_net.unwrap() < 0.0);
}

// ─────────────── case 7 — divide-by-zero guards (no trades) ─────────────────

#[test]
fn case7_zero_trade_guards() {
    // 3 quotes, no trades. All trade-denominated ratios None (no NaN/inf).
    let recs: Vec<FlowRecord> = (0..3).map(|_| quote("QONLY")).collect();
    let m = metrics_from("QONLY", &recs);
    assert_eq!(m.dpm_gross, None);
    assert_eq!(m.dpm_net, None);
    assert_eq!(m.captured_vs_offered, None);
    assert_eq!(m.breakeven_spread, None);
    assert_eq!(m.quote_to_trade_ratio, None);
    assert_eq!(m.hit_rate, Some(0.0));
    // Empty input ⇒ everything None/0, fishing 0 (no quotes consumed).
    let empty = metrics_from("NOBODY", &[]);
    assert_eq!(empty.dpm_gross, None);
    assert_eq!(empty.hit_rate, None);
    assert_eq!(empty.quote_to_trade_ratio, None);
    assert_eq!(empty.fishing_score, 0.0);
}

// ───────────── case 8 — captured-vs-offered + breakeven spread exact ────────

#[test]
fn case8_captured_and_breakeven() {
    // 1 fill: 5mm @ margin 40, quoted_spread 100, cover 2, markout 10, hedge 10.
    // captured_vs_offered = 40/100 = 0.4.
    // traded_mm = 5. cost = 20 ⇒ cost/mm = 20/5 = 4.
    // breakeven_spread = 4 / 0.4 = 10 ($/mm).
    // net = 40−20 = 20 ⇒ dpm_net = 20/5 = 4.
    let m = metrics_from(
        "SPREAD",
        &[FlowRecord {
            cover_distance: Some(2.0),
            markout: Some(10.0),
            hedge_cost: Some(10.0),
            ..fill("SPREAD", 5_000_000.0, 40.0, 100.0)
        }],
    );
    assert_eq!(m.captured_vs_offered, Some(0.4));
    assert_eq!(m.breakeven_spread, Some(10.0));
    assert_eq!(m.mean_cover_distance, Some(2.0));
    assert_eq!(m.dpm_net, Some(4.0));
}

// ─────────────── case 9 — grouping partition (exhaustive & disjoint) ────────

#[test]
fn case9_grouping_partition() {
    // Three clients, mixed. The per-client traded notional must sum back to the
    // whole-slice total (partition is exhaustive + disjoint).
    let recs = vec![
        fill("A", 1_000_000.0, 10.0, 0.0),
        quote("A"),
        fill("B", 2_000_000.0, 20.0, 0.0),
        fill("B", 3_000_000.0, 30.0, 0.0),
        fill("C", 4_000_000.0, 40.0, 0.0),
    ];
    let by_client = group_by_client(&recs);
    assert_eq!(by_client.len(), 3);
    assert_eq!(by_client["A"].traded_notional, 1_000_000.0);
    assert_eq!(by_client["A"].quote_count, 2); // fill + pure quote both quoted
    assert_eq!(by_client["A"].traded_count, 1);
    assert_eq!(by_client["B"].traded_notional, 5_000_000.0);
    assert_eq!(by_client["C"].traded_notional, 4_000_000.0);

    // Exhaustive & disjoint: Σ per-client == whole-slice total.
    let total: f64 = by_client.values().map(|m| m.traded_notional).sum();
    let whole = metrics_from("ALL", &recs);
    assert_eq!(total, whole.traded_notional);
    let count: u64 = by_client.values().map(|m| m.traded_count).sum();
    assert_eq!(count, whole.traded_count);

    // Instrument/cpty helpers group the same records under one key here.
    let by_instr = group_by_instrument(&recs);
    assert_eq!(by_instr.len(), 1);
    assert_eq!(by_instr["EURUSD"].traded_notional, whole.traded_notional);
    let by_cpty = group_by_counterparty(&recs);
    assert_eq!(by_cpty.len(), 1);
    assert_eq!(by_cpty["CPTY"].traded_count, whole.traded_count);
}

// ─────────────── case 10 — fishing_score standalone (dyadic) ────────────────

#[test]
fn case10_fishing_score_formula() {
    assert_eq!(DPM_VALUE_SCALE, 50.0);
    // hit_rate 0.25 ⇒ fished 0.75; net 25 ⇒ penalty 1−25/50 = 0.5 ⇒ 0.375.
    assert_eq!(fishing_score(Some(0.25), Some(25.0), 4), 0.375);
    // no quotes ⇒ 0 regardless.
    assert_eq!(fishing_score(None, None, 0), 0.0);
    // full conversion (hit 1) ⇒ 0 even at zero net.
    assert_eq!(fishing_score(Some(1.0), Some(0.0), 10), 0.0);
    // net ≥ SCALE ⇒ penalty 0 ⇒ 0 even with low hit-rate.
    assert_eq!(fishing_score(Some(0.1), Some(50.0), 10), 0.0);
    // pure fisher: no trades, low hit ⇒ 1.
    assert_eq!(fishing_score(Some(0.0), None, 20), 1.0);
}

// ─────────── case 11 — cross-asset grouping (FI vs FXO partition) ───────────

#[test]
fn case11_group_by_asset() {
    // Two products under one desk: an FXO fill (5mm@250) and two FI fills
    // (10mm@0 and 20mm@0 — the FI desk path carries no per-feature margin, so
    // its $/mm is legitimately 0/None, not fabricated). `group_by_asset` must
    // partition them exhaustively & disjointly by the opaque `asset` label.
    let fxo = FlowRecord {
        asset: "fxo".to_owned(),
        ..fill("ACME", 5_000_000.0, 250.0, 500.0)
    };
    let fi_a = FlowRecord {
        asset: "fi".to_owned(),
        instrument: "US10Y".to_owned(),
        ..fill("HFUND", 10_000_000.0, 0.0, 0.0)
    };
    let fi_b = FlowRecord {
        asset: "fi".to_owned(),
        instrument: "US30Y".to_owned(),
        ..fill("HFUND", 20_000_000.0, 0.0, 0.0)
    };
    let recs = vec![fxo, fi_a, fi_b];

    let by_asset = group_by_asset(&recs);
    assert_eq!(by_asset.len(), 2);
    // FXO: the single 5mm fill @ margin 250 ⇒ dpm_gross 50.
    assert_eq!(by_asset["fxo"].traded_notional, 5_000_000.0);
    assert_eq!(by_asset["fxo"].traded_count, 1);
    assert_eq!(by_asset["fxo"].dpm_gross, Some(50.0));
    // FI: 30mm across two fills, zero margin ⇒ dpm_gross 0 (present, not None:
    // notional traded, so the ratio is defined and exactly 0).
    assert_eq!(by_asset["fi"].traded_notional, 30_000_000.0);
    assert_eq!(by_asset["fi"].traded_count, 2);
    assert_eq!(by_asset["fi"].dpm_gross, Some(0.0));

    // Exhaustive & disjoint: Σ per-asset traded notional == whole-slice total.
    let whole = metrics_from("ALL", &recs);
    let total: f64 = by_asset.values().map(|m| m.traded_notional).sum();
    assert_eq!(total, whole.traded_notional);
    let count: u64 = by_asset.values().map(|m| m.traded_count).sum();
    assert_eq!(count, whole.traded_count);
}

// ───────────────────────────── property tests ──────────────────────────────

use proptest::prelude::*;

/// Integer-magnitude fill generator (exact-in-f64 sums); markout/hedge ≥ 0.
fn traded_fill(client: &'static str) -> impl Strategy<Value = FlowRecord> {
    (
        1u32..=1_000_000, // notional (integer USD, exact in f64)
        0u32..=10_000,    // margin
        0u32..=1_000,     // markout ≥ 0
        0u32..=1_000,     // hedge ≥ 0
        any::<bool>(),    // side
    )
        .prop_map(move |(n, mg, mk, hd, s)| FlowRecord {
            side: if s { Side::Buy } else { Side::Sell },
            markout: Some(f64::from(mk)),
            hedge_cost: Some(f64::from(hd)),
            ..fill(client, f64::from(n), f64::from(mg), 0.0)
        })
}

proptest! {
    /// With markout + hedge ≥ 0 and positive traded notional, net $/mm never
    /// exceeds gross $/mm (subtracting non-negative costs can only lower it).
    #[test]
    fn prop_dpm_net_le_gross(recs in prop::collection::vec(traded_fill("P"), 1..24)) {
        let m = metrics_from("P", &recs);
        // At least one integer-notional fill ⇒ traded_notional > 0 ⇒ both Some.
        let g = m.dpm_gross.expect("traded volume present");
        let n = m.dpm_net.expect("traded volume present");
        prop_assert!(n <= g);
    }

    /// The fishing score is always a well-formed probability in [0, 1].
    #[test]
    fn prop_fishing_bounded(
        recs in prop::collection::vec(
            prop_oneof![traded_fill("Q"), Just(quote("Q"))], 0..40)
    ) {
        let m = metrics_from("Q", &recs);
        prop_assert!(m.fishing_score.is_finite());
        prop_assert!((0.0..=1.0).contains(&m.fishing_score));
    }

    /// Grouping is an exhaustive, disjoint partition: Σ per-client traded
    /// notional == whole-slice total, and Σ per-client traded count == total.
    /// Integer notionals keep the sums exact in f64.
    #[test]
    fn prop_partition_exhaustive(
        rows in prop::collection::vec(
            ("A|B|C", 1u32..=1_000_000, any::<bool>()), 0..32)
    ) {
        let recs: Vec<FlowRecord> = rows
            .into_iter()
            .map(|(c, n, traded)| FlowRecord {
                was_traded: traded,
                ..fill(&c, f64::from(n), 1.0, 0.0)
            })
            .collect();
        let whole = metrics_from("ALL", &recs);
        let by_client = group_by_client(&recs);
        let notional: f64 = by_client.values().map(|m: &ClientFlowMetrics| m.traded_notional).sum();
        let count: u64 = by_client.values().map(|m| m.traded_count).sum();
        prop_assert_eq!(notional, whole.traded_notional);
        prop_assert_eq!(count, whole.traded_count);
    }
}
