//! Integration test for the **book resolver** — the pure core of the book-aware
//! feed. Given a set of `AggregatedBookDesc`s (as returned by
//! `AuthService.ListAggregatedBooks`), the set of `LP-SIM-0N` members this process
//! impersonates, and the instruments it can price, the resolver must produce exactly
//! the right `(LP member × instrument)` stream set, ignore books with no LP-SIM
//! member (and disabled books), and update correctly as books are added, removed, and
//! edited.
//!
//! The final case wires the resolved plan through [`plan_quotes_round`] and the REAL
//! [`celnet_aggregation`] consolidation engine, and validates the composite against
//! the surviving members' analytic envelope (best-bid = max bid, best-offer = min
//! offer, composite mid inside the members' mid range) — never a plausibility check,
//! and never asserting an uncrossed BBO (a max-bid/min-offer composite may legitimately
//! cross).

use std::collections::BTreeSet;

use celnet_aggregation::{ConsolidatedBook, VenueFeed};
use celnet_lp_sim::net::plan_quotes_round;
use celnet_lp_sim::{
    FaultSchedule, LpSimConfig, StreamPlan, TreasuryBond, build_fleet, into_feeds,
    load_coupon_universe, resolve_from_descs,
};
use celnet_proto::{AggregatedBookDesc, AggregationScopeMode};
use celnet_types::BrokenDate;

const S: i64 = 1_000_000_000;
const NOW: i64 = 100 * S;

/// The five members the sim impersonates (`LP-SIM-01`…`LP-SIM-05`).
fn impersonated() -> BTreeSet<String> {
    let cfg = cfg();
    (0..cfg.members).map(|i| cfg.member_venue(i)).collect()
}

fn cfg() -> LpSimConfig {
    LpSimConfig {
        members: 5,
        settlement: BrokenDate::new(2026, 4, 16),
        ..LpSimConfig::default()
    }
}

/// The first `n` modellable coupon Treasuries at the config settlement.
fn selection(n: usize) -> Vec<TreasuryBond> {
    let c = cfg();
    load_coupon_universe()
        .into_iter()
        .filter(|b| {
            b.yield_model(c.settlement, c.reversion_per_sec, c.perturbation)
                .is_some()
        })
        .take(n)
        .collect()
}

fn ids_of(bonds: &[TreasuryBond]) -> BTreeSet<String> {
    bonds
        .iter()
        .map(|b| b.instrument_id().to_string())
        .collect()
}

/// Build a wire book descriptor for the resolver.
fn book(
    id: &str,
    enabled: bool,
    members: &[&str],
    mode: AggregationScopeMode,
    instrument_ids: &[&str],
) -> AggregatedBookDesc {
    AggregatedBookDesc {
        id: id.to_string(),
        name: id.to_string(),
        member_connection_ids: members.iter().map(|s| (*s).to_string()).collect(),
        scope_mode: mode as i32,
        instrument_ids: instrument_ids.iter().map(|s| (*s).to_string()).collect(),
        params: None,
        enabled,
    }
}

#[test]
fn resolves_the_exact_member_by_instrument_stream_set() {
    let bonds = selection(3);
    assert_eq!(bonds.len(), 3, "need three modellable coupon bonds");
    let ids: Vec<String> = bonds
        .iter()
        .map(|b| b.instrument_id().to_string())
        .collect();
    let priceable = ids_of(&bonds);

    let books = vec![
        // An all-members-quote book whose LP-SIM members are LP-SIM-01..03.
        book(
            "ust-all",
            true,
            &["LP-SIM-01", "LP-SIM-02", "LP-SIM-03"],
            AggregationScopeMode::AllMembersQuote,
            &[],
        ),
        // An explicit-scope book whose members are LP-SIM-04..05, over two instruments.
        book(
            "ust-explicit",
            true,
            &["LP-SIM-04", "LP-SIM-05"],
            AggregationScopeMode::Explicit,
            &[&ids[0], &ids[1]],
        ),
        // A book with NO LP-SIM member — must be ignored entirely.
        book(
            "external",
            true,
            &["FIX-BOX-CELER", "API-LP-7"],
            AggregationScopeMode::AllMembersQuote,
            &[],
        ),
        // A disabled book naming an LP-SIM member — must be ignored.
        book(
            "disabled",
            false,
            &["LP-SIM-01"],
            AggregationScopeMode::AllMembersQuote,
            &[],
        ),
    ];

    let plan = resolve_from_descs(&books, &impersonated(), &priceable);

    // ust-all: 3 members × 3 instruments = 9; ust-explicit: 2 members × 2 = 4.
    assert_eq!(
        plan.len(),
        9 + 4,
        "exact (member × instrument) stream count"
    );

    // All-members-quote fans LP-SIM-01..03 over every priceable instrument.
    for m in ["LP-SIM-01", "LP-SIM-02", "LP-SIM-03"] {
        for id in &ids {
            assert!(plan.contains(m, id), "{m} must quote {id}");
        }
    }
    // Explicit scope restricts LP-SIM-04..05 to the two listed instruments only.
    for m in ["LP-SIM-04", "LP-SIM-05"] {
        assert!(plan.contains(m, &ids[0]));
        assert!(plan.contains(m, &ids[1]));
        assert!(
            !plan.contains(m, &ids[2]),
            "{m} must NOT quote the unlisted {}",
            ids[2]
        );
    }
    // The external book's members never appear.
    assert!(!plan.members().contains("FIX-BOX-CELER"));
    assert!(!plan.members().contains("API-LP-7"));
    // The disabled book contributes nothing beyond what ust-all already gave.
    assert_eq!(
        plan.members(),
        BTreeSet::from([
            "LP-SIM-01",
            "LP-SIM-02",
            "LP-SIM-03",
            "LP-SIM-04",
            "LP-SIM-05",
        ])
    );
}

