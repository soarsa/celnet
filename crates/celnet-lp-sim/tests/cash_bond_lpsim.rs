//! End-to-end check that the **cash-bond panel consolidates to a fillable composite**
//! — the OTC counterpart of `futures_lpsim.rs`, and the property the cash feed's
//! market-structure model exists for.
//!
//! ## The defect this pins
//!
//! The cash feed used to draw each member's market **level** independently. A yield
//! draw becomes a price displacement of the bond's DV01 times that yield move, so on
//! anything past the front end the members' mids ended up one to two orders of
//! magnitude further apart than the two-way they quoted around them. The consolidated
//! `best_bid = max(bids)` then routinely printed through `best_offer = min(offers)`:
//! measured at **58–93% of cash lines crossed**, worsening as the panel grew.
//!
//! A crossed composite is not cosmetic. `AggregationHub::resolve_rfq_composite`
//! rejects one outright, so the book stops quoting that instrument outbound, and any
//! consumer that (wrongly) routes through the composite guard to reach the member
//! panel stops filling on it too.
//!
//! Note what the fix is **not**: divergence gating cannot be the lever here. The
//! measurement above got *worse* at 3, 4 and 5 members — panel sizes where the
//! consolidator's median-consensus gate is decidable — because a MAD-scaled gate
//! deliberately does not flag a wide-but-mutually-consistent panel. The members were
//! not outliers; the panel was uniformly, legitimately dispersed. The defect was in
//! the feed's market model.
//!
//! ## The market-structure model asserted here
//!
//! A cash bond is OTC, so this is **not** the listed model wholesale: there is no
//! exchange print and no tick grid, and each member keeps its own spread width, size,
//! re-quote cadence and wandering private mark. What it does share with the listed
//! case is that the **level is one observable** — a dealer in a benchmark government
//! bond marks off the same inter-dealer level as its competitors. Cross-member
//! differentiation is carried by a deliberate lean, a fixed private view and a
//! wandering private view, each sized off the bond's **own DV01** and budgeted
//! strictly inside the tightest member's half-spread.
//!
//! So the assertions are analytic, not plausibility checks:
//!
//! 1. every cash line's per-member displacement budget provably fits inside the
//!    tightest member's half-spread (the never-crossed condition, in closed form);
//! 2. the REAL `celnet_aggregation::ConsolidatedBook` engine folds the whole cash
//!    universe into an **uncrossed** composite, at every panel size the platform
//!    deploys and across time — including the two-member book that sits below the
//!    divergence-gating quorum, where the consolidator has no gate to fall back on;
//! 3. the composite is the analytic BBO of the members' own two-ways (bit-exact), so
//!    the fix tightened the panel rather than papering over the fold;
//! 4. the panel is still a real multi-dealer panel — the members differ, the level
//!    moves, and the best-bid winner rotates, which is what the street-side LP
//!    analytics league table measures.

use std::collections::BTreeSet;

use celnet_aggregation::ConsolidatedBook;
use celnet_lp_sim::{
    LpSimConfig, QuotedLine, build_fleet, into_feeds, load_government_universe, quotable_lines,
};

const S: i64 = 1_000_000_000;

/// The member-panel sizes the platform deploys: the two-member book that is below the
/// consolidator's `MIN_SOURCES_FOR_GATING` quorum, up to the full named roster.
const PANEL_SIZES: [usize; 3] = [2, 3, 4];

/// The instants each line is consolidated at — spread across the shared market-level
/// cadence and each member's own re-quote cadence so the panel is sampled both
/// mid-tick and on a boundary.
const INSTANTS: [i64; 6] = [0, S / 4, S / 2, 7 * S, 100 * S, 613 * S];

/// The tightest counterparty's half-spread as a multiple of the fleet base, read off
/// the ROSTER rather than hard-coded: each profile's spread multiple is jittered by at
/// most ±10% (see `SimLpProfile::half_spread`), so the narrowest two-way any member
/// can show is `min(spread_multiple) × 0.9`, and every per-member displacement budget
/// must fit inside it or the panel's best bid prints through its best offer.
fn tightest_member_half_spread() -> f64 {
    celnet_lp_sim::OTC_ROSTER
        .iter()
        .map(|p| p.spread_multiple)
        .fold(f64::INFINITY, f64::min)
        * 0.9
}

fn cfg(members: usize) -> LpSimConfig {
    LpSimConfig {
        members,
        seed: 0xC0FF_EE01,
        ..LpSimConfig::default()
    }
}

