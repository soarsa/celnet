//! Integration test for the **LP-SIM Treasury feed** end to end: load the REAL
//! bundled US-Treasury universe, stand up the `LP-SIM` member panel over it, and
//! run it through the REAL [`celnet_aggregation::ConsolidatedBook`] engine — the
//! same engine an operator-defined FI Aggregated Book uses server-side.
//!
//! The assertions are analytic, not plausibility checks:
//!
//! - the consolidated `best_bid` is the **max** of the fresh members' bids and
//!   `best_offer` the **min** of their offers (exact bit-equality);
//! - every per-LP contribution is attributed to a named `LP-SIM…` venue and the
//!   book is keyed on the bond's canonical engine instrument (identity carried
//!   through);
//! - the composite mid lies inside the surviving members' mid envelope (the convex
//!   combination is never outside its inputs — validated against the envelope, not
//!   a possibly-crossed BBO);
//! - a stale member drops out and the BBO falls back to the fresh members only.

use celnet_aggregation::{ConsolidatedBook, ExclusionReason, VenueFeed};
use celnet_lp_sim::{
    Fault, LpSimConfig, TreasuryBond, build_fleet, into_feeds, load_coupon_universe,
};

const S: i64 = 1_000_000_000;
/// Valuation instant: 100 s (epoch nanos) — inside every fresh member's cutoff.
const NOW: i64 = 100 * S;

/// A small config that seeds a 4-member `LP-SIM` panel off the reference universe.
fn cfg() -> LpSimConfig {
    LpSimConfig {
        members: 4,
        seed: 0xA11CE,
        ..LpSimConfig::default()
    }
}

/// Pick a handful of modellable coupon Treasuries (ones with a solvable reference
/// yield at the config settlement) so the panel actually quotes them.
fn selection(cfg: &LpSimConfig) -> Vec<TreasuryBond> {
    load_coupon_universe()
        .into_iter()
        .filter(|b| {
            b.yield_model(cfg.settlement, cfg.reversion_per_sec, cfg.perturbation)
                .is_some()
        })
        .take(6)
        .collect()
}

#[test]
fn lp_sim_consolidates_real_treasuries_to_the_analytic_bbo() {
    let cfg = cfg();
    let bonds = selection(&cfg);
    assert!(
        !bonds.is_empty(),
        "no modellable coupon Treasuries in the universe"
    );

    let feeds = into_feeds(build_fleet(&cfg, &bonds));
    assert_eq!(feeds.len(), cfg.members, "one feed per LP-SIM member");

    let ccfg = cfg.consolidation();
    let mut checked = 0;
    for bond in &bonds {
        let instr = bond.engine_instrument();
        // Analytic ground truth from the members' own top-of-book at NOW.
        let quotes: Vec<_> = feeds
            .iter()
            .filter_map(|f| f.top_of_book(&instr, NOW))
            .collect();
        assert_eq!(
            quotes.len(),
            cfg.members,
            "every member quotes {}",
            bond.cusip
        );
        let want_bid = quotes
            .iter()
            .map(|q| q.bid)
            .fold(f64::NEG_INFINITY, f64::max);
        let want_offer = quotes.iter().map(|q| q.offer).fold(f64::INFINITY, f64::min);
        let min_mid = quotes.iter().map(|q| q.mid()).fold(f64::INFINITY, f64::min);
        let max_mid = quotes
            .iter()
            .map(|q| q.mid())
            .fold(f64::NEG_INFINITY, f64::max);

        let book = ConsolidatedBook::consolidate(&feeds, &instr, NOW, &ccfg).unwrap();

        // Identity carried through: the book is keyed on the bond's engine key, and
        // every contribution is attributed to a named LP-SIM connection.
        assert_eq!(
            book.instrument, instr,
            "book keyed on the bond's engine instrument"
        );
        assert_eq!(
            book.contributing(),
            cfg.members,
            "all fresh members contribute"
        );
        for c in &book.contributions {
            assert!(
                c.venue.as_str().starts_with("LP-SIM"),
                "contribution from an unnamed venue: {}",
                c.venue.as_str()
            );
            assert!(c.contributed(), "a fresh member was unexpectedly excluded");
        }

        // The engine reproduced the analytic BBO exactly.
        assert_eq!(
            book.best_bid.to_bits(),
            want_bid.to_bits(),
            "best bid for {}",
            bond.cusip
        );
        assert_eq!(
            book.best_offer.to_bits(),
            want_offer.to_bits(),
            "best offer for {}",
            bond.cusip
        );
        // Composite mid inside the surviving-member envelope (convex combination).
        assert!(
            book.composite_mid >= min_mid && book.composite_mid <= max_mid,
            "composite mid {} outside [{min_mid}, {max_mid}] for {}",
            book.composite_mid,
            bond.cusip
        );
        assert!((0.0..=1.0).contains(&book.confidence));
        // Prices are oracle-anchored near par (a Treasury trades close to 100).
        assert!(
            book.composite_mid > 50.0 && book.composite_mid < 160.0,
            "composite mid {} not a sane Treasury price for {}",
            book.composite_mid,
            bond.cusip
        );
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn a_stale_lp_sim_member_drops_out_of_the_composite() {
    let cfg = cfg();
    let bonds = selection(&cfg);
    let bond = bonds.first().cloned().expect("a modellable bond");
    let instr = bond.engine_instrument();

    // Build the panel, then freeze the last member's feed 200 s ago (past cutoff).
    let mut lps = build_fleet(&cfg, &bonds);
    let stale_venue = lps.last().unwrap().venue().clone();
    let last = lps.pop().unwrap().with_fault(Fault::Stale {
        frozen_at_nanos: NOW - 200 * S,
    });
    lps.push(last);
    let feeds = into_feeds(lps);

    let ccfg = cfg.consolidation();
    // Ground truth: max/min over the fresh members only (all but the stale one).
    let fresh: Vec<_> = feeds
        .iter()
        .filter_map(|f| f.top_of_book(&instr, NOW))
        .filter(|q| q.ts > NOW - 100 * S) // fresh members report a near-NOW ts
        .collect();
    let want_bid = fresh
        .iter()
        .map(|q| q.bid)
        .fold(f64::NEG_INFINITY, f64::max);
    let want_offer = fresh.iter().map(|q| q.offer).fold(f64::INFINITY, f64::min);

    let book = ConsolidatedBook::consolidate(&feeds, &instr, NOW, &ccfg).unwrap();

    let stale = book
        .contributions
        .iter()
        .find(|c| c.venue == stale_venue)
        .expect("the stale member is reported");
    assert_eq!(stale.excluded, Some(ExclusionReason::Stale));
    assert!(!stale.contributed());
    assert_eq!(
        book.contributing(),
        cfg.members - 1,
        "stale member excluded"
    );
    assert_eq!(book.best_bid.to_bits(), want_bid.to_bits());
    assert_eq!(book.best_offer.to_bits(), want_offer.to_bits());
}
