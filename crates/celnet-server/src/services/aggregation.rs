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
use celnet_bond::{Bond, accrued_interest, bond_risk};
use celnet_proto::{
    AggregatedBookSnapshot, AggregatedInstrument, EspOrRfq, LpContribution, LpQuote,
    PricingProvenance,
};
use celnet_rates::{AccrualBasis, PaymentFrequency};
use celnet_tiering::{FeatureKind, FeaturePipeline, PricedResult, PricingCtx, QuoteCtx, TwoWay};
use celnet_types::{Ccy, CommodityRef, Symbol, Tenor, Underlying};

use crate::clock::Clock;
use crate::config::identity::{
    AggregatedBookDef, AggregationParams, IdentityStore, PricingGroupResolver, Scope,
};
use crate::config::reference_data::{
    InstrumentFamily, accrual_basis_from_label, payment_frequency_from_label,
};

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

/// A live source of signed net inventory per instrument, read by a pricing group's
/// inventory-sensitive features (Inventory-skew / Position) to skew a client's outbound
/// two-way toward shedding risk (a **long** book skews **down**).
///
/// This is the seam between the aggregation publish path and the firm's FI position
/// book: `net_inventory` returns the dealer's signed position (long positive) for the
/// aggregated-book `instrument_id`, or `0.0` when the book holds none / no source is
/// wired. Kept a trait so the hub reads inventory without depending on any concrete
/// position store, and so a test can seed a deterministic book.
pub trait InventorySource: Send + Sync + std::fmt::Debug {
    /// The signed net position (long positive) for `instrument_id`; `0.0` if none.
    fn net_inventory(&self, instrument_id: &str) -> f64;
}

/// A concrete, in-memory [`InventorySource`]: signed net position per `instrument_id`.
///
/// The live inventory book the streaming-skew strategy reads. Positions are set/adjusted
/// by the FI booking path as deals lift the composite (the RFQ/auto-quote booking wiring
/// is the Phase 2b follow-up); until then a book with no recorded position nets to `0.0`,
/// so an inventory-skew feature contributes zero skew and the outbound two-way is
/// byte-identical to the flat markup alone.
#[derive(Debug, Default)]
pub struct InstrumentInventory {
    net: RwLock<HashMap<String, f64>>,
}

impl InstrumentInventory {
    /// An empty inventory book (every instrument nets to `0.0`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the signed net position for `instrument_id` (long positive), replacing any
    /// prior value.
    pub fn set(&self, instrument_id: impl Into<String>, qty: f64) {
        self.net
            .write()
            .expect("inventory lock poisoned")
            .insert(instrument_id.into(), qty);
    }

    /// Adjust the signed net position for `instrument_id` by `delta` (a booked fill).
    pub fn add(&self, instrument_id: &str, delta: f64) {
        *self
            .net
            .write()
            .expect("inventory lock poisoned")
            .entry(instrument_id.to_string())
            .or_insert(0.0) += delta;
    }
}

impl InventorySource for InstrumentInventory {
    fn net_inventory(&self, instrument_id: &str) -> f64 {
        self.net
            .read()
            .expect("inventory lock poisoned")
            .get(instrument_id)
            .copied()
            .unwrap_or(0.0)
    }
}

/// The static bond-pricing terms needed to derive an instrument's **DV01** for a
/// yield-bps pricing-group feature — the reference-data [`BondDef`] mapped onto
/// the `celnet-bond` leaf's contract inputs. Captured at reconcile (the terms are
/// static); DV01 itself is solved per quote from the live composite mid (see
/// [`bond_dv01`]). Only bonds whose coupon-frequency and day-count labels resolve to an
/// engine convention get an entry — a bond quoting `act_act` (no engine basis yet) has
/// no terms, so a yield-bps feature for it suppresses rather than publishes a bad price.
///
/// [`BondDef`]: crate::config::reference_data::BondDef
#[derive(Clone, Copy, Debug)]
struct BondPricingTerms {
    maturity: time::Date,
    coupon_rate: f64,
    frequency: PaymentFrequency,
    day_count: AccrualBasis,
    redemption: f64,
}

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
    /// The static bond-pricing terms per instrument, for deriving DV01 on a
    /// yield-bps pricing-group feature. Shared across books (built from the registry).
    bond_terms: Arc<HashMap<String, BondPricingTerms>>,
}

/// The published composite of a book at the version it reflects.
pub struct PublishedBook {
    /// The dirty-version this snapshot consolidates (monotone per book).
    pub version: u64,
    /// The raw consolidated composite subscribers receive. A book publishes its raw
    /// consolidated composite unchanged; per-client outbound pricing is applied by the
    /// pricing-group pipelines off this raw composite, never stored on the book.
    pub snapshot: AggregatedBookSnapshot,
}