#[test]
fn plan_updates_when_books_are_added_removed_and_edited() {
    let bonds = selection(3);
    let ids: Vec<String> = bonds
        .iter()
        .map(|b| b.instrument_id().to_string())
        .collect();
    let priceable = ids_of(&bonds);
    let members = impersonated();

    // Start with only ignored books → an empty plan.
    let start = resolve_from_descs(
        &[book(
            "external",
            true,
            &["OTHER"],
            AggregationScopeMode::AllMembersQuote,
            &[],
        )],
        &members,
        &priceable,
    );
    assert!(start.is_empty());

    // ADD an all-members book (LP-SIM-01..03).
    let added = resolve_from_descs(
        &[book(
            "ust-all",
            true,
            &["LP-SIM-01", "LP-SIM-02", "LP-SIM-03"],
            AggregationScopeMode::AllMembersQuote,
            &[],
        )],
        &members,
        &priceable,
    );
    let d = start.diff(&added);
    assert_eq!(d.added.len(), 9, "3 members × 3 instruments started");
    assert!(d.removed.is_empty());

    // EDIT: same book but drop a member and switch to explicit single-instrument scope.
    let edited = resolve_from_descs(
        &[book(
            "ust-all",
            true,
            &["LP-SIM-01", "LP-SIM-02"],
            AggregationScopeMode::Explicit,
            &[&ids[0]],
        )],
        &members,
        &priceable,
    );
    let d = added.diff(&edited);
    // Now only LP-SIM-01/02 × ids[0] survive (2 streams); the rest are removed.
    assert_eq!(edited.len(), 2);
    assert!(edited.contains("LP-SIM-01", &ids[0]));
    assert!(!edited.contains("LP-SIM-03", &ids[0]));
    assert_eq!(d.added.len(), 0);
    assert_eq!(d.removed.len(), 9 - 2);

    // REMOVE the book entirely → back to empty, everything removed.
    let removed = StreamPlan::default();
    let d = edited.diff(&removed);
    assert!(d.added.is_empty());
    assert_eq!(d.removed.len(), 2);
}

#[test]
fn priced_streams_stay_within_the_surviving_member_envelope() {
    let cfg = cfg();
    let bonds = selection(1);
    assert_eq!(bonds.len(), 1);
    let bond = &bonds[0];
    let priceable = ids_of(&bonds);

    // An all-members book naming all five LP-SIM members over this one instrument.
    let books = vec![book(
        "ust-one",
        true,
        &[
            "LP-SIM-01",
            "LP-SIM-02",
            "LP-SIM-03",
            "LP-SIM-04",
            "LP-SIM-05",
        ],
        AggregationScopeMode::AllMembersQuote,
        &[],
    )];
    let plan = resolve_from_descs(&books, &impersonated(), &priceable);
    assert_eq!(plan.len(), 5, "five members quote the one instrument");

    // Emit one round with faults OFF so every member is fresh (analytic envelope).
    let by_cusip = bonds.iter().map(|b| (b.instrument_id(), b)).collect();
    let fleet = build_fleet(&cfg, &bonds);
    let quotes = plan_quotes_round(
        &cfg,
        &fleet,
        &by_cusip,
        &plan,
        NOW,
        0,
        &FaultSchedule::off(),
    );
    assert_eq!(quotes.len(), 5);
    // Each LP's OWN two-way is uncrossed (offer ≥ bid); the composite BBO may cross.
    for q in &quotes {
        assert!(q.offer >= q.bid, "an LP's own two-way must not be crossed");
        assert_eq!(
            q.ts_nanos, NOW,
            "a fresh (unfaulted) member reports the query ts"
        );
    }

    // Analytic envelope over the five members' own top-of-book at NOW.
    let instrument = bond.engine_instrument();
    let bids: Vec<f64> = fleet
        .iter()
        .map(|m| m.top_of_book(&instrument, NOW).unwrap().bid)
        .collect();
    let offers: Vec<f64> = fleet
        .iter()
        .map(|m| m.top_of_book(&instrument, NOW).unwrap().offer)
        .collect();
    let mids: Vec<f64> = fleet
        .iter()
        .map(|m| m.top_of_book(&instrument, NOW).unwrap().mid())
        .collect();
    let max_bid = bids.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_offer = offers.iter().copied().fold(f64::INFINITY, f64::min);
    let min_mid = mids.iter().copied().fold(f64::INFINITY, f64::min);
    let max_mid = mids.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    // The REAL consolidation engine over the same fleet must reproduce the envelope.
    let feeds = into_feeds(fleet);
    let book =
        ConsolidatedBook::consolidate(&feeds, &instrument, NOW, &cfg.consolidation()).unwrap();
    assert_eq!(
        book.best_bid.to_bits(),
        max_bid.to_bits(),
        "best-bid = max surviving bid"
    );
    assert_eq!(
        book.best_offer.to_bits(),
        min_offer.to_bits(),
        "best-offer = min surviving offer"
    );
    assert!(
        book.composite_mid >= min_mid && book.composite_mid <= max_mid,
        "composite mid {} outside surviving envelope [{min_mid}, {max_mid}]",
        book.composite_mid
    );
    assert_eq!(book.contributing(), 5, "all five fresh members contribute");
}
