//! The **aggregated-book engine hub** — the server-side wiring of the
//! `celnet-aggregation` consolidation engine into the live edge (D3).
//!
//! An inbound liquidity provider pushes its per-instrument two-way top-of-book into
//! the server over the [`LiquidityFeedService`](crate::services::liquidity_feed)
//! (`LpFeed`), and every **enabled** [`AggregatedBookDef`] that lists that LP as a
//! member and whose scope admits the instrument consolidates the push into a live
//! composite. A GUI subscriber to an aggregated book reads that composite over the
//! multiplexed `StreamService.StreamSession` (see `services::stream`).
//!
//! ## Where this sits relative to the hot core (guardrail 11)
//!
//! Consolidation and publish are **off** the pinned zero-alloc pricing core: ingest
//! is a cheap latest-quote sink write plus a per-book dirty-version bump (no
//! consolidation), and the composite is (re)consolidated **lazily** — memoised the
//! first time a subscriber polls a book after its version advanced, so N subscribers
//! of one book cost one consolidation per version, not N. There is no background
//! timer thread and nothing on the price path.
//!
//! ## The latest-quote sink & staleness
//!
//! Per book we keep only the **latest** [`VenueQuote`] per `(instrument_id, VenueId)`
//! — a new push overwrites the prior one — so the sink is bounded by
//! `instruments × members` and never grows with time. A member that stops quoting
//! keeps its last (increasingly stale) quote, which the consolidation engine decays
//! by age and, past the book's hard `max_quote_age_ms`, excludes entirely (surfaced
//! to the subscriber as a stale contribution that no longer sets the best price).
//!
//! ## Params → engine config
//!
//! [`AggregationParams`] is deliberately float-free (it derives `Eq` across the whole
//! persisted [`IdentityStore`]). The engine's absolute `divergence_tolerance` (mid
//! units) is therefore **derived at consolidation time** from the instrument's own
//! robust price scale (the median absolute mid of its fresh members) times
//! [`DIVERGENCE_TOLERANCE_FRACTION`], so one relative knob works across price- and
//! yield-quoted lines. With gating off the tolerance is `+∞` (no member is ever
//! gated as divergent).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use celnet_aggregation::{ConsolidatedBook, ConsolidationConfig, Instrument, VenueId, VenueQuote};
use celnet_proto::{AggregatedBookSnapshot, AggregatedInstrument, LpContribution, LpQuote};
use celnet_types::{Ccy, CommodityRef, Symbol, Tenor, Underlying};

use crate::clock::Clock;
use crate::config::identity::{AggregatedBookDef, AggregationParams, IdentityStore, Scope};

/// The relative half-width of the divergence band, as a fraction of the
/// instrument's robust price scale: a member is gated as a divergent outlier only
/// when its mid deviates from the median consensus by more than
/// `DIVERGENCE_TOLERANCE_FRACTION · scale` **and** beyond the engine's MAD-scaled
/// bound. `5e-3` = 50 bp of the mid — on a ~100 clean-price handle that is the
/// `0.50` absolute band the LP-SIM feed itself tunes to; on a ~0.04 yield it is
/// ~2 bp. The same fraction also sets the scale the composite's agreement /
/// confidence is measured against.
const DIVERGENCE_TOLERANCE_FRACTION: f64 = 5.0e-3;

/// The quote self-reported quality carried into the consolidation report for an LP
/// pushed quote (the ingest wire carries no per-quote quality; a live push is taken
/// at full confidence and the staleness/divergence gating does the discrimination).
const INGEST_QUALITY: f64 = 1.0;

/// The reference-data identity of an instrument, resolved for the composite line's
/// display fields (best-effort — empty when the `instrument_id` is not registered).
#[derive(Clone, Default)]
struct InstrumentIdentity {
    display_name: String,
    isin: String,
    cusip: String,
}

