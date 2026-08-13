//! The **inbound-liquidity panel** fold — a per-liquidity-provider view of what
//! each LP is actually feeding the platform, and how much of the composite it
//! actually drives.
//!
//! ## Why this lives here and not in a new store
//!
//! Everything the panel reports is ALREADY held by the hub; nothing new is
//! recorded on the ingest path (guardrail 11 — the pinned core stays
//! alloc/lock/log-free and this fold never runs on it):
//!
//! | Panel column | Existing source |
//! |---|---|
//! | roster (which LPs exist at all) | each enabled book's `BookCfg::members` |
//! | lifetime quote-update count | the hub's `lp_ticks` tally |
//! | last quote / price / size | the per-book latest-quote sink `BookState::sink` |
//! | fresh vs stale, weight, deviation, top-of-book | one consolidation pass |
//!
//! The consolidation pass is the same computation the composite publish path
//! already performs (`super::consolidate_book`); the panel re-runs it here so it
//! can read the ENGINE-level [`VenueContribution`](celnet_aggregation::VenueContribution)
//! (weight / age / deviation / exclusion reason) rather than the deliberately
//! lossy `LpContribution` the composite wire carries. It runs on query only — an
//! admin screen polling a few times a minute, never a price tick.
//!
//! ## Provider identity
//!
//! One string is three things at once: an `AggregatedBookDef`'s
//! `member_connection_ids` entry, the aggregation `VenueId`, and the `lp_name`
//! every inbound `LpQuote` carries. [`AggregationHub::ingest`] matches them by
//! plain string equality, so the panel keys on the same value and a member id
//! that names no managed connection is reported as exactly that — an
//! unresolvable member, which is the misconfiguration this screen exists to make
//! visible.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use celnet_aggregation::{ConsolidatedBook, ExclusionReason, VenueQuote};

use super::{AggregationHub, consolidation_config, robust_scale};

/// One inbound liquidity provider's live feed health, folded across every enabled
/// aggregated book that lists it as a member.
///
/// Purely a report — the service layer maps this onto the wire
/// `LiquidityProviderDesc` and joins the connection-registry identity (name,
/// desk, enabled/running) onto it, which the hub deliberately knows nothing
/// about.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LpFeedHealth {
    /// The provider identity — the connection id / `VenueId` / `lp_name`.
    pub connection_id: String,
    /// The enabled aggregated books listing this provider as a member, sorted.
    pub book_ids: Vec<String>,
    /// Lifetime accepted quote-update pushes from this provider.
    pub quote_updates: u64,
    /// Most recent observation instant across its live quotes (epoch ns); `0` ⇒
    /// it has never quoted.
    pub last_quote_nanos: i64,
    /// Distinct instruments it currently shows a live two-way on.
    pub instruments_quoted: u32,
    /// Live quotes currently contributing to a composite.
    pub fresh_quotes: u32,
    /// Live quotes currently excluded (stale, divergent or non-finite).
    pub stale_quotes: u32,
    /// Instruments where it is setting the consolidated best bid.
    pub best_bid_count: u32,
    /// Instruments where it is setting the consolidated best offer.
    pub best_offer_count: u32,
    /// Mean normalized composite weight over its CONTRIBUTING quotes, `∈ [0, 1]`.
    /// `0` when it contributes nothing (the mean is over contributors only, so a
    /// provider excluded everywhere reads `0` rather than a diluted average).
    pub mean_weight: f64,
}

/// One live quote a provider is showing, with the consolidation verdict reached
/// on it — the per-provider drill-down row.
#[derive(Debug, Clone, PartialEq)]
pub struct LpFeedQuote {
    /// The aggregated book this quote was consolidated into.
    pub book_id: String,
    /// The canonical server `instrument_id`.
    pub instrument_id: String,
    /// Reference-data label for the instrument (empty when unresolved).
    pub display_name: String,
    /// The provider's own bid.
    pub bid: f64,
    /// The provider's own offer.
    pub offer: f64,
    /// Firm size at its bid.
    pub bid_size: f64,
    /// Firm size at its offer.
    pub offer_size: f64,
    /// The quote's observation instant (epoch ns).
    pub ts_nanos: i64,
    /// The quote's age in seconds at the valuation instant.
    pub age_secs: f64,
    /// Final normalized weight in the composite mid (`0` when excluded).
    pub weight: f64,
    /// Absolute deviation from the robust consensus, in mid units.
    pub deviation: f64,
    /// Why it was excluded, or empty when it contributed.
    pub excluded: String,
    /// Whether it is setting the consolidated best bid.
    pub best_bid: bool,
    /// Whether it is setting the consolidated best offer.
    pub best_offer: bool,
}

