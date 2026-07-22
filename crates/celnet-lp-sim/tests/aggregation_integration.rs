//! Integration test — the point of the crate: construct K [`SimLp`]s and run them
//! through the **real** [`celnet_aggregation`] consolidation engine, then assert
//! the resulting [`ConsolidatedBook`] against analytic ground truth derived from
//! the LPs' own (ladder-mode) quotes.
//!
//! Ladder mode makes every LP's bid/offer an exact constant, so the consolidated
//! `best_bid = max(fresh bids)` / `best_offer = min(fresh offers)` are exact
//! integer-bit assertions — not plausibility checks. Fault injection then drives
//! the consolidator's staleness decay (a stopped feed drops out) and MAD
//! divergence gate (a fat-finger print is excluded but reported), and the
//! degenerate all-stale panel proves the no-fresh-source ⇒ no-price contract.

use celnet_aggregation::{
    ConsolidatedBook, ConsolidationConfig, ConsolidationError, ExclusionReason, Instrument,
    VenueFeed,
};
use celnet_lp_sim::{Fault, LpParams, SimLp, into_feeds};
use celnet_types::{Ccy, CcyPair, Tenor};

const S: i64 = 1_000_000_000;
/// Valuation instant: 100 s (epoch nanos). Well inside every fresh LP's cutoff.
const NOW: i64 = 100 * S;

/// A 5-year bond line key. The consolidation is asset-agnostic over the key, so
/// the underlying's identity is cosmetic; the LP mids are bond price handles.
fn bond_5y() -> Instrument {
    Instrument::new(CcyPair::new(Ccy::EUR, Ccy::USD), Tenor::Years(5))
}

/// The consolidation config used across the suite: 30 s half-life, 60 s hard
/// cutoff, and a 0.50 (price-point) divergence tolerance suited to a ~100.0 bond
/// price handle.
fn cfg() -> ConsolidationConfig {
    ConsolidationConfig {
        staleness_half_life_secs: 30.0,
        staleness_cutoff_secs: 60.0,
        divergence_tolerance: 0.50,
    }
}

/// A healthy ladder LP quoting `mid` with the given half-spread and skew.
fn ladder(venue: &str, mid: f64, half_spread: f64, skew: f64) -> SimLp {
    let params = LpParams {
        half_spread,
        skew,
        ..LpParams::tight()
    };
    SimLp::quoting_fixed(venue, bond_5y(), mid, params)
}

/// The four fresh, well-behaved LPs shared by several cases (distinct mids,
/// spreads, and skews so the BBO winner is non-trivial).
fn healthy_panel() -> Vec<SimLp> {
    vec![
        ladder("lp-a", 100.00, 0.020, 0.000),
        ladder("lp-b", 100.01, 0.030, 0.010),
        ladder("lp-c", 99.99, 0.020, -0.010),
        ladder("lp-d", 100.02, 0.025, 0.000),
    ]
}

/// The analytic best bid (max) / best offer (min) over a set of LPs' own
/// top-of-book quotes at `NOW` — the ground truth the consolidator must match.
fn analytic_bbo(lps: &[SimLp]) -> (f64, f64) {
    let best_bid = lps
        .iter()
        .map(|lp| lp.top_of_book(&bond_5y(), NOW).unwrap().bid)
        .fold(f64::NEG_INFINITY, f64::max);
    let best_offer = lps
        .iter()
        .map(|lp| lp.top_of_book(&bond_5y(), NOW).unwrap().offer)
        .fold(f64::INFINITY, f64::min);
    (best_bid, best_offer)
}

#[test]
fn consolidated_bbo_equals_analytic_max_bid_min_offer() {
    let panel = healthy_panel();
    let (want_bid, want_offer) = analytic_bbo(&panel);

    let feeds = into_feeds(panel);
    let book = ConsolidatedBook::consolidate(&feeds, &bond_5y(), NOW, &cfg()).unwrap();

    // Exact bit-equality: the real engine reproduced the analytic BBO.
    assert_eq!(book.best_bid.to_bits(), want_bid.to_bits());
    assert_eq!(book.best_offer.to_bits(), want_offer.to_bits());
    assert_eq!(book.contributing(), 4);
    // The composite mid is the (equal-weight, all-fresh) mean of the LP midpoints,
    // so it must lie within the surviving LPs' mid range — the analytic envelope
    // for a convex combination (the tighter max-bid/min-offer BBO can even cross).
    let mids: Vec<f64> = feeds
        .iter()
        .map(|f| f.top_of_book(&bond_5y(), NOW).unwrap().mid())
        .collect();
    let min_mid = mids.iter().copied().fold(f64::INFINITY, f64::min);
    let max_mid = mids.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(book.composite_mid >= min_mid && book.composite_mid <= max_mid);
    assert!((0.0..=1.0).contains(&book.confidence));
}