/// A book composite resolved for an inbound RFQ (Phase 2b): the book it came from, that
/// book's raw consolidated composite two-way for the requested instrument (or the
/// pricing-group-priced two-way on the grouped path), and the book's contributing
/// member-LP lines. The RFQ path prices the single-dealer quote off
/// [`Self::best_bid`]/[`Self::best_offer`] and ranks [`Self::members`] for the
/// multi-dealer panel. Returned by [`AggregationHub::resolve_rfq_composite`].
pub struct RfqComposite {
    /// The id of the book the composite was resolved from (audit / attribution).
    pub book_id: String,
    /// The composite best bid — the outbound price a client SELLs into.
    pub best_bid: f64,
    /// The composite best offer — the outbound price a client BUYs at.
    pub best_offer: f64,
    /// The book's contributing member-LP lines (each LP's own raw two-way + staleness),
    /// for the multi-dealer ranked panel. A stale line is excluded from the panel by the
    /// caller.
    pub members: Vec<RfqMemberLine>,
    /// The per-feature pricing provenance (design §7), `Some` ONLY when the composite
    /// was priced through a pricing group's feature pipeline
    /// ([`AggregationHub::resolve_rfq_composite_priced`]) — the RAW → constructed →
    /// tiered → outbound waterfall the caller stamps onto the `Quote`. `None` for the
    /// ungrouped book-default composite ([`AggregationHub::resolve_rfq_composite`]).
    pub provenance: Option<PricingProvenance>,
}

/// One contributing member-LP line of a resolved [`RfqComposite`]: the LP's venue name,
/// its own raw two-way, and whether it is currently stale (aged out / gated — not setting
/// the composite). The RFQ multi-dealer panel ranks only the fresh member lines.
pub struct RfqMemberLine {
    /// The contributing LP's venue name (its `lp_id` on the ranked panel).
    pub lp_name: String,
    /// The LP's own bid.
    pub bid: f64,
    /// The LP's own offer.
    pub offer: f64,
    /// Whether this contribution is stale (aged out / gated); excluded from the panel.
    pub stale: bool,
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
    /// The optional live inventory source the outbound-tiering skew reads (see
    /// [`InventorySource`]). `None` ⇒ every book nets to zero inventory (no skew).
    inventory: Option<Arc<dyn InventorySource>>,
    /// The **caller → pricing-group** resolver, rebuilt on every [`Self::reconcile`]
    /// from the persisted `pricing_groups` registry and shared read-only across the
    /// per-subscriber ESP path ([`Self::snapshot_priced`]) and the per-caller RFQ path
    /// ([`Self::resolve_rfq_composite_priced`]). An empty resolver (no enabled groups,
    /// the default) means both paths behave exactly as before — the no-group path stays
    /// byte-identical. Behind an `RwLock<Arc<…>>` for the same hot-swap-whole discipline
    /// as each book's `cfg`.
    pricing: RwLock<Arc<PricingGroupResolver>>,
    /// **Street-side LP tick tally** — a bounded per-LP monotonic count of accepted
    /// quote-update pushes, keyed by `lp_name`. This is the tick-rate source for the
    /// LP liquidity analytics fold (`docs/ANALYTICS-REQUIREMENTS.md` §2.4): the sink
    /// only ever keeps the *latest* quote per venue (it overwrites), so the number of
    /// updates cannot be recovered from the sink — this counter records it at the
    /// ingest seam. Off the pinned hot core (ingest already runs on the async feed
    /// edge, guardrail 11); bounded by the member-LP count, never by time. Read
    /// on-query by [`Self::lp_tick_counts`].
    lp_ticks: Mutex<HashMap<String, u64>>,
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

/// The aggregation hub as a **street-side / LP liquidity analytics source**
/// (`docs/ANALYTICS-REQUIREMENTS.md` §2.4): it contributes the per-LP quote-update
/// **tick tally** (the tick-rate seam) to the fold. It holds no RFQ panel history,
/// so it emits no panel records — only the tick counts, snapshotted on-query.
#[tonic::async_trait]
impl crate::services::analytics::lp::LpFlowSource for AggregationHub {
    async fn lp_flow_records(
        &self,
        _from: Option<i64>,
        _to: Option<i64>,
    ) -> Vec<celnet_analytics::LpFlowRecord> {
        Vec::new()
    }

