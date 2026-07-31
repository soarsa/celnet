//! Independent oracle for the **street-side / LP liquidity** rollup (guardrail 5).
//!
//! Every expected value below is computed **by hand / first principles** in the
//! comment beside it — NOT by calling the crate's own rollup path. Inputs are
//! chosen so the expectations are exactly representable in binary IEEE-754
//! (integer notionals; win-rates that are exact ratios of small integers; cover
//! distances that are integers or dyadic fractions), so the metrics are asserted
//! with **exact `==` equality**, not a slackened tolerance.
//!
//! Model recap (see `lp_metrics.rs` / `lp_record.rs`):
//!   tick_count        = Σ (record.tick)
//!   quote_count       = Σ (record.was_quoted)              // response presence
//!   deals_won         = Σ (record.was_won)
//!   won_notional      = Σ notional over won records
//!   missed            = Σ (record.was_missed)
//!   last_look_rejects = Σ (record.was_last_look_reject)
//!   win_rate          = deals_won / quote_count            (None if quote_count = 0)
//!   mean_cover        = Σ cover_distance / count(present)  (None if none present)

use celnet_analytics::{
    LpFlowMetrics, LpFlowRecord, group_by_lp, lp_metrics_from, merge_tick_counts,
};
use std::collections::BTreeMap;

/// A won panel row for `lp` at `notional` (a win implies it was quoted).
fn won(lp: &str, notional: f64) -> LpFlowRecord {
    LpFlowRecord {
        was_quoted: true,
        was_won: true,
        notional,
        ..LpFlowRecord::blank(lp, "EURUSD")
    }
}

/// A quoted-but-lost panel row (on the panel, deal went elsewhere).
fn missed(lp: &str) -> LpFlowRecord {
    LpFlowRecord {
        was_quoted: true,
        was_missed: true,
        ..LpFlowRecord::blank(lp, "EURUSD")
    }
}

/// A cover row: the LP was the runner-up, `dist` from the winner (also quoted+lost).
fn cover(lp: &str, dist: f64) -> LpFlowRecord {
    LpFlowRecord {
        was_quoted: true,
        was_missed: true,
        cover_distance: Some(dist),
        ..LpFlowRecord::blank(lp, "EURUSD")
    }
}

/// A last-look rejection (the LP responded, then its quote lapsed at ranking).
fn last_look(lp: &str) -> LpFlowRecord {
    LpFlowRecord {
        was_quoted: true,
        was_last_look_reject: true,
        ..LpFlowRecord::blank(lp, "EURUSD")
    }
}

// ───────────────────────────── case 1 — a winning LP ────────────────────────

#[test]
fn case1_winning_lp() {
    // 4 panels, all won by this LP. quote=4, won=4 ⇒ win_rate = 4/4 = 1.
    // won_notional = 4 × 5mm = 20mm. no misses/rejects/cover.
    let recs: Vec<LpFlowRecord> = (0..4).map(|_| won("LP_TIGHT", 5_000_000.0)).collect();
    let m = lp_metrics_from("LP_TIGHT", &recs);
    assert_eq!(m.quote_count, 4);
    assert_eq!(m.deals_won, 4);
    assert_eq!(m.won_notional, 20_000_000.0);
    assert_eq!(m.missed, 0);
    assert_eq!(m.last_look_rejects, 0);
    assert_eq!(m.win_rate, Some(1.0));
    assert_eq!(m.mean_cover, None);
    assert_eq!(m.tick_count, 0);
}

// ─────────────────────────── case 2 — a chronic-miss LP ─────────────────────

#[test]
fn case2_chronic_miss_lp() {
    // Quoted on 5 panels, won 1, missed 4. quote=5, won=1 ⇒ win_rate = 1/5 = 0.2.
    let mut recs = vec![won("LP_WIDE", 1_000_000.0)];
    recs.extend((0..4).map(|_| missed("LP_WIDE")));
    let m = lp_metrics_from("LP_WIDE", &recs);
    assert_eq!(m.quote_count, 5);
    assert_eq!(m.deals_won, 1);
    assert_eq!(m.missed, 4);
    assert_eq!(m.won_notional, 1_000_000.0);
    assert_eq!(m.win_rate, Some(0.2));
}

// ───────────────────────── case 3 — a last-look rejecter ────────────────────

#[test]
fn case3_last_look_rejecter() {
    // 8 panels quoted; won 0, rejected on last-look 3 (the other 5 simply lost).
    // quote=8, won=0 ⇒ win_rate = 0/8 = 0. last_look_rejects = 3.
    let mut recs: Vec<LpFlowRecord> = (0..5).map(|_| missed("LP_FADE")).collect();
    recs.extend((0..3).map(|_| last_look("LP_FADE")));
    let m = lp_metrics_from("LP_FADE", &recs);
    assert_eq!(m.quote_count, 8);
    assert_eq!(m.deals_won, 0);
    assert_eq!(m.last_look_rejects, 3);
    assert_eq!(m.win_rate, Some(0.0));
}