/// The cash-bond lines of the sim's quotable set (the listed contracts, which quote on
/// a tick grid, are covered by `futures_lpsim.rs`; the swap/OIS curve points, which are
/// also off-grid but quote a par RATE with no bond or DV01 behind them, are covered by
/// `celnet_lp_sim::ois`). Selecting on the mid arm — not on `tick.is_none()` — is what
/// keeps this a cash-bond test now that an off-grid line is no longer necessarily a bond.
fn cash_lines(cfg: &LpSimConfig) -> Vec<QuotedLine> {
    quotable_lines(cfg, &load_government_universe(false))
        .into_iter()
        .filter(|l| l.tick.is_none() && l.mid.yield_model().is_some())
        .collect()
}

#[test]
fn every_cash_line_budgets_its_dispersion_inside_the_tightest_half_spread() {
    let cfg = cfg(5);
    let lines = cash_lines(&cfg);
    assert!(
        !lines.is_empty(),
        "no modellable cash bonds in the universe"
    );

    for line in &lines {
        // The line quotes at the fleet's cash width, off any grid.
        assert_eq!(
            line.tick, None,
            "{}: a cash bond is not gridded",
            line.instrument_id
        );
        let half_spread = cfg.half_spread * line.spread_scale;
        let model = line
            .mid
            .yield_model()
            .unwrap_or_else(|| panic!("{}: not a bond-yield line", line.instrument_id));

        // The bond's own DV01 per 100 face, from the real analytics leaf — the
        // conversion between the yield the model is driven in and the price the
        // crossing happens in.
        let dv01 = celnet_bond::dv01(&model.bond, celnet_types::Rate(model.initial_yield))
            .unwrap_or_else(|e| panic!("{}: no DV01: {e}", line.instrument_id));
        assert!(dv01 > 0.0, "{}: non-positive DV01", line.instrument_id);
        let to_price = |y: f64| y * dv01 / 1.0e-4;

        // The three per-member displacements, at the outermost member of the panel.
        let peak_lean = (cfg.panel_size() as f64 - 1.0) / 2.0 * cfg.skew_step * line.lean_scale;
        let peak_view = to_price(
            line.yield_dispersion
                .unwrap_or_else(|| panic!("{}: no sized dispersion", line.instrument_id)),
        );
        let peak_wander = to_price(line.dealer_view);
        assert!(
            peak_wander > 0.0,
            "{}: an OTC member with no private view of its own would never rotate the \
             best-price winner",
            line.instrument_id
        );

        // THE never-crossed condition. Two members' mids differ by at most twice this
        // sum; their best bid reaches their best offer only once that difference
        // exceeds the two half-spreads they quote around it, i.e. at least twice the
        // tightest member's. Holding the sum below one tightest half-spread therefore
        // makes crossing impossible for ANY pair, at any instant.
        let displacement = peak_lean + peak_view + peak_wander;
        assert!(
            displacement < tightest_member_half_spread() * half_spread,
            "{}: peak member displacement {displacement} would cross a {} half-spread \
             (lean {peak_lean}, view {peak_view}, wander {peak_wander})",
            line.instrument_id,
            tightest_member_half_spread() * half_spread,
        );
    }
}

#[test]
fn the_real_engine_consolidates_every_cash_bond_to_an_uncrossed_composite() {
    for members in PANEL_SIZES {
        let cfg = cfg(members);
        let lines = cash_lines(&cfg);
        let feeds = into_feeds(build_fleet(&cfg, &lines));
        let ccfg = cfg.consolidation();

        for line in &lines {
            for now in INSTANTS {
                let book = ConsolidatedBook::consolidate(&feeds, &line.instrument, now, &ccfg)
                    .unwrap_or_else(|e| {
                        panic!(
                            "{} @ {now} ({members} members): no composite: {e}",
                            line.instrument_id
                        )
                    });

                // THE property. `resolve_rfq_composite` rejects `best_bid > best_offer`
                // outright, so a crossed line is an unquotable instrument.
                assert!(
                    book.best_bid <= book.best_offer,
                    "{} @ {now} ({members} members): CROSSED composite — bid {} > offer {}",
                    line.instrument_id,
                    book.best_bid,
                    book.best_offer
                );
                // ...and it is a real two-way, not a degenerate zero-width one.
                assert!(
                    book.best_offer - book.best_bid > 0.0,
                    "{} @ {now}: locked composite",
                    line.instrument_id
                );
                // Every fresh member contributes: the panel is tightened by the market
                // model, NOT by gating members out of it.
                assert_eq!(
                    book.contributing(),
                    members,
                    "{} @ {now}: a member was excluded — the composite must be uncrossed \
                     because the panel agrees, not because members were dropped",
                    line.instrument_id
                );

                // The analytic BBO ground truth over the members' own top-of-book.
                let quotes: Vec<_> = feeds
                    .iter()
                    .filter_map(|f| f.top_of_book(&line.instrument, now))
                    .collect();
                let want_bid = quotes
                    .iter()
                    .map(|q| q.bid)
                    .fold(f64::NEG_INFINITY, f64::max);
                let want_offer = quotes.iter().map(|q| q.offer).fold(f64::INFINITY, f64::min);
                assert_eq!(
                    book.best_bid.to_bits(),
                    want_bid.to_bits(),
                    "{} @ {now}: best bid is not the max member bid",
                    line.instrument_id
                );
                assert_eq!(
                    book.best_offer.to_bits(),
                    want_offer.to_bits(),
                    "{} @ {now}: best offer is not the min member offer",
                    line.instrument_id
                );
                // Oracle-anchored: a government bond prices near par.
                assert!(
                    book.composite_mid > 50.0 && book.composite_mid < 200.0,
                    "{} @ {now}: implausible composite mid {}",
                    line.instrument_id,
                    book.composite_mid
                );
            }
        }
    }
}

