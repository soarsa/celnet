//! End-to-end check that the **Treasury futures complex actually reaches an
//! aggregated book** — the property the whole futures-venue slice exists for.
//!
//! A DV01-ratio hedge on a corporate position is expressed in a benchmark Treasury
//! future. The server seeds those contracts into its tradeable reference registry, so
//! if no LP feed **quotes** them they never reach an aggregated book,
//! `AggregationHub::best_fill` finds no composite line, and every futures hedge
//! backstops to the synthetic COMPOSITE venue. Exactly that tradeable-but-unquotable
//! asymmetry already caused a production defect on the freshly-auctioned Treasuries.
//!
//! So this test walks the real path, with no mocks anywhere:
//!
//! 1. the sim's quotable set is built from the SAME `celnet-refdata` universes the
//!    server seeds its registry from, and must contain every listed contract;
//! 2. an operator-style all-members aggregated book resolves a streaming plan that
//!    includes every contract for every member;
//! 3. one streaming round produces well-formed, on-tick-grid, uncrossed two-ways;
//! 4. the REAL `celnet_aggregation::ConsolidatedBook` engine folds the panel into a
//!    composite that is **not crossed** — a crossed composite is rejected by the
//!    server's RFQ resolver, which would starve the hedge the venue exists to fill;
//! 5. the composite that comes out is usable as a hedge: the DV01-ratio contract
//!    count off the contract's derived DV01 is the ratio in the requirement.

use std::collections::{BTreeMap, BTreeSet};

use celnet_aggregation::ConsolidatedBook;
use celnet_lp_sim::net::plan_quotes_round;
use celnet_lp_sim::{
    FaultSchedule, LpSimConfig, QuotedLine, build_fleet, into_feeds, load_futures_universe,
    load_government_universe, quotable_lines, resolve_from_descs,
};
use celnet_proto::{AggregatedBookDesc, AggregationScopeMode};
use celnet_types::BrokenDate;

const S: i64 = 1_000_000_000;
const NOW: i64 = 100 * S;

/// A 5-member panel — the deployed shape, and enough members for the consolidator's
/// median-consensus divergence gating to be decidable.
fn cfg() -> LpSimConfig {
    LpSimConfig {
        members: 5,
        seed: 0xFEED_1234,
        settlement: BrokenDate::new(2026, 4, 16),
        ..LpSimConfig::default()
    }
}

fn members() -> BTreeSet<String> {
    (1..=5).map(|i| format!("LP-SIM-{i:02}")).collect()
}

/// The listed contract codes, straight from reference data.
fn contract_codes() -> Vec<String> {
    celnet_refdata::treasury_futures_universe()
        .into_iter()
        .map(|s| s.instrument_id)
        .collect()
}

/// Just the futures lines of the sim's quotable set.
fn futures_only(lines: &[QuotedLine]) -> Vec<QuotedLine> {
    let codes: BTreeSet<String> = contract_codes().into_iter().collect();
    lines
        .iter()
        .filter(|l| codes.contains(&l.instrument_id))
        .cloned()
        .collect()
}

#[test]
fn the_quotable_set_covers_every_tradeable_futures_contract() {
    let cfg = cfg();
    let lines = quotable_lines(&cfg, &load_government_universe(false));
    let quotable: BTreeSet<&str> = lines.iter().map(QuotedLine::instrument_id).collect();

    let codes = contract_codes();
    assert!(!codes.is_empty(), "reference data lists no futures");
    for code in &codes {
        assert!(
            quotable.contains(code.as_str()),
            "{code} is tradeable (the server seeds it) but NOT quotable — it would \
             never reach an aggregated book and every hedge on it would backstop to \
             the synthetic COMPOSITE venue"
        );
    }
    // The cash universe is untouched by the extension.
    assert!(
        lines.len() > codes.len(),
        "the cash bond lines must still be there"
    );
}

#[test]
fn an_all_members_book_plans_and_streams_every_contract() {
    let cfg = cfg();
    let lines = quotable_lines(&cfg, &load_government_universe(false));
    let futures = futures_only(&lines);
    assert_eq!(futures.len(), contract_codes().len());

    let priceable: BTreeSet<String> = lines.iter().map(|l| l.instrument_id.clone()).collect();
    let books = vec![AggregatedBookDesc {
        id: "ust-composite".to_string(),
        name: "UST composite".to_string(),
        member_connection_ids: members().into_iter().collect(),
        scope_mode: AggregationScopeMode::AllMembersQuote as i32,
        instrument_ids: Vec::new(),
        params: None,
        enabled: true,
    }];
    let plan = resolve_from_descs(&books, &members(), &priceable);

    for line in &futures {
        for m in members() {
            assert!(
                plan.contains(&m, &line.instrument_id),
                "{m} does not stream {}",
                line.instrument_id
            );
        }
    }

    // One streaming round over the futures lines only, faults off so every member is
    // fresh and the two-ways are the analytic ones.
    let fleet = build_fleet(&cfg, &futures);
    let by_id: BTreeMap<&str, &QuotedLine> = futures
        .iter()
        .map(|l| (l.instrument_id.as_str(), l))
        .collect();
    let futures_plan = {
        let ids: BTreeSet<String> = futures.iter().map(|l| l.instrument_id.clone()).collect();
        resolve_from_descs(&books, &members(), &ids)
    };
    let quotes = plan_quotes_round(
        &cfg,
        &fleet,
        &by_id,
        &futures_plan,
        NOW,
        0,
        &FaultSchedule::off(),
    );
    assert_eq!(quotes.len(), futures.len() * 5);

    let specs: BTreeMap<String, celnet_refdata::TreasuryFutureSpec> =
        celnet_refdata::treasury_futures_universe()
            .into_iter()
            .map(|s| (s.instrument_id.clone(), s))
            .collect();
    for q in &quotes {
        let spec = &specs[&q.instrument_id];
        let tick = spec.terms.tick_size_points;
        assert!(q.bid.is_finite() && q.offer.is_finite());
        assert!(
            q.offer >= q.bid,
            "{}: a member's own two-way is crossed",
            q.instrument_id
        );
        for px in [q.bid, q.offer] {
            let ticks = px / tick;
            assert!(
                (ticks - ticks.round()).abs() < 1e-6,
                "{}: {px} is off the published tick grid ({tick})",
                q.instrument_id
            );
        }
        // A member's market is at most a handful of ticks wide — a listed contract,
        // not a cash-bond two-way.
        assert!(
            q.offer - q.bid <= 3.0 * tick + 1e-12,
            "{}: {} ticks wide is not a listed market",
            q.instrument_id,
            (q.offer - q.bid) / tick
        );
        assert_eq!(q.ts_nanos, NOW);
    }
}