/// The whole inbound-liquidity panel at one valuation instant.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LpPanel {
    /// One row per provider, in `connection_id` order.
    pub providers: Vec<LpFeedHealth>,
    /// The focused provider's live quotes, in (`book_id`, `instrument_id`) order.
    /// Empty when the fold was not given a focus.
    pub quotes: Vec<LpFeedQuote>,
    /// Whether the firm-wide inbound ingest kill-switch is enabled.
    pub inbound_enabled: bool,
    /// The valuation instant the fold ran at (epoch ns).
    pub as_of_nanos: i64,
}

/// The wire spelling of an [`ExclusionReason`] — a stable lower-snake token the
/// client renders, rather than a `Debug` format that would drift with the enum.
fn exclusion_token(reason: ExclusionReason) -> &'static str {
    match reason {
        ExclusionReason::NonFinite => "non_finite",
        ExclusionReason::Stale => "stale",
        ExclusionReason::Divergent => "divergent",
    }
}

/// Whether `price` is the consolidated best — an exact comparison, because the
/// consolidated best bid/offer IS a copy of some surviving member's own price
/// (the max bid / min offer), never a derived quantity. Ties therefore credit
/// EVERY member quoting that level, which is the honest reading of "who is on
/// top of book". A non-finite best (no surviving member) credits nobody.
fn sets_best(price: f64, best: f64) -> bool {
    best.is_finite() && price == best
}

/// The per-provider accumulator, folded across books then finalized.
#[derive(Default)]
struct Acc {
    book_ids: BTreeSet<String>,
    last_quote_nanos: i64,
    instruments: BTreeSet<String>,
    fresh_quotes: u32,
    stale_quotes: u32,
    best_bid_count: u32,
    best_offer_count: u32,
    /// Sum of contributing weights, and how many contributed — the mean is taken
    /// over CONTRIBUTORS only (see [`LpFeedHealth::mean_weight`]).
    weight_sum: f64,
    weight_n: u32,
}