    fn tick_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.lp_tick_counts()
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
            inventory: None,
            pricing: RwLock::new(Arc::new(PricingGroupResolver::default())),
            lp_ticks: Mutex::new(HashMap::new()),
        })
    }

    /// Construct a hub bound to `clock` **and** a live [`InventorySource`] the
    /// outbound-tiering skew reads. Used by the edge boot to wire the FI inventory book
    /// so an [`InventorySkew`](celnet_tiering::InventorySkew) strategy skews on the real
    /// net position; a hub built via [`Self::new`] reads zero inventory (no skew).
    #[must_use]
    pub fn with_inventory(clock: Clock, inventory: Arc<dyn InventorySource>) -> Arc<Self> {
        Arc::new(Self {
            books: RwLock::new(HashMap::new()),
            clock,
            inventory: Some(inventory),
            pricing: RwLock::new(Arc::new(PricingGroupResolver::default())),
            lp_ticks: Mutex::new(HashMap::new()),
        })
    }

    /// Rebuild the running engine set from the persisted identity store: stand up
    /// an engine for every **enabled** book, refresh the config of the ones that
    /// persist (preserving their live quote sink), and tear down the ones that were
    /// disabled or deleted. Idempotent — safe to call at boot and after every admin
    /// create/update/delete.
    pub fn reconcile(&self, store: &IdentityStore) {
        // Rebuild the caller → pricing-group resolver from the persisted registry
        // (hot-swap whole, exactly like each book's cfg). Cheap, and shared read-only
        // by the ESP / RFQ pricing paths until the next reconcile.
        *self.pricing.write().expect("pricing groups lock poisoned") =
            Arc::new(PricingGroupResolver::build(&store.pricing_groups));
        let identities = Arc::new(build_identities(store));
        let bond_terms = Arc::new(build_bond_terms(store));
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
            let cfg = Arc::new(book_cfg(
                def,
                Arc::clone(&identities),
                Arc::clone(&bond_terms),
            ));
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
        // Street-side tick tally: one accepted push = one quote-update from this LP.
        // Counted once per ingest (not per book fed) at the off-core feed edge.
        if accepted {
            *self
                .lp_ticks
                .lock()
                .expect("lp tick tally lock poisoned")
                .entry(q.lp_name.clone())
                .or_insert(0) += 1;
        }
        accepted
    }

    /// A snapshot of the per-LP quote-update tick tally (`lp_name` → count) — the
    /// tick-rate source for the street-side LP liquidity analytics fold. Read
    /// on-query, off the hot path; cloned out under the brief tally lock.
    #[must_use]
    pub fn lp_tick_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.lp_ticks
            .lock()
            .expect("lp tick tally lock poisoned")
            .iter()
            .map(|(lp, &n)| (lp.clone(), n))
            .collect()
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
            // Consolidate and publish the RAW composite (quorum/confidence unchanged).
            // Per-client outbound pricing is applied downstream by the pricing-group
            // pipelines off this raw composite, never stored on the book.
            let raw = consolidate_book(&engine.id, &cfg, &state.sink, now);
            let published = PublishedBook {
                version: state.dirty_version,
                snapshot: raw,
            };
            let version = state.dirty_version;
            state.published = Arc::new(published);
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

    /// Resolve the composite line for `instrument_id` from the first enabled book
    /// (deterministic id order) whose scope admits it **and** that currently publishes a
    /// well-formed composite line for it (quorum met, fresh members, a finite non-crossed
    /// two-way). The returned [`RfqComposite`] carries the book's raw consolidated
    /// two-way and its contributing member lines, so the RFQ path prices against the
    /// book's composite directly (Phase 2b).
    ///
    /// `None` when no book covers the instrument, no covering book has a live composite
    /// line for it (e.g. no fresh member LPs / below quorum), or the composite is
    /// degenerate (non-finite / crossed) — the RFQ path then falls back to the synthetic
    /// demo panel, so nothing regresses when no book is configured.
    #[must_use]
    pub fn resolve_rfq_composite(&self, instrument_id: &str) -> Option<RfqComposite> {
        // Candidate books whose scope admits the id, in deterministic id order (a stable
        // resolution when more than one book admits the same instrument). Collected under
        // a short read lock that is released before consolidating — `snapshot` re-locks,
        // so holding the read guard across it could deadlock.
        let candidates: Vec<String> = {
            let books = self.books.read().expect("aggregation books lock poisoned");
            let mut ids: Vec<String> = books
                .iter()
                .filter(|(_, engine)| {
                    let cfg = engine.cfg.lock().expect("book cfg lock poisoned");
                    scope_admits(&cfg.scope, instrument_id)
                })
                .map(|(id, _)| id.clone())
                .collect();
            ids.sort();
            ids
        };
        for book_id in candidates {
            let Some(published) = self.snapshot(&book_id) else {
                continue;
            };
            let Some(line) = published
                .snapshot
                .instruments
                .iter()
                .find(|i| i.instrument_id == instrument_id)
            else {
                continue;
            };
            // Guard a degenerate composite (non-finite / crossed): skip to the next book
            // (ultimately the synthetic fallback) rather than emit a bad RFQ price.
            if !(line.best_bid.is_finite()
                && line.best_offer.is_finite()
                && line.best_bid <= line.best_offer)
            {
                continue;
            }
            let members = line
                .contributions
                .iter()
                .map(|c| RfqMemberLine {
                    lp_name: c.lp_name.clone(),
                    bid: c.bid,
                    offer: c.offer,
                    stale: c.stale,
                })
                .collect();
            return Some(RfqComposite {
                book_id,
                best_bid: line.best_bid,
                best_offer: line.best_offer,
                members,
                // The book-default composite is not priced through a group pipeline, so
                // it carries no per-feature provenance (design §7).
                provenance: None,
            });
        }
        None
    }

    /// The current **caller → pricing-group** resolver (an `Arc` clone — cheap, and a
    /// stable read-only snapshot until the next [`Self::reconcile`]). The ESP / RFQ
    /// hooks resolve their subscriber/caller against this before pricing.
    #[must_use]
    pub fn pricing_groups(&self) -> Arc<PricingGroupResolver> {
        Arc::clone(&self.pricing.read().expect("pricing groups lock poisoned"))
    }

    /// The per-subscriber **ESP** composite for `book_id`, priced through `pipeline`
    /// (a pricing group's ESP pipeline) applied to the book's raw consolidated
    /// composite — so a grouped subscriber receives its own outbound two-way off the
    /// same raw liquidity (`docs/FI-PRICING-GROUPS-DESIGN.md` §5). `None` when no enabled
    /// book with that id exists (mirroring [`Self::snapshot`]).
    ///
    /// The no-group ESP path does **not** call this — it keeps handing out
    /// [`PublishedBook::snapshot`] verbatim, so an ungrouped subscriber stays
    /// byte-identical.
    #[must_use]
    pub fn snapshot_priced(
        &self,
        book_id: &str,
        pipeline: &FeaturePipeline,
    ) -> Option<AggregatedBookSnapshot> {
        let published = self.snapshot(book_id)?;
        let cfg = {
            let books = self.books.read().expect("aggregation books lock poisoned");
            let engine = books.get(book_id)?;
            Arc::clone(&*engine.cfg.lock().expect("book cfg lock poisoned"))
        };
        Some(apply_pipeline(
            &published.snapshot,
            pipeline,
            &cfg,
            self.inventory.as_deref(),
            self.clock.now_nanos(),
        ))
    }

    /// Resolve the **RFQ** composite line for `instrument_id`, priced through `pipeline`
    /// (a pricing group's effective RFQ pipeline) applied to the book's raw consolidated
    /// two-way — so a grouped caller's quote is built from the raw liquidity through its
    /// own pipeline (`docs/FI-PRICING-GROUPS-DESIGN.md` §5). Book selection,
    /// freshness/quorum, the degenerate-composite guard, and the member-line panel are
    /// **identical** to [`Self::resolve_rfq_composite`]; only the priced best bid/offer
    /// differ.
    ///
    /// The no-group RFQ path does **not** call this — it keeps using
    /// [`Self::resolve_rfq_composite`] (the raw composite line), so an ungrouped caller
    /// stays byte-identical.
    #[must_use]
    pub fn resolve_rfq_composite_priced(
        &self,
        instrument_id: &str,
        pricing_group_id: &str,
        pipeline: &FeaturePipeline,
    ) -> Option<RfqComposite> {
        let now = self.clock.now_nanos();
        let settlement = settlement_date(now);
        let candidates: Vec<String> = {
            let books = self.books.read().expect("aggregation books lock poisoned");
            let mut ids: Vec<String> = books
                .iter()
                .filter(|(_, engine)| {
                    let cfg = engine.cfg.lock().expect("book cfg lock poisoned");
                    scope_admits(&cfg.scope, instrument_id)
                })
                .map(|(id, _)| id.clone())
                .collect();
            ids.sort();
            ids
        };
        for book_id in candidates {
            let Some(published) = self.snapshot(&book_id) else {
                continue;
            };
            let cfg = {
                let books = self.books.read().expect("aggregation books lock poisoned");
                let Some(engine) = books.get(&book_id) else {
                    continue;
                };
                Arc::clone(&*engine.cfg.lock().expect("book cfg lock poisoned"))
            };
            // Price off the raw composite line — the pipeline is the caller's own
            // outbound construction.
            let Some(line) = published
                .snapshot
                .instruments
                .iter()
                .find(|i| i.instrument_id == instrument_id)
            else {
                continue;
            };
            if !(line.best_bid.is_finite()
                && line.best_offer.is_finite()
                && line.best_bid <= line.best_offer)
            {
                continue;
            }
            let ctx = pricing_ctx_for_line(line, &cfg, self.inventory.as_deref(), settlement);
            let priced = pipeline.run(
                TwoWay {
                    bid: line.best_bid,
                    offer: line.best_offer,
                },
                &ctx,
            );
            // The pipeline's guardrails keep the outbound two-way finite and
            // non-crossed; skip a (degenerate) non-finite result rather than emit it.
            if !(priced.outbound.bid.is_finite() && priced.outbound.offer.is_finite()) {
                continue;
            }
            let members = line
                .contributions
                .iter()
                .map(|c| RfqMemberLine {
                    lp_name: c.lp_name.clone(),
                    bid: c.bid,
                    offer: c.offer,
                    stale: c.stale,
                })
                .collect();
            return Some(RfqComposite {
                book_id,
                best_bid: priced.outbound.bid,
                best_offer: priced.outbound.offer,
                members,
                // Stamp the per-feature provenance from the SAME already-run pipeline —
                // computed once here, never re-derived (design §7).
                provenance: Some(provenance_from_priced(
                    &priced,
                    pricing_group_id,
                    EspOrRfq::Rfq,
                )),
            });
        }
        None
    }
}