#[test]
fn the_real_engine_consolidates_every_contract_to_an_uncrossed_composite() {
    let cfg = cfg();
    let lines = futures_only(&quotable_lines(&cfg, &load_government_universe(false)));
    let feeds = into_feeds(build_fleet(&cfg, &lines));
    let ccfg = cfg.consolidation();
    let specs: BTreeMap<String, celnet_refdata::TreasuryFutureSpec> =
        celnet_refdata::treasury_futures_universe()
            .into_iter()
            .map(|s| (s.instrument_id.clone(), s))
            .collect();

    for line in &lines {
        let spec = &specs[&line.instrument_id];
        let tick = spec.terms.tick_size_points;
        let book = ConsolidatedBook::consolidate(&feeds, &line.instrument, NOW, &ccfg)
            .unwrap_or_else(|e| panic!("{}: no composite: {e}", line.instrument_id));

        // THE property: a listed panel must not produce a crossed composite. The
        // server's `resolve_rfq_composite` rejects `best_bid > best_offer` outright,
        // so a crossed line is an unfillable hedge.
        assert!(
            book.best_bid <= book.best_offer,
            "{}: CROSSED composite — bid {} > offer {}",
            line.instrument_id,
            book.best_bid,
            book.best_offer
        );
        // ...and it is a real, tight, listed-looking market.
        assert!(
            book.best_offer - book.best_bid <= 3.0 * tick + 1e-12,
            "{}: composite {} ticks wide",
            line.instrument_id,
            (book.best_offer - book.best_bid) / tick
        );
        assert_eq!(
            book.contributions.len(),
            5,
            "{}: every member should contribute",
            line.instrument_id
        );
        assert!(book.contributions.iter().all(|c| c.excluded.is_none()));

        // The analytic BBO ground truth over the members' own top-of-book.
        let quotes: Vec<_> = feeds
            .iter()
            .filter_map(|f| f.top_of_book(&line.instrument, NOW))
            .collect();
        let want_bid = quotes.iter().map(|q| q.bid).fold(f64::MIN, f64::max);
        let want_offer = quotes.iter().map(|q| q.offer).fold(f64::MAX, f64::min);
        assert!(
            (book.best_bid - want_bid).abs() < 1e-9,
            "{}",
            line.instrument_id
        );
        assert!(
            (book.best_offer - want_offer).abs() < 1e-9,
            "{}",
            line.instrument_id
        );
    }
}

#[test]
fn a_composite_futures_line_sizes_a_dv01_hedge() {
    // The requirement's worked example: a $25,000/bp corporate book hedged in the
    // front 10-Year contract. The contract count must be the DV01 ratio computed off
    // the SAME derived DV01 the reference data publishes, at the SAME cash-curve
    // yield the feed prices the contract at.
    let cfg = cfg();
    let bonds = load_government_universe(false);
    let contracts = load_futures_universe(&bonds, cfg.settlement);
    let front = contracts
        .iter()
        .find(|c| c.spec.terms.symbol == "ZN")
        .expect("a 10-Year contract in the universe");

    let contract_dv01 = front.dv01_per_contract().expect("derives");
    // Anchored to the real cash curve, this must land in the plausible band for a
    // 10-Year Note contract's basis-point value — tens of dollars, not hundreds.
    assert!(
        (40.0..120.0).contains(&contract_dv01),
        "implausible 10-Year contract DV01 {contract_dv01}"
    );

    let portfolio_dv01 = 25_000.0;
    let want = (portfolio_dv01 / contract_dv01).round() as i64;
    let got = front
        .spec
        .hedge_contracts(portfolio_dv01, front.reference_yield)
        .expect("sizes");
    assert_eq!(got, want);
    assert!(got > 0, "a long-duration book sells futures");

    // And the contract the hedge would fill against is quoted, on its grid, in the
    // exchange's points-and-32nds convention.
    let px = front.reference_price().expect("prices");
    let rendered = front
        .spec
        .format_price_32nds(front.spec.round_bid_to_tick(px));
    assert!(
        rendered.contains('\''),
        "a futures price renders in points and 32nds, got {rendered}"
    );
}