impl AggregationHub {
    /// Fold the inbound-liquidity panel at the current valuation instant.
    ///
    /// `focus` names the provider whose per-instrument drill-down to include;
    /// `None` returns the roster alone, so the panel's default poll stays bounded
    /// by the provider count rather than instruments × providers.
    ///
    /// Every enabled book is visited once, its latest-quote sink consolidated
    /// once, and the resulting contribution report attributed to its provider. A
    /// book member that has never quoted still gets a row (all-zero counters) — a
    /// silent LP is precisely what an operator opens this screen to find.
    #[must_use]
    pub fn liquidity_panel(&self, focus: Option<&str>) -> LpPanel {
        let now = self.clock.now_nanos();
        let ticks = self.lp_tick_counts();
        let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
        let mut quotes: Vec<LpFeedQuote> = Vec::new();

        {
            let books = self.books.read().expect("aggregation books lock poisoned");
            for (book_id, engine) in books.iter() {
                let cfg = Arc::clone(&*engine.cfg.lock().expect("book cfg lock poisoned"));
                // The roster first: every member of an enabled book gets a row even
                // if it has never pushed a quote.
                for venue in &cfg.members {
                    acc.entry(venue.as_str().to_string())
                        .or_default()
                        .book_ids
                        .insert(book_id.clone());
                }

                let state = engine.state.lock().expect("book state lock poisoned");
                for (instrument_id, venue_map) in &state.sink {
                    // The sink is already scope-filtered (ingest drops an
                    // out-of-scope instrument), so member filtering is the only
                    // narrowing left — the same one `consolidate_book` applies.
                    let mut member_quotes: Vec<VenueQuote> = venue_map
                        .iter()
                        .filter(|(venue, _)| cfg.members.contains(venue))
                        .map(|(_, quote)| quote.clone())
                        .collect();
                    if member_quotes.is_empty() {
                        continue;
                    }
                    // Deterministic member order — the same tie-break basis the
                    // composite publish path uses.
                    member_quotes.sort_by(|a, b| a.venue.cmp(&b.venue));

                    // Liveness is read from the RAW sink, independent of quorum: an
                    // instrument below its book's `min_contributors` publishes no
                    // composite line, but the LP is still quoting it and the panel
                    // must say so.
                    for quote in &member_quotes {
                        let row = acc.entry(quote.venue.as_str().to_string()).or_default();
                        row.last_quote_nanos = row.last_quote_nanos.max(quote.ts);
                        row.instruments.insert(instrument_id.clone());
                    }

                    let config = consolidation_config(&cfg.params, robust_scale(&member_quotes));
                    let consolidated =
                        match ConsolidatedBook::from_quotes(&member_quotes, now, &config) {
                            Ok(book) => book,
                            Err(_) => {
                                // No consolidated book at all — every member has aged
                                // out or is otherwise unusable. Each quote is still
                                // LIVE in the sink and still excluded from the
                                // composite, so it must be reported as excluded
                                // rather than vanish: a silent drop makes a whole book
                                // going stale read as "no quotes excluded", which is
                                // the opposite of what happened.
                                for quote in &member_quotes {
                                    acc.entry(quote.venue.as_str().to_string())
                                        .or_default()
                                        .stale_quotes += 1;
                                }
                                continue;
                            }
                        };
                    for contribution in &consolidated.contributions {
                        let venue = contribution.venue.as_str();
                        let raw = member_quotes.iter().find(|q| q.venue == contribution.venue);
                        let contributed = contribution.contributed();
                        let best_bid = contributed
                            && raw.is_some_and(|q| sets_best(q.bid, consolidated.best_bid));
                        let best_offer = contributed
                            && raw.is_some_and(|q| sets_best(q.offer, consolidated.best_offer));

                        let row = acc.entry(venue.to_string()).or_default();
                        if contributed {
                            row.fresh_quotes += 1;
                            row.weight_sum += contribution.weight;
                            row.weight_n += 1;
                        } else {
                            row.stale_quotes += 1;
                        }
                        row.best_bid_count += u32::from(best_bid);
                        row.best_offer_count += u32::from(best_offer);

                        if focus == Some(venue) {
                            quotes.push(LpFeedQuote {
                                book_id: book_id.clone(),
                                instrument_id: instrument_id.clone(),
                                display_name: cfg
                                    .identities
                                    .get(instrument_id)
                                    .map(|i| i.display_name.clone())
                                    .unwrap_or_default(),
                                bid: raw.map_or(f64::NAN, |q| q.bid),
                                offer: raw.map_or(f64::NAN, |q| q.offer),
                                bid_size: raw.map_or(0.0, |q| q.bid_size),
                                offer_size: raw.map_or(0.0, |q| q.offer_size),
                                ts_nanos: raw.map_or(0, |q| q.ts),
                                age_secs: contribution.age_secs,
                                weight: contribution.weight,
                                deviation: contribution.deviation,
                                excluded: contribution
                                    .excluded
                                    .map(exclusion_token)
                                    .unwrap_or_default()
                                    .to_string(),
                                best_bid,
                                best_offer,
                            });
                        }
                    }
                }
            }
        }

        // A provider that has pushed but belongs to no enabled book still gets a
        // row: its pushes are being dropped at ingest and the panel must not hide
        // that behind an absent line.
        for lp_name in ticks.keys() {
            acc.entry(lp_name.clone()).or_default();
        }

        let providers = acc
            .into_iter()
            .map(|(connection_id, a)| LpFeedHealth {
                quote_updates: ticks.get(&connection_id).copied().unwrap_or(0),
                book_ids: a.book_ids.into_iter().collect(),
                last_quote_nanos: a.last_quote_nanos,
                instruments_quoted: u32::try_from(a.instruments.len()).unwrap_or(u32::MAX),
                fresh_quotes: a.fresh_quotes,
                stale_quotes: a.stale_quotes,
                best_bid_count: a.best_bid_count,
                best_offer_count: a.best_offer_count,
                mean_weight: if a.weight_n == 0 {
                    0.0
                } else {
                    a.weight_sum / f64::from(a.weight_n)
                },
                connection_id,
            })
            .collect();

        quotes.sort_by(|a, b| {
            a.book_id
                .cmp(&b.book_id)
                .then_with(|| a.instrument_id.cmp(&b.instrument_id))
        });

        LpPanel {
            providers,
            quotes,
            inbound_enabled: self.pricing_control.inbound_enabled(),
            as_of_nanos: now,
        }
    }
}