/// Map a [`celnet_tiering::FeatureKind`] to its wire [`celnet_proto::FeatureKind`]
/// (the two enums are the same ordered palette; an explicit match keeps them pinned
/// together rather than relying on discriminant coincidence).
fn feature_kind_to_wire(kind: FeatureKind) -> celnet_proto::FeatureKind {
    match kind {
        FeatureKind::MidShift => celnet_proto::FeatureKind::MidShift,
        FeatureKind::Tiering => celnet_proto::FeatureKind::Tiering,
        FeatureKind::Axe => celnet_proto::FeatureKind::Axe,
        FeatureKind::Position => celnet_proto::FeatureKind::Position,
        FeatureKind::PanicSkew => celnet_proto::FeatureKind::PanicSkew,
    }
}

/// Derive the wire [`PricingProvenance`] (design §7) from an already-run
/// [`PricedResult`] — the RAW → constructed → tiered → outbound waterfall plus the
/// attributed margin/skew and the ordered feature kinds. Computed once from the
/// pipeline output; never re-prices.
#[must_use]
pub(crate) fn provenance_from_priced(
    priced: &PricedResult,
    pricing_group_id: &str,
    mode: EspOrRfq,
) -> PricingProvenance {
    let raw = priced.raw;
    let constructed = priced.constructed();
    let tiered = priced.tiered();
    let outbound = priced.outbound;
    PricingProvenance {
        pricing_group_id: pricing_group_id.to_owned(),
        mode: mode as i32,
        raw_bid: raw.bid,
        raw_mid: 0.5 * (raw.bid + raw.offer),
        raw_offer: raw.offer,
        constructed_bid: constructed.bid,
        constructed_offer: constructed.offer,
        tiered_bid: tiered.bid,
        tiered_offer: tiered.offer,
        outbound_bid: outbound.bid,
        outbound_offer: outbound.offer,
        applied_margin: priced.applied_margin,
        applied_skew: priced.applied_skew,
        features: priced
            .feature_kinds()
            .into_iter()
            .map(|k| feature_kind_to_wire(k) as i32)
            .collect(),
    }
}

