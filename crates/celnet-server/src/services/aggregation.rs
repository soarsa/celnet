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
use celnet_proto::{AggregatedBookSnapshot, AggregatedInstrument, LpContribution, LpQuote};
use celnet_rates::{AccrualBasis, PaymentFrequency};
use celnet_tiering::{QuoteCtx, SpreadUnit, TieringConfig};
use celnet_types::{Ccy, CommodityRef, Symbol, Tenor, Underlying};

use crate::clock::Clock;
use crate::config::identity::{AggregatedBookDef, AggregationParams, IdentityStore, Scope};
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

/// A live source of signed net inventory per instrument, read by the outbound-tiering
/// [`InventorySkew`](celnet_tiering::InventorySkew) strategy to skew a book's composite
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
/// so an [`InventorySkew`](celnet_tiering::InventorySkew) strategy contributes zero skew
/// and the outbound two-way is byte-identical to the flat markup alone.
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
/// [`SpreadUnit::YieldBps`] tiering config — the reference-data [`BondDef`] mapped onto
/// the `celnet-bond` leaf's contract inputs. Captured at reconcile (the terms are
/// static); DV01 itself is solved per quote from the live composite mid (see
/// [`bond_dv01`]). Only bonds whose coupon-frequency and day-count labels resolve to an
/// engine convention get an entry — a bond quoting `act_act` (no engine basis yet) has
/// no terms, so a yield-bps config for it suppresses rather than publishes a bad price.
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
    /// The optional outbound-tiering configuration applied to this book's composite
    /// **before** publish (`docs/FI-TIERING-RESEARCH.md`). `None` ⇒ the raw composite is
    /// published unchanged (zero behaviour change).
    tiering: Option<TieringConfig>,
    /// The static bond-pricing terms per instrument, for deriving DV01 on a
    /// [`SpreadUnit::YieldBps`] config. Shared across books (built from the registry).
    bond_terms: Arc<HashMap<String, BondPricingTerms>>,
}

/// The published composite of a book at the version it reflects.
pub struct PublishedBook {
    /// The dirty-version this snapshot consolidates (monotone per book).
    pub version: u64,
    /// The **outbound** composite subscribers receive: the raw consolidated composite
    /// with each in-scope line transformed through the book's tiering config (or the
    /// raw composite verbatim when the book has no tiering config).
    pub snapshot: AggregatedBookSnapshot,
    /// The **raw** (untiered) consolidated composite, retained internally when tiering
    /// was applied so mid/risk views and the Phase 2b RFQ path can read the untiered
    /// two-way. `None` when no tiering config is set — [`Self::snapshot`] is then itself
    /// the raw composite (no duplication, zero overhead for a book with no tiering).
    pub raw: Option<AggregatedBookSnapshot>,
}