// ─────────────────────── case 4 — tick-rate over a window ───────────────────

#[test]
fn case4_tick_rate_over_window() {
    // 100 quote-update ticks, no panel activity. tick_count = 100; everything
    // else zero/None (an LP that streams but hasn't been dealt in the window).
    let recs: Vec<LpFlowRecord> = (0..100)
        .map(|_| LpFlowRecord::tick("LP_FAST", "EURUSD"))
        .collect();
    let m = lp_metrics_from("LP_FAST", &recs);
    assert_eq!(m.tick_count, 100);
    assert_eq!(m.quote_count, 0);
    assert_eq!(m.deals_won, 0);
    assert_eq!(m.win_rate, None);
    assert_eq!(m.mean_cover, None);
}

// ───────────────────────── case 5 — win-rate exact ratio ────────────────────

#[test]
fn case5_win_rate_exact() {
    // Quoted 8, won 3 (5 lost). win_rate = 3/8 = 0.375 (exact dyadic).
    let mut recs: Vec<LpFlowRecord> = (0..3).map(|_| won("LP_MID", 2_000_000.0)).collect();
    recs.extend((0..5).map(|_| missed("LP_MID")));
    let m = lp_metrics_from("LP_MID", &recs);
    assert_eq!(m.quote_count, 8);
    assert_eq!(m.deals_won, 3);
    assert_eq!(m.missed, 5);
    assert_eq!(m.win_rate, Some(0.375));
    assert_eq!(m.won_notional, 6_000_000.0);
}

// ──────────────────── case 6 — multi-LP aggregate (partition) ───────────────

#[test]
fn case6_multi_lp_partition() {
    // Three LPs on shared panels. group_by_lp must partition exhaustively &
    // disjointly: Σ per-LP wins/missed/quotes == whole-slice totals.
    let recs = vec![
        won("A", 1_000_000.0),
        missed("A"),
        won("B", 2_000_000.0),
        won("B", 3_000_000.0),
        missed("C"),
        last_look("C"),
    ];
    let by_lp = group_by_lp(&recs);
    assert_eq!(by_lp.len(), 3);
    assert_eq!(by_lp["A"].quote_count, 2);
    assert_eq!(by_lp["A"].deals_won, 1);
    assert_eq!(by_lp["A"].win_rate, Some(0.5));
    assert_eq!(by_lp["B"].deals_won, 2);
    assert_eq!(by_lp["B"].won_notional, 5_000_000.0);
    assert_eq!(by_lp["B"].win_rate, Some(1.0));
    assert_eq!(by_lp["C"].deals_won, 0);
    assert_eq!(by_lp["C"].last_look_rejects, 1);
    assert_eq!(by_lp["C"].win_rate, Some(0.0));

    // Exhaustive & disjoint: Σ per-LP == whole-slice total.
    let won_total: u64 = by_lp.values().map(|m| m.deals_won).sum();
    let quote_total: u64 = by_lp.values().map(|m| m.quote_count).sum();
    assert_eq!(won_total, 3); // A:1 + B:2 + C:0
    assert_eq!(quote_total, 6); // every record was quoted
}

// ─────────────────────────── case 7 — zero / empty guards ───────────────────

#[test]
fn case7_zero_guards() {
    // No quotes at all ⇒ win_rate None (no NaN/inf); no cover ⇒ mean_cover None.
    let empty = lp_metrics_from("NOBODY", &[]);
    assert_eq!(empty.win_rate, None);
    assert_eq!(empty.mean_cover, None);
    assert_eq!(empty.deals_won, 0);
    assert_eq!(empty.tick_count, 0);

    // Ticks only (no panel) ⇒ still win_rate None.
    let ticks = lp_metrics_from("STREAM", &[LpFlowRecord::tick("STREAM", "X")]);
    assert_eq!(ticks.win_rate, None);
    assert_eq!(ticks.tick_count, 1);
}

// ───────────────────────── case 8 — mean cover distance ─────────────────────

#[test]
fn case8_mean_cover_distance() {
    // LP was the cover on 3 panels at distances 2, 4, 6 ⇒ mean = 12/3 = 4.
    // (each cover row is also quoted+missed.) A 4th missed row carries no cover,
    // so it does NOT dilute the mean (mean is over present-cover rows only).
    let recs = vec![
        cover("LP_COVER", 2.0),
        cover("LP_COVER", 4.0),
        cover("LP_COVER", 6.0),
        missed("LP_COVER"),
    ];
    let m = lp_metrics_from("LP_COVER", &recs);
    assert_eq!(m.quote_count, 4);
    assert_eq!(m.missed, 4);
    assert_eq!(m.mean_cover, Some(4.0));
}