#[test]
fn the_panel_is_still_a_competing_multi_dealer_panel() {
    let cfg = cfg(5);
    let lines = cash_lines(&cfg);
    let feeds = into_feeds(build_fleet(&cfg, &lines));

    // Sharing the market level must not collapse the panel into one repeated quote:
    // the members still show their own two-ways...
    let probe = &lines[lines.len() / 2];
    let quotes: Vec<_> = feeds
        .iter()
        .filter_map(|f| f.top_of_book(&probe.instrument, 7 * S))
        .collect();
    assert_eq!(quotes.len(), cfg.panel_size());
    let distinct_bids: BTreeSet<u64> = quotes.iter().map(|q| q.bid.to_bits()).collect();
    assert_eq!(
        distinct_bids.len(),
        cfg.panel_size(),
        "{}: members must not quote one identical bid",
        probe.instrument_id
    );

    // ...the shared level actually moves the market over time...
    let mids: BTreeSet<u64> = INSTANTS
        .iter()
        .filter_map(|t| feeds[0].top_of_book(&probe.instrument, *t))
        .map(|q| q.mid().to_bits())
        .collect();
    assert!(
        mids.len() > 1,
        "{}: the shared market level never moves",
        probe.instrument_id
    );

    // ...and the best-bid winner rotates over time on the great majority of the book,
    // which is what makes the street-side LP league table a measurement rather than a
    // constant. (A handful of lines are legitimately owned by one dealer throughout —
    // the member showing the tightest spread on them wins every round.)
    let mut rotating = 0usize;
    let mut winners: BTreeSet<String> = BTreeSet::new();
    for line in &lines {
        let per_line: BTreeSet<String> = (0..24_i64)
            .filter_map(|k| {
                feeds
                    .iter()
                    .filter_map(|f| f.top_of_book(&line.instrument, k * S / 3))
                    .max_by(|a, b| a.bid.total_cmp(&b.bid))
                    .map(|q| q.venue.as_str().to_string())
            })
            .collect();
        if per_line.len() > 1 {
            rotating += 1;
        }
        winners.extend(per_line);
    }
    // Under the NAMED roster the touch is won on price and the clip is won on size,
    // and those are deliberately different counterparties. A wide principal dealer
    // does not win the touch — it charges for balance sheet and then provides it —
    // so asserting "every member wins the best bid somewhere" would be asserting
    // that the roster has no real personalities. What must hold is that the touch is
    // genuinely CONTESTED (more than one winner across the book) and that the widest
    // quoter is compensated by being the deepest.
    assert!(
        winners.len() > 1,
        "the best bid is owned by a single counterparty across the whole book: {winners:?}"
    );
    let widest = celnet_lp_sim::OTC_ROSTER
        .iter()
        .max_by(|a, b| a.spread_multiple.total_cmp(&b.spread_multiple))
        .expect("non-empty roster");
    let deepest = celnet_lp_sim::OTC_ROSTER
        .iter()
        .max_by(|a, b| {
            let depth = |p: &celnet_lp_sim::SimLpProfile| {
                p.size_multiple * (1.0 - p.depth_decay.powi(i32::from(p.depth_levels) + 1))
            };
            depth(a).total_cmp(&depth(b))
        })
        .expect("non-empty roster");
    assert_eq!(
        widest.id, deepest.id,
        "the widest quoter ({}) is not the deepest ({}) — a counterparty that wins \
         neither the touch nor the clip has no reason to be on the panel",
        widest.id, deepest.id
    );
    assert!(
        rotating * 4 >= lines.len() * 3,
        "the best-bid winner rotates on only {rotating}/{} lines — the panel is too \
         static to measure LP competition on",
        lines.len()
    );
}