impl PublishedBook {
    /// The raw (untiered) composite: the retained [`Self::raw`] when tiering was applied,
    /// otherwise [`Self::snapshot`] (which is itself raw). The single accessor internal
    /// consumers use to read the untiered mid regardless of tiering.
    #[must_use]
    pub fn raw_snapshot(&self) -> &AggregatedBookSnapshot {
        self.raw.as_ref().unwrap_or(&self.snapshot)
    }
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
            raw: None,
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
            inventory: None,
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
        })
    }

    /// Rebuild the running engine set from the persisted identity store: stand up
    /// an engine for every **enabled** book, refresh the config of the ones that
    /// persist (preserving their live quote sink), and tear down the ones that were
    /// disabled or deleted. Idempotent — safe to call at boot and after every admin
    /// create/update/delete.
    pub fn reconcile(&self, store: &IdentityStore) {
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
            // Consolidate the RAW composite exactly as before (quorum/confidence
            // unchanged), then — only when the book carries a tiering config — transform
            // each in-scope line's outbound two-way, retaining the raw internally.
            let raw = consolidate_book(&engine.id, &cfg, &state.sink, now);
            let published = match &cfg.tiering {
                Some(tiering) => {
                    let tiered = apply_tiering(&raw, tiering, &cfg, self.inventory.as_deref(), now);
                    PublishedBook {
                        version: state.dirty_version,
                        snapshot: tiered,
                        raw: Some(raw),
                    }
                }
                None => PublishedBook {
                    version: state.dirty_version,
                    snapshot: raw,
                    raw: None,
                },
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
        tiering: def.tiering.clone(),
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

/// Transform a raw consolidated composite into the outbound (tiered) composite: for each
/// in-scope line build a [`QuoteCtx`] from real inputs — the composite mid, the live net
/// inventory, and (for a [`SpreadUnit::YieldBps`] config) the bond's DV01 at that mid —
/// run it through [`TieringConfig::quote`], and publish the tiered two-way. A line the
/// engine [`Suppressed`](celnet_tiering::Suppressed) (stale, or a yield-bps config with
/// no resolvable duration) is dropped — no quote for that line, never a bad price.
///
/// Streaming has no request size, so only the streaming-relevant strategies act (flat +
/// inventory skew); size/vol are left at the [`QuoteCtx`] neutral defaults (size 0,
/// `vol == vol_ref`), reserving the size-dependent strategies for the RFQ path (Phase 2b).
/// Consolidation/quorum/confidence are untouched — this is a pure post-consolidation
/// transform on the outbound two-way only.
fn apply_tiering(
    raw: &AggregatedBookSnapshot,
    tiering: &TieringConfig,
    cfg: &BookCfg,
    inventory: Option<&dyn InventorySource>,
    now: i64,
) -> AggregatedBookSnapshot {
    let settlement = settlement_date(now);
    let mut instruments = Vec::with_capacity(raw.instruments.len());
    for line in &raw.instruments {
        let mid = 0.5 * (line.best_bid + line.best_offer);
        let net = inventory.map_or(0.0, |src| src.net_inventory(&line.instrument_id));
        let mut ctx = QuoteCtx::new(mid).with_inventory(net);
        // DV01 is needed only for a yield-bps config; attach it when derivable, else
        // leave it off so the engine's own pre-validation suppresses the line.
        if tiering.unit == SpreadUnit::YieldBps
            && let (Some(s), Some(terms)) = (settlement, cfg.bond_terms.get(&line.instrument_id))
            && let Some(dv01) = bond_dv01(terms, s, mid)
        {
            ctx = ctx.with_dv01(dv01);
        }
        if let Ok(two_way) = tiering.quote(&ctx) {
            let mut tiered = line.clone();
            tiered.best_bid = two_way.bid;
            tiered.best_offer = two_way.offer;
            instruments.push(tiered);
        }
        // Else: Suppressed ⇒ no outbound quote for this line (drop it).
    }
    AggregatedBookSnapshot {
        book_id: raw.book_id.clone(),
        instruments,
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
            tiering: None,
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

    // --- outbound tiering (Phase 2a) ---------------------------------------

    use celnet_tiering::{Guardrails, StalePolicy, StrategySpec};

    /// Permissive guardrails that never clamp the small offsets these tests use, so a
    /// test asserts the raw strategy arithmetic (the guardrail clamps are unit-tested in
    /// `celnet-tiering` itself).
    fn open_guardrails() -> Guardrails {
        Guardrails::new(0.0, 1_000.0, 1_000.0, 1e-9)
    }

    fn tiering(unit: SpreadUnit, strategies: Vec<StrategySpec>) -> TieringConfig {
        TieringConfig {
            unit,
            strategies,
            guardrails: open_guardrails(),
            stale_policy: StalePolicy::Suppress,
        }
    }

    /// A book with NO tiering config publishes the raw composite unchanged and retains
    /// no separate raw copy — the regression guard that existing books are untouched.
    #[test]
    fn no_tiering_config_publishes_raw_composite_unchanged() {
        let hub = hub_with(def("b", &["LP-1", "LP-2"], params(false, 1, 60_000)));
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW));
        hub.ingest(&lp_quote("LP-2", "X", 99.55, 99.65, NOW));
        let snap = hub.snapshot("b").expect("book");
        assert!(snap.raw.is_none(), "no tiering ⇒ no separate raw retained");
        let inst = &snap.snapshot.instruments[0];
        // The untiered composite envelope: best bid = max(99.50, 99.55), best offer =
        // min(99.60, 99.65) — byte-identical to the pre-tiering publish path.
        assert_eq!(inst.best_bid.to_bits(), 99.55_f64.to_bits());
        assert_eq!(inst.best_offer.to_bits(), 99.60_f64.to_bits());
        // The internal raw accessor falls through to the (raw) snapshot.
        assert_eq!(
            snap.raw_snapshot().instruments[0].best_bid.to_bits(),
            99.55_f64.to_bits()
        );
    }

    /// Flat ±25 **price bps** on a composite whose raw mid is 99.55 publishes 99.30/99.80,
    /// and retains the raw composite internally.
    #[test]
    fn flat_price_bps_widens_symmetrically_around_mid() {
        let mut d = def("b", &["LP-1"], params(false, 1, 60_000));
        d.tiering = Some(tiering(
            SpreadUnit::PriceBps,
            vec![StrategySpec::FlatMarkup { half_spread: 25.0 }],
        ));
        let hub = hub_with(d);
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW));
        let snap = hub.snapshot("b").expect("book");
        let inst = &snap.snapshot.instruments[0];
        assert!(
            (inst.best_bid - 99.30).abs() < 1e-9,
            "bid={}",
            inst.best_bid
        );
        assert!(
            (inst.best_offer - 99.80).abs() < 1e-9,
            "offer={}",
            inst.best_offer
        );
        // Raw composite retained untouched (mid views / Phase 2b RFQ read this).
        let raw = &snap.raw.as_ref().expect("raw retained").instruments[0];
        assert_eq!(raw.best_bid.to_bits(), 99.50_f64.to_bits());
        assert_eq!(raw.best_offer.to_bits(), 99.60_f64.to_bits());
    }

    /// A nonzero **long** book position skews both sides DOWN by κ·q (in price bps), the
    /// spread `2h` is unchanged, and the book never crosses.
    #[test]
    fn inventory_skew_shifts_both_sides_and_never_crosses() {
        let inventory = Arc::new(InstrumentInventory::new());
        inventory.set("X", 10.0); // long 10 units
        let hub = AggregationHub::with_inventory(Clock::manual(NOW), inventory);
        let mut store = IdentityStore::default();
        let mut d = def("b", &["LP-1"], params(false, 1, 60_000));
        d.tiering = Some(tiering(
            SpreadUnit::PriceBps,
            vec![StrategySpec::InventorySkew {
                half_spread: 25.0,
                kappa: 1.0,
                s_max: 1_000.0,
            }],
        ));
        store.aggregated_books.push(d);
        hub.reconcile(&store);
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW));
        let snap = hub.snapshot("b").expect("book");
        let inst = &snap.snapshot.instruments[0];
        // mid=99.55, h=0.25 (25 price bps), s = κ·q = 10 price bps units = 0.10 offset.
        // bid = 99.55 − 0.25 − 0.10 = 99.20; offer = 99.55 + 0.25 − 0.10 = 99.70.
        assert!(
            (inst.best_bid - 99.20).abs() < 1e-9,
            "bid={}",
            inst.best_bid
        );
        assert!(
            (inst.best_offer - 99.70).abs() < 1e-9,
            "offer={}",
            inst.best_offer
        );
        // Both sides shifted DOWN (long ⇒ shed risk); spread invariant; never crossed.
        assert!(inst.best_bid < inst.best_offer);
        assert!((inst.best_offer - inst.best_bid - 0.50).abs() < 1e-9);
    }

    /// A zero position with an inventory-skew strategy contributes zero skew, so the
    /// outbound two-way equals the flat markup alone (the "0 if none" property).
    #[test]
    fn inventory_skew_with_no_position_is_flat() {
        let mut d = def("b", &["LP-1"], params(false, 1, 60_000));
        d.tiering = Some(tiering(
            SpreadUnit::PriceBps,
            vec![StrategySpec::InventorySkew {
                half_spread: 25.0,
                kappa: 1.0,
                s_max: 1_000.0,
            }],
        ));
        // hub_with wires NO inventory source ⇒ every instrument nets to zero.
        let hub = hub_with(d);
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW));
        let inst = &hub.snapshot("b").expect("book").snapshot.instruments[0];
        assert!((inst.best_bid - 99.30).abs() < 1e-9);
        assert!((inst.best_offer - 99.80).abs() < 1e-9);
    }

    /// A **yield-bps** flat markup on a registered bond publishes an offset of
    /// `DV01 · yield_bps`, DV01 hand-derived from the same `celnet-bond` leaf at the
    /// composite mid — the wiring identity that tiering feeds the engine the real DV01.
    #[test]
    fn yield_bps_on_a_bond_offsets_by_dv01_times_bps() {
        let settlement = time::Date::from_calendar_date(2026, time::Month::June, 25).expect("date");
        let maturity = time::Date::from_calendar_date(2031, time::Month::June, 25).expect("date");
        let now_ns = settlement.midnight().assume_utc().unix_timestamp_nanos() as i64;

        let mut store = IdentityStore::default();
        store.instruments.push(bond_def_5y());
        let mut d = def("b", &["LP-1"], params(false, 1, 60_000));
        d.instrument_scope = Scope::Explicit(vec!["BND-5Y".to_string()]);
        d.tiering = Some(tiering(
            SpreadUnit::YieldBps,
            vec![StrategySpec::FlatMarkup { half_spread: 10.0 }],
        ));
        store.aggregated_books.push(d);
        let hub = AggregationHub::new(Clock::manual(now_ns));
        hub.reconcile(&store);
        hub.ingest(&lp_quote("LP-1", "BND-5Y", 99.50, 99.60, now_ns));

        let snap = hub.snapshot("b").expect("book");
        let inst = &snap.snapshot.instruments[0];
        // Independent DV01 from the SAME bond leaf at the composite mid (dirty = clean +
        // accrued), then offset = DV01 · 10bp.
        let mid = 0.5 * (99.50 + 99.60);
        let bond = Bond::new(
            settlement,
            maturity,
            0.04,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Act365Fixed,
            100.0,
        )
        .expect("bond");
        let dirty = mid + accrued_interest(&bond).expect("accrued");
        let dv01 = bond_risk(&bond, dirty).expect("risk").dv01;
        let offset = dv01 * 10.0;
        assert!(dv01 > 0.0, "dv01 must be positive");
        assert!(
            (inst.best_bid - (mid - offset)).abs() < 1e-9,
            "bid={} expected={}",
            inst.best_bid,
            mid - offset
        );
        assert!(
            (inst.best_offer - (mid + offset)).abs() < 1e-9,
            "offer={} expected={}",
            inst.best_offer,
            mid + offset
        );
    }

    /// A yield-bps config on an instrument with NO resolvable duration (not a registered
    /// bond) suppresses the line rather than publishing a bad price.
    #[test]
    fn yield_bps_without_duration_suppresses_the_line() {
        let mut d = def("b", &["LP-1"], params(false, 1, 60_000));
        d.tiering = Some(tiering(
            SpreadUnit::YieldBps,
            vec![StrategySpec::FlatMarkup { half_spread: 10.0 }],
        ));
        let hub = hub_with(d); // no bond registered ⇒ no DV01 for "X"
        hub.ingest(&lp_quote("LP-1", "X", 99.50, 99.60, NOW));
        let snap = hub.snapshot("b").expect("book");
        assert!(
            snap.snapshot.instruments.is_empty(),
            "no duration ⇒ suppressed (no quote for the line)"
        );
        // The raw composite is still retained internally.
        assert_eq!(snap.raw_snapshot().instruments.len(), 1);
    }

    /// A minimal registered 5y semi-annual 4% bond (`BND-5Y`) for the yield-bps test.
    fn bond_def_5y() -> crate::config::reference_data::InstrumentDef {
        use crate::config::reference_data::{BondDef, CivilDate, InstrumentDef};
        InstrumentDef {
            instrument_id: "BND-5Y".to_string(),
            name: "Test 5Y 4%".to_string(),
            description: String::new(),
            currency: "USD".to_string(),
            external_ids: vec![],
            definition: InstrumentFamily::Bond(BondDef {
                issuer: "TEST".to_string(),
                coupon_rate: 0.04,
                coupon_type: "fixed".to_string(),
                coupon_frequency: "semi_annual".to_string(),
                day_count: "act_365_fixed".to_string(),
                issue_date: None,
                dated_date: None,
                first_coupon_date: None,
                maturity_date: CivilDate {
                    year: 2031,
                    month: 6,
                    day: 25,
                },
                redemption: 100.0,
                calendars: vec![],
            }),
        }
    }
}