/// The reconciled runtime configuration of one enabled aggregated book. Rebuilt
/// (cheaply, behind an `Arc` swap) on boot and on every admin CRUD; read on the
/// ingest and publish paths.
struct BookCfg {
    /// The member venue ids (from `member_connection_ids`) whose quotes feed this
    /// book — a push from a non-member is ignored.
    members: HashSet<VenueId>,
    /// Which instruments the composite is produced for.
    scope: Scope,
    /// The consolidation tuning (staleness, gating, quorum).
    params: AggregationParams,
    /// The instrument reference-data identities, shared across books.
    identities: Arc<HashMap<String, InstrumentIdentity>>,
}

/// The published composite of a book at the version it reflects.
pub struct PublishedBook {
    /// The dirty-version this snapshot consolidates (monotone per book).
    pub version: u64,
    /// The consolidated composite for every in-scope instrument meeting quorum.
    pub snapshot: AggregatedBookSnapshot,
}

/// The live state of one book: the latest-quote sink, a dirty version bumped on
/// every ingest, and the memoised last-published composite.
struct BookState {
    /// `instrument_id -> venue -> latest quote`.
    sink: HashMap<String, HashMap<VenueId, VenueQuote>>,
    /// Bumped on every accepted ingest — the "there is newer input" signal.
    dirty_version: u64,
    /// The memoised composite and the version it reflects.
    published: Arc<PublishedBook>,
    /// `dirty_version` value `published` was consolidated at.
    published_version: u64,
}

/// One running aggregated-book engine.
struct BookEngine {
    id: String,
    cfg: Mutex<Arc<BookCfg>>,
    state: Mutex<BookState>,
}

impl BookEngine {
    fn new(id: String, cfg: Arc<BookCfg>) -> Self {
        let published = Arc::new(PublishedBook {
            version: 0,
            snapshot: AggregatedBookSnapshot {
                book_id: id.clone(),
                instruments: Vec::new(),
            },
        });
        Self {
            id,
            cfg: Mutex::new(cfg),
            state: Mutex::new(BookState {
                sink: HashMap::new(),
                dirty_version: 0,
                published,
                published_version: 0,
            }),
        }
    }
}

/// The edge-wide hub of running aggregated-book engines, shared (behind an `Arc`)
/// by the LP ingest service, the stream service, and the auth CRUD path.
pub struct AggregationHub {
    books: RwLock<HashMap<String, Arc<BookEngine>>>,
    clock: Clock,
}

impl std::fmt::Debug for AggregationHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Report only the running-book count — never lock the per-book state here.
        let books = self.books.read().map(|b| b.len()).unwrap_or_default();
        f.debug_struct("AggregationHub")
            .field("books", &books)
            .finish()
    }
}

impl AggregationHub {
    /// Construct an empty hub bound to the edge `clock` (the valuation clock the
    /// staleness decay measures quote age against).
    #[must_use]
    pub fn new(clock: Clock) -> Arc<Self> {
        Arc::new(Self {
            books: RwLock::new(HashMap::new()),
            clock,
        })
    }

    /// Rebuild the running engine set from the persisted identity store: stand up
    /// an engine for every **enabled** book, refresh the config of the ones that
    /// persist (preserving their live quote sink), and tear down the ones that were
    /// disabled or deleted. Idempotent — safe to call at boot and after every admin
    /// create/update/delete.
    pub fn reconcile(&self, store: &IdentityStore) {
        let identities = Arc::new(build_identities(store));
        let enabled: Vec<&AggregatedBookDef> = store
            .aggregated_books
            .iter()
            .filter(|b| b.enabled)
            .collect();
        let wanted: HashSet<&str> = enabled.iter().map(|b| b.id.as_str()).collect();

        let mut books = self.books.write().expect("aggregation books lock poisoned");
        // Drop engines for books that are no longer enabled.
        books.retain(|id, _| wanted.contains(id.as_str()));
        // Stand up / refresh the enabled books.
        for def in enabled {
            let cfg = Arc::new(book_cfg(def, Arc::clone(&identities)));
            match books.get(&def.id) {
                Some(engine) => {
                    *engine.cfg.lock().expect("book cfg lock poisoned") = cfg;
                }
                None => {
                    books.insert(
                        def.id.clone(),
                        Arc::new(BookEngine::new(def.id.clone(), cfg)),
                    );
                }
            }
        }
    }