/// Build the per-line [`PricingCtx`] a pricing-group feature pipeline reads, mirroring
/// [`apply_tiering`]'s [`QuoteCtx`] construction: the composite `mid`, the book-wide net
/// inventory for the line's instrument, and — when the instrument has bond terms and a
/// settlement resolves — its DV01 (so a [`SpreadUnit::YieldBps`] feature converts a
/// yield move to a price offset). The raw observed spread is carried for provenance.
///
/// Unlike [`apply_tiering`], DV01 is attached whenever it resolves (not gated on a
/// single config unit) because a pipeline may mix units across features; and the
/// per-instrument fading-memory **smoothed** spread is intentionally not threaded here
/// (a per-client EWMA is a later phase), so a `ScaledSmoothedSpread` feature prices at
/// its documented indicative fallback under this path.
fn pricing_ctx_for_line(
    line: &AggregatedInstrument,
    cfg: &BookCfg,
    inventory: Option<&dyn InventorySource>,
    settlement: Option<time::Date>,
) -> PricingCtx {
    let mid = 0.5 * (line.best_bid + line.best_offer);
    let net = inventory.map_or(0.0, |src| src.net_inventory(&line.instrument_id));
    let mut ctx = QuoteCtx::new(mid).with_inventory(net);
    if let (Some(s), Some(terms)) = (settlement, cfg.bond_terms.get(&line.instrument_id))
        && let Some(dv01) = bond_dv01(terms, s, mid)
    {
        ctx = ctx.with_dv01(dv01);
    }
    let raw_spread = line.best_offer - line.best_bid;
    if raw_spread.is_finite() && raw_spread > 0.0 {
        ctx = ctx.with_raw_spread(raw_spread);
    }
    PricingCtx::new(ctx)
}