// ───────────────────── case 9 — merge_tick_counts overlay ───────────────────

#[test]
fn case9_merge_tick_counts() {
    // Fold: LP "A" won a deal (quote 1, won 1). Then merge the aggregation-ingest
    // tick tally: A ticked 40 times, and a stream-only LP "Z" ticked 7 times with
    // NO panel presence — Z becomes a tick_only row.
    let mut by_lp = group_by_lp(&[won("A", 5_000_000.0)]);
    let mut ticks: BTreeMap<String, u64> = BTreeMap::new();
    ticks.insert("A".to_owned(), 40);
    ticks.insert("Z".to_owned(), 7);
    merge_tick_counts(&mut by_lp, &ticks);

    assert_eq!(by_lp.len(), 2);
    // A's panel metrics are untouched; only tick_count grows.
    assert_eq!(by_lp["A"].tick_count, 40);
    assert_eq!(by_lp["A"].deals_won, 1);
    assert_eq!(by_lp["A"].win_rate, Some(1.0));
    // Z is a tick-only row: ticks present, everything else zero/None (honest —
    // "ticked but no deals seen", not a fabricated outcome).
    assert_eq!(by_lp["Z"], LpFlowMetrics::tick_only("Z", 7));
    assert_eq!(by_lp["Z"].win_rate, None);
}

// ───────────────────────────── property tests ──────────────────────────────

use proptest::prelude::*;

/// A well-formed panel outcome: exactly one of {won, missed, last-look-reject,
/// quote-only, tick}. `won`/`missed`/`reject`/`quote-only` all imply `was_quoted`,
/// keeping the flag invariants (won ⇒ quoted; won & missed disjoint) true by
/// construction — so the fold's counts must satisfy the same inequalities.
fn outcome(lp: &'static str) -> impl Strategy<Value = LpFlowRecord> {
    prop_oneof![
        (1u32..=1_000_000).prop_map(move |n| won(lp, f64::from(n))),
        Just(missed(lp)),
        Just(last_look(lp)),
        Just(LpFlowRecord {
            was_quoted: true,
            ..LpFlowRecord::blank(lp, "EURUSD")
        }),
        Just(LpFlowRecord::tick(lp, "EURUSD")),
    ]
}

proptest! {
    /// Wins never exceed quotes, and wins + missed never exceed quotes (both are
    /// disjoint subsets of the quoted rows). win_rate ∈ [0, 1] when defined.
    #[test]
    fn prop_won_and_missed_bounded_by_quoted(recs in prop::collection::vec(outcome("P"), 0..40)) {
        let m = lp_metrics_from("P", &recs);
        prop_assert!(m.deals_won <= m.quote_count);
        prop_assert!(m.deals_won + m.missed <= m.quote_count);
        if let Some(wr) = m.win_rate {
            prop_assert!((0.0..=1.0).contains(&wr));
        } else {
            prop_assert_eq!(m.quote_count, 0);
        }
    }

    /// group_by_lp is an exhaustive, disjoint partition: Σ per-LP extensive
    /// quantities == the whole-slice totals. Integer notionals keep sums exact.
    #[test]
    fn prop_partition_exhaustive(
        rows in prop::collection::vec(("A|B|C", outcome("_")), 0..48)
    ) {
        // Re-key each generated record onto one of three LPs.
        let recs: Vec<LpFlowRecord> = rows
            .into_iter()
            .map(|(lp, r)| LpFlowRecord { lp_id: lp.to_owned(), ..r })
            .collect();
        let whole = lp_metrics_from("ALL", &recs);
        let by_lp = group_by_lp(&recs);
        let tick: u64 = by_lp.values().map(|m: &LpFlowMetrics| m.tick_count).sum();
        let quote: u64 = by_lp.values().map(|m| m.quote_count).sum();
        let won_c: u64 = by_lp.values().map(|m| m.deals_won).sum();
        let miss: u64 = by_lp.values().map(|m| m.missed).sum();
        let rej: u64 = by_lp.values().map(|m| m.last_look_rejects).sum();
        let won_n: f64 = by_lp.values().map(|m| m.won_notional).sum();
        prop_assert_eq!(tick, whole.tick_count);
        prop_assert_eq!(quote, whole.quote_count);
        prop_assert_eq!(won_c, whole.deals_won);
        prop_assert_eq!(miss, whole.missed);
        prop_assert_eq!(rej, whole.last_look_rejects);
        prop_assert_eq!(won_n, whole.won_notional);
    }
}