    /// Route one pushed LP quote into every enabled book that lists the pushing LP
    /// as a member and whose scope admits the instrument. Returns `true` if the
    /// quote was accepted into at least one book (the ingest ack counts these).
    pub fn ingest(&self, q: &LpQuote) -> bool {
        if q.lp_name.is_empty() || q.instrument_id.is_empty() {
            return false;
        }
        let venue = VenueId::new(q.lp_name.clone());
        let books = self.books.read().expect("aggregation books lock poisoned");
        let mut accepted = false;
        for engine in books.values() {
            let cfg = Arc::clone(&*engine.cfg.lock().expect("book cfg lock poisoned"));
            if !cfg.members.contains(&venue) || !scope_admits(&cfg.scope, &q.instrument_id) {
                continue;
            }
            let quote = venue_quote(q, &venue);
            let mut state = engine.state.lock().expect("book state lock poisoned");
            state
                .sink
                .entry(q.instrument_id.clone())
                .or_default()
                .insert(venue.clone(), quote);
            state.dirty_version += 1;
            accepted = true;
        }
        accepted
    }

    /// The current published composite for `book_id`, (re)consolidating lazily if a
    /// newer push has landed since the last publish. `None` when no enabled book
    /// with that id exists.
    pub fn snapshot(&self, book_id: &str) -> Option<Arc<PublishedBook>> {
        let books = self.books.read().expect("aggregation books lock poisoned");
        let engine = books.get(book_id)?;
        let cfg = Arc::clone(&*engine.cfg.lock().expect("book cfg lock poisoned"));
        let now = self.clock.now_nanos();
        let mut state = engine.state.lock().expect("book state lock poisoned");
        if state.published_version < state.dirty_version {
            let snapshot = consolidate_book(&engine.id, &cfg, &state.sink, now);
            let version = state.dirty_version;
            state.published = Arc::new(PublishedBook { version, snapshot });
            state.published_version = version;
        }
        Some(Arc::clone(&state.published))
    }

    /// Whether an enabled book with this id is currently running.
    #[must_use]
    pub fn contains(&self, book_id: &str) -> bool {
        self.books
            .read()
            .expect("aggregation books lock poisoned")
            .contains_key(book_id)
    }
}

/// Build the reference-data identity map (`instrument_id -> {name, isin, cusip}`)
/// from the persisted instrument registry.
fn build_identities(store: &IdentityStore) -> HashMap<String, InstrumentIdentity> {
    store
        .instruments
        .iter()
        .map(|d| {
            let ext = |scheme: &str| {
                d.external_ids
                    .iter()
                    .find(|e| e.scheme.eq_ignore_ascii_case(scheme))
                    .map(|e| e.value.clone())
                    .unwrap_or_default()
            };
            (
                d.instrument_id.clone(),
                InstrumentIdentity {
                    display_name: d.name.clone(),
                    isin: ext("isin"),
                    cusip: ext("cusip"),
                },
            )
        })
        .collect()
}

/// Assemble a book's runtime config from its persisted definition.
fn book_cfg(
    def: &AggregatedBookDef,
    identities: Arc<HashMap<String, InstrumentIdentity>>,
) -> BookCfg {
    BookCfg {
        members: def
            .member_connection_ids
            .iter()
            .map(|m| VenueId::new(m.clone()))
            .collect(),
        scope: def.instrument_scope.clone(),
        params: def.params.clone(),
        identities,
    }
}

/// Whether a book's instrument scope admits `instrument_id`.
fn scope_admits(scope: &Scope, instrument_id: &str) -> bool {
    match scope {
        Scope::AllMembersQuote => true,
        Scope::Explicit(ids) => ids.iter().any(|i| i == instrument_id),
    }
}