/// Re-price a book's **raw** composite through a pricing-group [`FeaturePipeline`], one
/// instrument line at a time. Mirrors [`apply_tiering`] but runs the ordered,
/// trader-composed features (RAW → … → outbound) instead of a single [`TieringConfig`],
/// returning each line's guarded outbound two-way (`docs/FI-PRICING-GROUPS-DESIGN.md`
/// §6). A line whose raw two-way is non-finite is dropped (as the tiering path drops a
/// non-finite mid).
fn apply_pipeline(
    raw: &AggregatedBookSnapshot,
    pipeline: &FeaturePipeline,
    cfg: &BookCfg,
    inventory: Option<&dyn InventorySource>,
    now: i64,
) -> AggregatedBookSnapshot {
    let settlement = settlement_date(now);
    let mut instruments = Vec::with_capacity(raw.instruments.len());
    for line in &raw.instruments {
        if !(line.best_bid.is_finite() && line.best_offer.is_finite()) {
            continue;
        }
        let ctx = pricing_ctx_for_line(line, cfg, inventory, settlement);
        let priced = pipeline.run(
            TwoWay {
                bid: line.best_bid,
                offer: line.best_offer,
            },
            &ctx,
        );
        let mut out = line.clone();
        out.best_bid = priced.outbound.bid;
        out.best_offer = priced.outbound.offer;
        instruments.push(out);
    }
    AggregatedBookSnapshot {
        book_id: raw.book_id.clone(),
        instruments,
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
    bond_terms: Arc<HashMap<String, BondPricingTerms>>,
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
        bond_terms,
    }
}

/// Build the bond-pricing-terms map (`instrument_id -> BondPricingTerms`) from the
/// registry: every [`InstrumentFamily::Bond`] whose coupon-frequency and day-count
/// labels resolve to an engine convention. A bond whose day count is `act_act` (no
/// engine basis yet) is skipped, so a yield-bps tiering config for it suppresses rather
/// than publishing a price off an unmodellable duration.
fn build_bond_terms(store: &IdentityStore) -> HashMap<String, BondPricingTerms> {
    let mut out = HashMap::new();
    for d in &store.instruments {
        let InstrumentFamily::Bond(b) = &d.definition else {
            continue;
        };
        let Some(frequency) = payment_frequency_from_label(&b.coupon_frequency) else {
            continue;
        };
        let Some(day_count) = accrual_basis_from_label(&b.day_count) else {
            continue;
        };
        let m = b.maturity_date;
        let Ok(month_num) = u8::try_from(m.month) else {
            continue;
        };
        let Ok(month) = time::Month::try_from(month_num) else {
            continue;
        };
        let Ok(day) = u8::try_from(m.day) else {
            continue;
        };
        let Ok(maturity) = time::Date::from_calendar_date(m.year, month, day) else {
            continue;
        };
        out.insert(
            d.instrument_id.clone(),
            BondPricingTerms {
                maturity,
                coupon_rate: b.coupon_rate,
                frequency,
                day_count,
                redemption: b.redemption,
            },
        );
    }
    out
}

/// The valuation/settlement date the streaming DV01 is computed at — the calendar date
/// of the edge clock (`now` = unix nanos). `None` if `now` is out of the representable
/// range (a yield-bps line then suppresses rather than pricing off a bad date).
fn settlement_date(now: i64) -> Option<time::Date> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(now))
        .ok()
        .map(|dt| dt.date())
}