#[test]
fn staled_lp_drops_out_of_the_composite() {
    // One LP stopped updating 200 s ago — well past the 60 s hard cutoff.
    let mut panel = healthy_panel();
    panel.push(
        ladder("lp-stale", 100.005, 0.02, 0.0).with_fault(Fault::Stale {
            frozen_at_nanos: NOW - 200 * S,
        }),
    );

    // Ground truth excludes the stale LP: max/min over the four fresh ones only.
    let (want_bid, want_offer) = analytic_bbo(&panel[..4]);

    let feeds = into_feeds(panel);
    let book = ConsolidatedBook::consolidate(&feeds, &bond_5y(), NOW, &cfg()).unwrap();

    let stale = book
        .contributions
        .iter()
        .find(|c| c.venue.as_str() == "lp-stale")
        .unwrap();
    assert_eq!(stale.excluded, Some(ExclusionReason::Stale));
    assert!(!stale.contributed());
    // The stale LP set neither the BBO nor the composite; four venues remain.
    assert_eq!(book.contributing(), 4);
    assert_eq!(book.best_bid.to_bits(), want_bid.to_bits());
    assert_eq!(book.best_offer.to_bits(), want_offer.to_bits());
}

#[test]
fn outlier_lp_is_mad_excluded_but_reported() {
    // Four tight LPs around 100.0 plus one fat-finger print +5.0 points rich.
    let mut panel = healthy_panel();
    panel.push(ladder("lp-fat", 100.00, 0.02, 0.0).with_fault(Fault::Outlier { shift: 5.0 }));

    let (want_bid, want_offer) = analytic_bbo(&panel[..4]);

    let feeds = into_feeds(panel);
    let book = ConsolidatedBook::consolidate(&feeds, &bond_5y(), NOW, &cfg()).unwrap();

    let fat = book
        .contributions
        .iter()
        .find(|c| c.venue.as_str() == "lp-fat")
        .unwrap();
    // Visible in the report, excluded as divergent, with its large deviation.
    assert_eq!(fat.excluded, Some(ExclusionReason::Divergent));
    assert!(
        fat.deviation > 4.0,
        "outlier deviation {} too small",
        fat.deviation
    );
    assert!(
        !book.gating_undecidable,
        "5 fresh venues ⇒ gating is decidable"
    );
    // The divergent print set neither the BBO nor the composite mid.
    assert_eq!(book.contributing(), 4);
    assert_eq!(book.best_bid.to_bits(), want_bid.to_bits());
    assert_eq!(book.best_offer.to_bits(), want_offer.to_bits());
    assert!(
        book.composite_mid < 101.0,
        "outlier poisoned the composite mid"
    );
}

#[test]
fn confidence_and_contributor_count_track_the_active_set() {
    // A tight, all-fresh panel should be more confident than a dispersed one.
    let tight = into_feeds(vec![
        ladder("a", 100.000, 0.01, 0.0),
        ladder("b", 100.005, 0.01, 0.0),
        ladder("c", 99.995, 0.01, 0.0),
    ]);
    let dispersed = into_feeds(vec![
        ladder("a", 99.80, 0.01, 0.0),
        ladder("b", 100.00, 0.01, 0.0),
        ladder("c", 100.20, 0.01, 0.0),
    ]);
    let bt = ConsolidatedBook::consolidate(&tight, &bond_5y(), NOW, &cfg()).unwrap();
    let bd = ConsolidatedBook::consolidate(&dispersed, &bond_5y(), NOW, &cfg()).unwrap();

    assert_eq!(bt.contributing(), 3);
    assert_eq!(bd.contributing(), 3);
    for b in [&bt, &bd] {
        assert!((0.0..=1.0).contains(&b.confidence));
    }
    assert!(
        bt.confidence > bd.confidence,
        "tighter agreement ⇒ higher confidence ({} vs {})",
        bt.confidence,
        bd.confidence
    );
}

#[test]
fn zero_fresh_contributors_is_no_price() {
    // Every LP stopped updating long ago ⇒ the consolidator has no fresh source
    // and returns AllExcluded (an undecidable / no-price book).
    let dead: Vec<SimLp> = ["a", "b", "c"]
        .iter()
        .map(|v| {
            ladder(v, 100.0, 0.02, 0.0).with_fault(Fault::Stale {
                frozen_at_nanos: NOW - 300 * S,
            })
        })
        .collect();
    let feeds = into_feeds(dead);
    let err = ConsolidatedBook::consolidate(&feeds, &bond_5y(), NOW, &cfg()).unwrap_err();
    assert_eq!(err, ConsolidationError::AllExcluded);
}