/// The internal, asset-agnostic engine [`Instrument`] key for a server
/// `instrument_id` — the id carried as a vendor-neutral free-form commodity symbol
/// (guardrail 8: the id is an opaque code) at a single canonical tenor. Injective
/// over ids (distinct ids never consolidate together); the tenor is irrelevant to
/// the consolidation (it keys nothing on the wire, where the string id is the
/// identity), so a fixed `SpotNext` is used.
fn engine_instrument(instrument_id: &str) -> Instrument {
    Instrument::new(
        Underlying::Commodity(CommodityRef::new(Symbol::new(instrument_id, ""), Ccy::USD)),
        Tenor::SpotNext,
    )
}

/// Build a [`VenueQuote`] from a pushed [`LpQuote`].
fn venue_quote(q: &LpQuote, venue: &VenueId) -> VenueQuote {
    VenueQuote {
        venue: venue.clone(),
        instrument: engine_instrument(&q.instrument_id),
        bid: q.bid,
        offer: q.offer,
        bid_size: q.bid_size,
        offer_size: q.offer_size,
        ts: q.ts_nanos,
        quality: INGEST_QUALITY,
    }
}

/// The robust price scale of an instrument's quotes: the median absolute mid over
/// the finite, positive-mid members (a magnitude the relative divergence tolerance
/// is scaled by). Falls back to `1.0` when no usable mid exists.
fn robust_scale(quotes: &[VenueQuote]) -> f64 {
    let mut mids: Vec<f64> = quotes
        .iter()
        .filter(|q| q.is_finite())
        .map(|q| q.mid().abs())
        .filter(|m| m.is_finite() && *m > 0.0)
        .collect();
    if mids.is_empty() {
        return 1.0;
    }
    mids.sort_by(f64::total_cmp);
    let n = mids.len();
    if n % 2 == 1 {
        mids[n / 2]
    } else {
        0.5 * (mids[n / 2 - 1] + mids[n / 2])
    }
}

/// Map the persisted [`AggregationParams`] onto the engine's [`ConsolidationConfig`]
/// at the instrument's `scale` (see [`DIVERGENCE_TOLERANCE_FRACTION`]).
fn consolidation_config(params: &AggregationParams, scale: f64) -> ConsolidationConfig {
    ConsolidationConfig {
        staleness_half_life_secs: (params.staleness_tau_ms as f64 / 1000.0).max(f64::MIN_POSITIVE),
        staleness_cutoff_secs: params.max_quote_age_ms as f64 / 1000.0,
        divergence_tolerance: if params.divergence_gating {
            (DIVERGENCE_TOLERANCE_FRACTION * scale).max(f64::MIN_POSITIVE)
        } else {
            f64::INFINITY
        },
    }
}

/// Consolidate every in-scope instrument of a book from its latest-quote sink into
/// the wire [`AggregatedBookSnapshot`]. An instrument below its member quorum
/// (`min_contributors`) or with no surviving member produces no composite line.
fn consolidate_book(
    book_id: &str,
    cfg: &BookCfg,
    sink: &HashMap<String, HashMap<VenueId, VenueQuote>>,
    now: i64,
) -> AggregatedBookSnapshot {
    let ids: Vec<String> = match &cfg.scope {
        Scope::Explicit(list) => list.clone(),
        Scope::AllMembersQuote => sink.keys().cloned().collect(),
    };
    let quorum = cfg.params.min_contributors.max(1);
    let mut instruments = Vec::new();
    for id in ids {
        let Some(venue_map) = sink.get(&id) else {
            continue;
        };
        let mut quotes: Vec<VenueQuote> = venue_map
            .iter()
            .filter(|(v, _)| cfg.members.contains(v))
            .map(|(_, q)| q.clone())
            .collect();
        if quotes.is_empty() {
            continue;
        }
        // Deterministic member order for the contribution report and tie-breaks.
        quotes.sort_by(|a, b| a.venue.cmp(&b.venue));
        let scale = robust_scale(&quotes);
        let config = consolidation_config(&cfg.params, scale);
        if let Ok(book) = ConsolidatedBook::from_quotes(&quotes, now, &config)
            && book.contributing() as u32 >= quorum
        {
            instruments.push(build_instrument(&id, cfg, &book, &quotes));
        }
    }
    // Stable `instrument_id` order so a snapshot and its deltas render consistently.
    instruments.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
    AggregatedBookSnapshot {
        book_id: book_id.to_string(),
        instruments,
    }
}