/// Derive a bond's **DV01** (dirty-price move per +1bp of yield) at the live composite
/// `mid_clean`, off the `celnet-bond` leaf: build the contract, add accrued to the clean
/// mid to get the dirty price the market yield solves from, then return the leaf's DV01
/// at that yield. `None` (⇒ the yield-bps line suppresses) if the contract is malformed
/// for this settlement/maturity or the yield solve fails.
fn bond_dv01(terms: &BondPricingTerms, settlement: time::Date, mid_clean: f64) -> Option<f64> {
    let bond = Bond::new(
        settlement,
        terms.maturity,
        terms.coupon_rate,
        terms.frequency,
        terms.day_count,
        terms.redemption,
    )
    .ok()?;
    let accrued = accrued_interest(&bond).ok()?;
    let dirty = mid_clean + accrued;
    let risk = bond_risk(&bond, dirty).ok()?;
    risk.dv01.is_finite().then_some(risk.dv01)
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

    // --- pricing groups: per-client pipelines off the same raw composite ----

    use celnet_tiering::{
        FeaturePipeline, Guardrails, PricingFeature, SpreadUnit, StalePolicy, StrategySpec,
        TieringConfig,
    };

    /// Permissive guardrails that never clamp the small offsets these tests use, so a
    /// test asserts the raw strategy arithmetic (the guardrail clamps are unit-tested in
    /// `celnet-tiering` itself).
    fn open_guardrails() -> Guardrails {
        Guardrails::new(0.0, 1_000.0, 1_000.0, 1e-9)
    }

    /// A single [`TieringConfig`] the pricing-group tests embed in a `PricingFeature::Tiering`.
    fn tiering(unit: SpreadUnit, strategies: Vec<StrategySpec>) -> TieringConfig {
        TieringConfig {
            unit,
            strategies,
            guardrails: open_guardrails(),
            stale_policy: StalePolicy::Suppress,
        }
    }

    /// A single-TIERING (Flat ±`bps` price-bps) pipeline with open guardrails — the
    /// per-client outbound construction a pricing group applies to the RAW composite.
    fn flat_pipeline(bps: f64) -> FeaturePipeline {
        FeaturePipeline::new(
            vec![PricingFeature::Tiering {
                config: tiering(
                    SpreadUnit::PriceBps,
                    vec![StrategySpec::FlatMarkup { half_spread: bps }],
                ),
            }],
            open_guardrails(),
        )
    }

    /// The oracle: two clients in DIFFERENT pricing groups receive DIFFERENT two-ways off
    /// the SAME raw composite (mid 99.55). Group A (Flat ±25 price bps) → 99.30/99.80;
    /// Group B (Flat ±50) → 99.05/100.05; the ungrouped default path publishes the raw
    /// composite verbatim (99.50/99.60).
    #[test]
    fn pricing_groups_price_two_clients_differently_off_same_raw_composite() {
        let hub = hub_with(def("b", &["LP-1"], params(false, 1, 60_000)));
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW)); // raw mid 99.55
        let group_a = flat_pipeline(25.0);
        let group_b = flat_pipeline(50.0);
        let a = hub.snapshot_priced("b", &group_a).expect("book");
        let b = hub.snapshot_priced("b", &group_b).expect("book");
        let ai = &a.instruments[0];
        let bi = &b.instruments[0];
        assert!((ai.best_bid - 99.30).abs() < 1e-9, "A bid={}", ai.best_bid);
        assert!(
            (ai.best_offer - 99.80).abs() < 1e-9,
            "A offer={}",
            ai.best_offer
        );
        assert!((bi.best_bid - 99.05).abs() < 1e-9, "B bid={}", bi.best_bid);
        assert!(
            (bi.best_offer - 100.05).abs() < 1e-9,
            "B offer={}",
            bi.best_offer
        );
        // The SAME raw composite fed both — the outbound two-ways differ per group.
        assert_ne!(ai.best_bid.to_bits(), bi.best_bid.to_bits());
        // No-group default path is byte-identical: the raw composite published verbatim.
        let default = hub.snapshot("b").expect("book");
        let di = &default.snapshot.instruments[0];
        assert_eq!(di.best_bid.to_bits(), 99.50_f64.to_bits());
        assert_eq!(di.best_offer.to_bits(), 99.60_f64.to_bits());
    }

    /// The no-group ESP path streams the RAW consolidated composite unchanged (tiering is
    /// group-only — a book carries no outbound tier), while a grouped subscriber prices off
    /// that same RAW composite through its pipeline.
    #[test]
    fn no_group_esp_publishes_raw_while_group_prices_off_raw() {
        let d = def("b", &["LP-1"], params(false, 1, 60_000));
        let hub = hub_with(d);
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW));
        // No-group path: the RAW composite line 99.50/99.60 (no book tier).
        let snap = hub.snapshot("b").expect("book");
        let inst = &snap.snapshot.instruments[0];
        assert!(
            (inst.best_bid - 99.50).abs() < 1e-9,
            "bid={}",
            inst.best_bid
        );
        assert!(
            (inst.best_offer - 99.60).abs() < 1e-9,
            "offer={}",
            inst.best_offer
        );
        // Grouped subscriber prices off the RAW mid 99.55 (NOT the 99.45/99.65 book line):
        // Flat ±25 → 99.30/99.80.
        let priced = hub
            .snapshot_priced("b", &flat_pipeline(25.0))
            .expect("book");
        let pi = &priced.instruments[0];
        assert!((pi.best_bid - 99.30).abs() < 1e-9, "bid={}", pi.best_bid);
        assert!(
            (pi.best_offer - 99.80).abs() < 1e-9,
            "offer={}",
            pi.best_offer
        );
    }

    /// An RFQ caller in a pricing group prices via its RFQ pipeline off the RAW composite,
    /// while the no-group RFQ path returns the RAW composite (tiering is group-only). Member
    /// LP lines are the raw contributions in both (pricing groups do not alter the panel).
    #[test]
    fn rfq_group_prices_off_raw_via_rfq_pipeline() {
        let d = def("b", &["LP-1"], params(false, 1, 60_000));
        let hub = hub_with(d);
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW));
        // No-group RFQ: the RAW composite line 99.50/99.60 (no book tier).
        let base = hub.resolve_rfq_composite("X").expect("composite");
        assert!(
            (base.best_bid - 99.50).abs() < 1e-9,
            "bid={}",
            base.best_bid
        );
        assert!(
            (base.best_offer - 99.60).abs() < 1e-9,
            "offer={}",
            base.best_offer
        );
        // Grouped RFQ: Flat ±50 off the RAW mid 99.55 → 99.05/100.05.
        let priced = hub
            .resolve_rfq_composite_priced("X", "grp-test", &flat_pipeline(50.0))
            .expect("composite");
        assert!(
            (priced.best_bid - 99.05).abs() < 1e-9,
            "bid={}",
            priced.best_bid
        );
        assert!(
            (priced.best_offer - 100.05).abs() < 1e-9,
            "offer={}",
            priced.best_offer
        );
        assert_eq!(priced.book_id, "b");
        // Member LP lines are the raw contributions in both — unchanged by the group.
        assert_eq!(priced.members.len(), base.members.len());
        assert_eq!(
            priced.members[0].bid.to_bits(),
            base.members[0].bid.to_bits()
        );
        // The grouped composite carries per-feature provenance (design §7): the group
        // id + RFQ mode, the RAW mid 99.55, and the Flat-tiered outbound = the
        // reconstructed waterfall. The ungrouped base composite carries none.
        assert!(
            base.provenance.is_none(),
            "book-default carries no provenance"
        );
        let prov = priced.provenance.as_ref().expect("grouped ⇒ provenance");
        assert_eq!(prov.pricing_group_id, "grp-test");
        assert_eq!(prov.mode, celnet_proto::EspOrRfq::Rfq as i32);
        assert!(
            (prov.raw_mid - 99.55).abs() < 1e-9,
            "raw_mid={}",
            prov.raw_mid
        );
        // No MID SHIFT ⇒ constructed = RAW; TIERING ⇒ tiered = outbound = 99.05/100.05.
        assert!((prov.constructed_bid - prov.raw_bid).abs() < 1e-12);
        assert!(
            (prov.tiered_bid - 99.05).abs() < 1e-9,
            "tiered_bid={}",
            prov.tiered_bid
        );
        assert!((prov.tiered_offer - 100.05).abs() < 1e-9);
        assert!((prov.outbound_bid - priced.best_bid).abs() < 1e-12);
        assert!((prov.outbound_offer - priced.best_offer).abs() < 1e-12);
        assert_eq!(
            prov.features,
            vec![celnet_proto::FeatureKind::Tiering as i32]
        );
        // applied_margin = tiered half-spread − constructed(=raw) half-spread.
        let raw_half = 0.5 * (prov.raw_offer - prov.raw_bid);
        let tiered_half = 0.5 * (prov.tiered_offer - prov.tiered_bid);
        assert!((prov.applied_margin - (tiered_half - raw_half)).abs() < 1e-9);
    }
}