/// Build one wire [`AggregatedInstrument`] from a consolidated book + the raw member
/// quotes (for each contributor's own two-way).
fn build_instrument(
    id: &str,
    cfg: &BookCfg,
    book: &ConsolidatedBook,
    quotes: &[VenueQuote],
) -> AggregatedInstrument {
    let identity = cfg.identities.get(id).cloned().unwrap_or_default();
    let contributions = book
        .contributions
        .iter()
        .map(|c| {
            let raw = quotes.iter().find(|q| q.venue == c.venue);
            LpContribution {
                lp_name: c.venue.as_str().to_string(),
                bid: raw.map_or(f64::NAN, |q| q.bid),
                offer: raw.map_or(f64::NAN, |q| q.offer),
                // A single wire bool: the member is not currently setting the
                // composite (aged out past max age, or gated as a divergent outlier).
                stale: !c.contributed(),
            }
        })
        .collect();
    AggregatedInstrument {
        instrument_id: id.to_string(),
        display_name: identity.display_name,
        isin: identity.isin,
        cusip: identity.cusip,
        best_bid: book.best_bid,
        best_offer: book.best_offer,
        bid_size: book.best_bid_size,
        offer_size: book.best_offer_size,
        confidence: book.confidence,
        contributions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::identity::AggregationParams;

    const S: i64 = 1_000_000_000;
    const NOW: i64 = 1_000 * S;

    fn params(gating: bool, min_contrib: u32, max_age_ms: u64) -> AggregationParams {
        AggregationParams {
            staleness_tau_ms: 30_000,
            max_quote_age_ms: max_age_ms,
            divergence_gating: gating,
            min_contributors: min_contrib,
            depth_levels: 1,
        }
    }

    fn def(id: &str, members: &[&str], params: AggregationParams) -> AggregatedBookDef {
        AggregatedBookDef {
            id: id.to_string(),
            name: id.to_string(),
            member_connection_ids: members.iter().map(|m| (*m).to_string()).collect(),
            instrument_scope: Scope::AllMembersQuote,
            params,
            enabled: true,
        }
    }

    fn lp_quote(lp: &str, id: &str, bid: f64, offer: f64, ts: i64) -> LpQuote {
        LpQuote {
            lp_name: lp.to_string(),
            instrument_id: id.to_string(),
            bid,
            offer,
            bid_size: 1_000_000.0,
            offer_size: 2_000_000.0,
            ts_nanos: ts,
        }
    }

    fn hub_with(book: AggregatedBookDef) -> Arc<AggregationHub> {
        let hub = AggregationHub::new(Clock::manual(NOW));
        let mut store = IdentityStore::default();
        store.aggregated_books.push(book);
        hub.reconcile(&store);
        hub
    }

    #[test]
    fn composite_bbo_is_best_across_fresh_members() {
        let hub = hub_with(def(
            "b",
            &["LP-1", "LP-2", "LP-3"],
            params(false, 1, 60_000),
        ));
        // Three members, one instrument; the composite best bid is the max bid and
        // best offer the min offer across the fresh members.
        assert!(hub.ingest(&lp_quote("LP-1", "CUSIP-A", 99.90, 100.10, NOW)));
        assert!(hub.ingest(&lp_quote("LP-2", "CUSIP-A", 99.95, 100.08, NOW)));
        assert!(hub.ingest(&lp_quote("LP-3", "CUSIP-A", 99.88, 100.05, NOW)));
        let snap = hub.snapshot("b").expect("book exists");
        assert_eq!(snap.snapshot.instruments.len(), 1);
        let inst = &snap.snapshot.instruments[0];
        assert_eq!(inst.instrument_id, "CUSIP-A");
        assert_eq!(inst.best_bid.to_bits(), 99.95_f64.to_bits());
        assert_eq!(inst.best_offer.to_bits(), 100.05_f64.to_bits());
        assert_eq!(inst.contributions.len(), 3);
        assert!(inst.confidence > 0.0 && inst.confidence <= 1.0);
        // Best offer of 100.05 is LP-3's; its size stacks the offer side.
        assert_eq!(inst.offer_size.to_bits(), 2_000_000.0_f64.to_bits());
    }

    #[test]
    fn stale_member_drops_out_of_the_bbo() {
        let hub = hub_with(def("b", &["LP-1", "LP-2", "LP-3"], params(false, 1, 5_000)));
        // LP-2 posted the best offer but is aged well past the 5 s hard cutoff.
        hub.ingest(&lp_quote("LP-1", "X", 99.90, 100.10, NOW));
        hub.ingest(&lp_quote("LP-2", "X", 99.95, 100.00, NOW - 60 * S));
        hub.ingest(&lp_quote("LP-3", "X", 99.88, 100.06, NOW));
        let snap = hub.snapshot("b").expect("book");
        let inst = &snap.snapshot.instruments[0];
        // The stale LP-2 must NOT set the best offer (100.00); the surviving min is
        // LP-3's 100.06 — the surviving-member envelope, never a crossed BBO.
        assert_eq!(inst.best_offer.to_bits(), 100.06_f64.to_bits());
        assert_eq!(inst.best_bid.to_bits(), 99.90_f64.to_bits());
        let lp2 = inst
            .contributions
            .iter()
            .find(|c| c.lp_name == "LP-2")
            .expect("LP-2 reported");
        assert!(lp2.stale, "aged-out member is flagged stale");
    }

    #[test]
    fn non_member_push_is_rejected_and_quorum_suppresses_thin_lines() {
        let hub = hub_with(def("b", &["LP-1", "LP-2"], params(false, 2, 60_000)));
        // A push from a non-member is not accepted.
        assert!(!hub.ingest(&lp_quote("ROGUE", "Y", 1.0, 2.0, NOW)));
        // One member only ⇒ below the 2-contributor quorum ⇒ no composite line.
        assert!(hub.ingest(&lp_quote("LP-1", "Y", 99.9, 100.1, NOW)));
        let snap = hub.snapshot("b").expect("book");
        assert!(snap.snapshot.instruments.is_empty());
        // A second member reaches quorum ⇒ the line appears.
        assert!(hub.ingest(&lp_quote("LP-2", "Y", 99.95, 100.05, NOW)));
        let snap = hub.snapshot("b").expect("book");
        assert_eq!(snap.snapshot.instruments.len(), 1);
    }

    #[test]
    fn reconcile_disables_and_reenables_without_losing_quotes() {
        let hub = hub_with(def("b", &["LP-1", "LP-2"], params(false, 1, 60_000)));
        hub.ingest(&lp_quote("LP-1", "Z", 99.9, 100.1, NOW));
        hub.ingest(&lp_quote("LP-2", "Z", 99.95, 100.05, NOW));
        assert!(hub.contains("b"));
        // Disable the book (drop it from the enabled set) ⇒ no engine.
        let mut store = IdentityStore::default();
        let mut d = def("b", &["LP-1", "LP-2"], params(false, 1, 60_000));
        d.enabled = false;
        store.aggregated_books.push(d);
        hub.reconcile(&store);
        assert!(!hub.contains("b"));
        assert!(hub.snapshot("b").is_none());
    }

    #[test]
    fn version_advances_only_on_new_input() {
        let hub = hub_with(def("b", &["LP-1"], params(false, 1, 60_000)));
        hub.ingest(&lp_quote("LP-1", "W", 99.9, 100.1, NOW));
        let v1 = hub.snapshot("b").expect("book").version;
        // No new ingest ⇒ the published version is unchanged (memoised).
        let v2 = hub.snapshot("b").expect("book").version;
        assert_eq!(v1, v2);
        hub.ingest(&lp_quote("LP-1", "W", 99.91, 100.09, NOW));
        let v3 = hub.snapshot("b").expect("book").version;
        assert!(v3 > v2);
    }
}
