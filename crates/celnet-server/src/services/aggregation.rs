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
use celnet_refdata::{CrosswalkBasis, IdentifierCrosswalk, IdentifierSet};
use celnet_tiering::{FeatureKind, FeaturePipeline, PricedResult, PricingCtx, QuoteCtx, TwoWay};
use celnet_types::{Ccy, CommodityRef, Symbol, Tenor, Underlying};

mod lp_health;

pub use lp_health::{LpFeedHealth, LpFeedQuote, LpPanel};

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
    /// The **identifier cross-walk** over [`Self::snapshot`]'s lines, so a lookup whose
    /// id lives in a different identifier namespace than the one this panel is keyed by
    /// still resolves (see [`IdentifierCrosswalk`]).
    ///
    /// Built ONCE per published version alongside the consolidation, never per lookup:
    /// this index is read on the auto-hedge fill path, where a repeated linear scan of
    /// the quoted universe (hundreds of instruments) per fill would be a real cost. The
    /// [`Resolution::row`] it returns indexes straight back into `snapshot.instruments`.
    panel: IdentifierCrosswalk,
}

impl PublishedBook {
    /// The composite line for `query` — resolved through the panel cross-walk, so a
    /// caller holding the security under a different identifier (a registry slug against
    /// a CUSIP-keyed panel, or vice versa) still finds the line the venues are quoting.
    ///
    /// `None` when no identifier `query` carries resolves exactly to a published line.
    /// The caller then has an honest miss and must fall back rather than approximate.
    fn line(&self, query: &IdentifierSet) -> Option<(&AggregatedInstrument, CrosswalkBasis)> {
        let hit = self.panel.resolve(query)?;
        // The index is built from `snapshot.instruments` in order, so the row is in
        // range by construction; `get` keeps that an honest miss rather than a panic if
        // the two ever drift.
        let line = self.snapshot.instruments.get(hit.row)?;
        Some((line, hit.basis))
    }
}

/// Index a published snapshot's lines for the identifier cross-walk. Every composite
/// line already carries its ISIN and CUSIP (resolved from reference data at
/// consolidation), so the join keys exist on the panel side without any new plumbing.
fn panel_crosswalk(snapshot: &AggregatedBookSnapshot) -> IdentifierCrosswalk {
    IdentifierCrosswalk::build(snapshot.instruments.iter().map(|i| IdentifierSet {
        instrument_id: i.instrument_id.clone(),
        cusip: i.cusip.clone(),
        isin: i.isin.clone(),
    }))
}

/// One instrument the firm advertises as tradeable that no liquidity provider can price.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnquotableInstrument {
    /// The registry `instrument_id` as advertised.
    pub instrument_id: String,
    /// Its display name, so the warning names a security a human recognises.
    pub name: String,
    /// The instrument family (`bond` / `bond_future`).
    pub kind: &'static str,
    /// Its registered CUSIP, or empty — shown because a MISSING cross-reference is the
    /// usual reason an otherwise-real security fails to cross-walk onto a panel.
    pub cusip: String,
    /// Its registered ISIN, or empty.
    pub isin: String,
}

/// The result of [`AggregationHub::audit_unquotable`] — how many risk-transferable
/// instruments the registry advertises, and which of them no venue can quote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnquotableAudit {
    /// How many tradeable (bond / bond-future) instruments were examined.
    pub tradeable: usize,
    /// How many of the quotable ones were only reachable through a **cross-reference**
    /// (CUSIP or ISIN) rather than an exact `instrument_id` match — i.e. how many
    /// securities the registry and the wire disagree about the name of. These fill
    /// correctly, but a growing count means the two namespaces are drifting.
    pub cross_walked: usize,
    /// The tradeable instruments no liquidity provider can price, by `instrument_id`.
    pub unquotable: Vec<UnquotableInstrument>,
}

impl UnquotableAudit {
    /// Whether the tradeable and quotable universes coincide — the healthy state.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.unquotable.is_empty()
    }

    /// A compact one-line rendering of the offending instruments for a log field.
    #[must_use]
    pub fn summary(&self) -> String {
        self.unquotable
            .iter()
            .map(|u| {
                let cross_refs = match (u.cusip.is_empty(), u.isin.is_empty()) {
                    (true, true) => "no cross-refs".to_owned(),
                    (true, false) => format!("isin={}", u.isin),
                    (false, true) => format!("cusip={}", u.cusip),
                    (false, false) => format!("cusip={} isin={}", u.cusip, u.isin),
                };
                format!("{} [{}] ({})", u.instrument_id, u.kind, cross_refs)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Whether the **curated universe** `celnet-refdata` ships — the universe the liquidity
/// providers price from — knows this security, and on which identifier.
///
/// Applies the SAME precedence [`IdentifierCrosswalk::resolve`] does (exact
/// `instrument_id` → CUSIP → ISIN) so the audit's verdict and the live lookup's verdict
/// cannot disagree. Each arm is an O(1) probe of the crate's static identity index, so
/// this stays cheap over the whole registry. Returns `None` — never an approximate hit —
/// when no identifier the query carries is in the curated universe.
fn curated_basis(query: &IdentifierSet) -> Option<CrosswalkBasis> {
    let known =
        |id: &str| !id.trim().is_empty() && celnet_refdata::curated_identifiers(id).is_some();
    if known(&query.instrument_id) {
        return Some(CrosswalkBasis::Exact);
    }
    if known(&query.cusip) {
        return Some(CrosswalkBasis::Cusip);
    }
    if known(&query.isin) {
        return Some(CrosswalkBasis::Isin);
    }
    None
}

/// The identifiers carried by a reference-data definition, for the audit's cross-walk
/// query. Reads the same `external_ids` [`build_identities`] does, so the audit and the
/// live lookup agree by construction on what a security's cross-references are.
fn query_identifiers_from_def(def: &crate::config::reference_data::InstrumentDef) -> IdentifierSet {
    let ext = |scheme: &str| {
        def.external_ids
            .iter()
            .find(|e| e.scheme.eq_ignore_ascii_case(scheme))
            .map(|e| e.value.clone())
            .unwrap_or_default()
    };
    IdentifierSet {
        instrument_id: def.instrument_id.clone(),
        cusip: ext("cusip"),
        isin: ext("isin"),
    }
}

/// The identifiers known for `instrument_id` on the QUERY side of the cross-walk.
///
/// Resolution order for the aliases themselves mirrors how much the platform trusts the
/// source: the **reference-data registry** first (an admin-curated entry is the firm's
/// own statement of a security's identity), then the **curated universe** shipped in
/// `celnet-refdata` (which knows every government bond an LP can quote). An id in
/// neither yields an exact-only set — it can still match a panel line by its own id, and
/// otherwise stays honestly unresolved.
fn query_identifiers(
    identities: &HashMap<String, InstrumentIdentity>,
    instrument_id: &str,
) -> IdentifierSet {
    if let Some(identity) = identities.get(instrument_id) {
        let ids = IdentifierSet {
            instrument_id: instrument_id.to_owned(),
            cusip: identity.cusip.clone(),
            isin: identity.isin.clone(),
        };
        if ids.has_cross_refs() {
            return ids;
        }
        // A registered instrument with no cross-refs (a rates slug) still gets the
        // curated fallback below rather than nothing.
    }
    celnet_refdata::curated_identifiers(instrument_id).map_or_else(
        || IdentifierSet::from_id(instrument_id),
        |mut ids| {
            // Keep the CALLER's id as the exact key — the curated record's canonical id
            // is reached through the cross-refs, and overwriting it here would silently
            // re-point an exact match.
            ids.instrument_id = instrument_id.to_owned();
            ids
        },
    )
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
            panel: IdentifierCrosswalk::default(),
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
    /// The reference-data identity map (`instrument_id -> {name, isin, cusip}`) rebuilt
    /// on every [`Self::reconcile`] — the same `Arc` each book's `cfg` carries, hoisted
    /// onto the hub so a lookup can resolve a query's cross-references BEFORE it knows
    /// which book covers the instrument (the identifier cross-walk needs the aliases to
    /// pick the covering book at all). Behind an `RwLock<Arc<…>>` for the same
    /// hot-swap-whole discipline as `pricing`.
    identities: RwLock<Arc<HashMap<String, InstrumentIdentity>>>,
    /// **Street-side LP tick tally** — a bounded per-LP monotonic count of accepted
    /// quote-update pushes, keyed by `lp_name`. This is the tick-rate source for the
    /// LP liquidity analytics fold (`docs/ANALYTICS-REQUIREMENTS.md` §2.4): the sink
    /// only ever keeps the *latest* quote per venue (it overwrites), so the number of
    /// updates cannot be recovered from the sink — this counter records it at the
    /// ingest seam. Off the pinned hot core (ingest already runs on the async feed
    /// edge, guardrail 11); bounded by the member-LP count, never by time. Read
    /// on-query by [`Self::lp_tick_counts`].
    lp_ticks: Mutex<HashMap<String, u64>>,
    /// The firm-wide **inbound** pricing kill-switch. Read (a cheap `Relaxed` load) at
    /// the very top of every [`Self::ingest`] before any book lock is taken: when
    /// inbound ingest is disabled the pushed quote is dropped (no composite mutation),
    /// resuming from live on re-enable. A default-constructed hub (via [`Self::new`] /
    /// [`Self::with_inventory`]) uses a both-enabled control, so every existing ingest
    /// test and the standalone path behave exactly as before; the boot path shares the
    /// one runtime control via [`Self::with_control`].
    pricing_control: Arc<crate::services::pricing_control::PricingControl>,
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

/// The aggregation hub as a live **external-hedge LP source** (§6.2): the standing LP panel
/// an auto-hedge shed fills against is the SAME inbound per-LP quotes the agg book
/// consolidates. For a shed of `net_risk` on `instrument`, this returns the BEST executable
/// member-LP price on the required side (reducing a long ⇒ we SELL to an LP at its bid;
/// reducing a short ⇒ we BUY at its offer), among the fresh (non-stale) member lines — or
/// `None` when no covering book has a fresh member with a firm price on that side (an honest
/// miss the executor backstops to the composite / warehouses, never a fabricated fill).
///
/// # Why this reads the member panel and not the composite
///
/// A hedge fills against **one** named LP at **one** of that LP's own prices. It never needs
/// the book's consolidated two-way, so it must not inherit the non-crossed guard
/// [`AggregationHub::resolve_rfq_composite`] applies: that guard exists because quoting a
/// *crossed composite* out to a client as a two-way is nonsense, which is a statement about
/// the composite, not about the panel underneath it. A crossed composite means one member's
/// bid sits above another member's offer — every one of those prices is still a real, firm,
/// executable one-sided price from a named LP, and the desk shedding risk would happily lift
/// the best of them. Refusing to fill there is refusing free liquidity, and it is what made
/// every auto-hedge silently backstop to the synthetic COMPOSITE venue whenever the panel
/// happened to consolidate crossed (a divergent member, a fat-finger print, a two-member book
/// below the divergence-gating quorum) — so street-side LP analytics never recorded a fill.
impl crate::services::auto_hedge::LpHedgeSource for AggregationHub {
    fn best_fill(
        &self,
        instrument: &str,
        net_risk: f64,
        _size: f64,
    ) -> Option<crate::services::auto_hedge::LpFill> {
        // Reducing a long (net_risk > 0) sheds by SELLING → lift an LP's BID (best = highest);
        // reducing a short sheds by BUYING → lift an LP's OFFER (best = lowest). A zero/None
        // net has no side; treat it as a sell (a degenerate shed) for determinism.
        let sell = net_risk > 0.0;
        // Resolve the security's identifiers ONCE (one hash probe), then reuse them for
        // book admission and for every panel lookup: the id the hedge holds is not
        // necessarily the id the LPs key their panel by, and re-deriving the cross-refs
        // per book would put work on the fill path for no gain.
        let query = self.query_ids(instrument);
        // Covering books in deterministic id order: the first one holding an executable
        // member price wins, so a book whose members have all aged out falls through to the
        // next rather than masking live liquidity behind it.
        for book_id in self.covering_books_for(&query) {
            let Some((members, basis)) = self.resolve_member_panel_for(&book_id, &query) else {
                continue;
            };
            if basis != CrosswalkBasis::Exact {
                // The traded id and the quoted id are in different namespaces. The fill is
                // real and correct, but the asymmetry is worth seeing: it is what the
                // tradeable-vs-quotable audit exists to keep rare.
                tracing::debug!(
                    instrument = %instrument,
                    book = %book_id,
                    basis = basis.label(),
                    "hedge panel resolved through the identifier cross-walk"
                );
            }
            let mut best: Option<(&str, f64)> = None;
            for m in &members {
                if m.stale {
                    continue; // a stale contribution is not executable.
                }
                let price = if sell { m.bid } else { m.offer };
                if !price.is_finite() {
                    continue;
                }
                let improves = match best {
                    None => true,
                    Some((_, incumbent)) => {
                        if sell {
                            price > incumbent // we receive more selling into a higher bid.
                        } else {
                            price < incumbent // we pay less buying at a lower offer.
                        }
                    }
                };
                if improves {
                    best = Some((m.lp_name.as_str(), price));
                }
            }
            if let Some((lp_id, price)) = best {
                return Some(crate::services::auto_hedge::LpFill {
                    lp_id: lp_id.to_owned(),
                    price,
                });
            }
        }
        None
    }
}

impl AggregationHub {
    /// The shared constructor: an empty hub bound to `clock`, with the optional
    /// live `inventory` source and the firm-wide inbound `pricing_control` gate.
    fn build(
        clock: Clock,
        inventory: Option<Arc<dyn InventorySource>>,
        pricing_control: Arc<crate::services::pricing_control::PricingControl>,
    ) -> Arc<Self> {
        Arc::new(Self {
            books: RwLock::new(HashMap::new()),
            clock,
            inventory,
            pricing: RwLock::new(Arc::new(PricingGroupResolver::default())),
            identities: RwLock::new(Arc::new(HashMap::new())),
            lp_ticks: Mutex::new(HashMap::new()),
            pricing_control,
        })
    }

    /// Construct an empty hub bound to the edge `clock` (the valuation clock the
    /// staleness decay measures quote age against). The inbound kill-switch defaults to
    /// **enabled** (byte-identical to before the kill-switch); the boot path shares the
    /// runtime control via [`Self::with_control`].
    #[must_use]
    pub fn new(clock: Clock) -> Arc<Self> {
        Self::build(
            clock,
            None,
            crate::services::pricing_control::PricingControl::new(true, true),
        )
    }

    /// Construct a hub bound to `clock` **and** a live [`InventorySource`] the
    /// outbound-tiering skew reads. Used by the edge boot to wire the FI inventory book
    /// so an [`InventorySkew`](celnet_tiering::InventorySkew) strategy skews on the real
    /// net position; a hub built via [`Self::new`] reads zero inventory (no skew).
    #[must_use]
    pub fn with_inventory(clock: Clock, inventory: Arc<dyn InventorySource>) -> Arc<Self> {
        Self::build(
            clock,
            Some(inventory),
            crate::services::pricing_control::PricingControl::new(true, true),
        )
    }

    /// Construct an empty hub bound to `clock` sharing the firm-wide runtime inbound
    /// **pricing kill-switch** — the boot path so `SetPricingControl(inbound_enabled=false)`
    /// halts LP ingestion into this hub's books.
    #[must_use]
    pub fn with_control(
        clock: Clock,
        pricing_control: Arc<crate::services::pricing_control::PricingControl>,
    ) -> Arc<Self> {
        Self::build(clock, None, pricing_control)
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
        *self.identities.write().expect("identities lock poisoned") = Arc::clone(&identities);
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
        // Release the write guard before the audit: it reads the books through the same
        // lock (and re-locks via `snapshot`), so holding it here would deadlock.
        drop(books);
        // Surface any instrument the firm advertises as tradeable that no venue can quote
        // — the asymmetry that makes a hedge silently backstop instead of filling on a
        // named LP. Boot and every admin CRUD, so an instrument added without liquidity is
        // reported the moment it appears.
        self.report_unquotable(store);
    }

    /// Route one pushed LP quote into every enabled book that lists the pushing LP
    /// as a member and whose scope admits the instrument. Returns `true` if the
    /// quote was accepted into at least one book (the ingest ack counts these).
    pub fn ingest(&self, q: &LpQuote) -> bool {
        // Firm-wide inbound kill-switch (cheap `Relaxed` load, before any book lock): a
        // halted feed drops the quote — no composite mutation, and `LpFeedAck.accepted`
        // does not count it. Resumes from live on re-enable (no stale replay).
        if !self.pricing_control.inbound_enabled() {
            return false;
        }
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
            // Index the panel's identifiers once per published version — the lookup path
            // must stay O(1), not re-scan the quoted universe per fill.
            let panel = panel_crosswalk(&raw);
            let published = PublishedBook {
                version: state.dirty_version,
                snapshot: raw,
                panel,
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

    /// The ids of the enabled books whose scope admits `instrument_id`, in deterministic
    /// id order (a stable resolution when more than one book admits the same instrument).
    ///
    /// Collected under a short read lock that is released before returning, because every
    /// caller goes on to call [`Self::snapshot`], which re-locks — holding the read guard
    /// across that could deadlock.
    fn covering_books(&self, instrument_id: &str) -> Vec<String> {
        self.covering_books_for(&IdentifierSet::from_id(instrument_id))
    }

    /// The ids of the enabled books whose scope admits **any identifier `query` carries**
    /// (its own id, its CUSIP, or its ISIN), in deterministic id order.
    ///
    /// Scope admission is the *first* place the identifier-namespace mismatch bites: an
    /// `Explicit` book scoped by the CUSIPs its LPs stream does not list a registry slug
    /// for the same security, so an exact-only check rules the book out before the panel
    /// is ever consulted. Widening admission to the query's cross-references — and only
    /// to those exact strings — makes the covering-book choice agree with the cross-walk
    /// that follows it. An `AllMembersQuote` book was already admitting everything, so
    /// this changes nothing there.
    ///
    /// Passing an [`IdentifierSet::from_id`] (no cross-references) is byte-identical to
    /// the exact-only [`Self::covering_books`].
    fn covering_books_for(&self, query: &IdentifierSet) -> Vec<String> {
        let books = self.books.read().expect("aggregation books lock poisoned");
        let mut ids: Vec<String> = books
            .iter()
            .filter(|(_, engine)| {
                let cfg = engine.cfg.lock().expect("book cfg lock poisoned");
                scope_admits_any(&cfg.scope, query)
            })
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// The identifiers to query the cross-walk with for `instrument_id` — the reference
    /// -data cross-references the registry (or the curated universe) holds for it. Cheap:
    /// one `Arc` clone of the hot-swapped identity map plus at most one hash probe.
    fn query_ids(&self, instrument_id: &str) -> IdentifierSet {
        let identities = Arc::clone(&self.identities.read().expect("identities lock poisoned"));
        query_identifiers(&identities, instrument_id)
    }

    /// The contributing member-LP panel `book_id` currently publishes for
    /// `instrument_id` — **without** the non-crossed/finite composite guard
    /// [`Self::resolve_rfq_composite`] applies.
    ///
    /// That guard belongs to the question "what two-way do we show a client?", not to
    /// "which named LP can we hit?". A caller that fills against a single member's own
    /// firm price (see the [`LpHedgeSource`](crate::services::auto_hedge::LpHedgeSource)
    /// impl above) needs the panel and nothing else, and a crossed or otherwise
    /// degenerate composite says nothing about whether the members underneath it are
    /// executable. Callers that price a two-way keep using `resolve_rfq_composite`.
    ///
    /// The lookup is **identifier-namespace aware**: the panel is keyed by whatever
    /// `instrument_id` the LPs put on the wire (a CUSIP for a US Treasury, a contract
    /// code, a slug), which is not necessarily the id the caller holds. An exact match is
    /// tried first and, failing that, the security's CUSIP and then its ISIN — see
    /// [`IdentifierCrosswalk`] for the full precedence. An id that resolves on none of
    /// them stays unresolved.
    ///
    /// `None` when the book is not running or publishes no line resolvable from
    /// `instrument_id`.
    #[must_use]
    pub fn resolve_member_panel(
        &self,
        book_id: &str,
        instrument_id: &str,
    ) -> Option<Vec<RfqMemberLine>> {
        self.resolve_member_panel_for(book_id, &self.query_ids(instrument_id))
            .map(|(members, _)| members)
    }

    /// [`Self::resolve_member_panel`] against pre-resolved identifiers, also reporting
    /// **which** identifier the panel line was found on. `best_fill` resolves the query's
    /// cross-references once and reuses them across every covering book rather than
    /// re-deriving them per book.
    fn resolve_member_panel_for(
        &self,
        book_id: &str,
        query: &IdentifierSet,
    ) -> Option<(Vec<RfqMemberLine>, CrosswalkBasis)> {
        let published = self.snapshot(book_id)?;
        let (line, basis) = published.line(query)?;
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
        Some((members, basis))
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
    ///
    /// A caller that only needs to hit ONE named LP must NOT use this: the non-crossed
    /// guard is about the two-way this returns, not about whether the members underneath
    /// are executable. Use [`Self::resolve_member_panel`].
    #[must_use]
    pub fn resolve_rfq_composite(&self, instrument_id: &str) -> Option<RfqComposite> {
        for book_id in self.covering_books(instrument_id) {
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

    /// Audit the **tradeable-vs-quotable asymmetry**: every instrument the firm advertises
    /// as risk-transferable that no liquidity provider can price.
    ///
    /// # Why this guard exists
    ///
    /// An instrument that is tradeable but unquotable is invisible until it is too late.
    /// Everything looks healthy — the panel is full, the LPs are fresh, the book publishes
    /// — right up until a hedge on that ONE instrument finds no LP, silently backstops to
    /// the synthetic composite venue, and the desk's street-side analytics record a fill
    /// that never happened. That is exactly the shape of this defect, and of a prior
    /// production one. The asymmetry is cheap to check and expensive to discover, so it is
    /// checked on every [`Self::reconcile`] — boot and every admin CRUD, which is when the
    /// tradeable set can change.
    ///
    /// # What counts as quotable (and what this deliberately does NOT claim)
    ///
    /// An instrument is quotable when **either**:
    ///
    /// * it resolves — by id, CUSIP, or ISIN — into the curated universe `celnet-refdata`
    ///   ships, which is the universe the liquidity providers price from; **or**
    /// * some enabled book is *currently publishing* a line the cross-walk resolves it to
    ///   (direct live evidence an LP quotes it, even if it is outside the curated set).
    ///
    /// A curated instrument that simply has no live quote right now is **not** reported:
    /// that is an idle feed, not a structural gap, and crying wolf about it would train
    /// the desk to ignore the warning. This audit only reports instruments that no venue
    /// could quote *at all*.
    ///
    /// Scope is the risk-transferable families — cash bonds and bond futures. A rates
    /// curve pillar (a deposit, FRA, or swap) is a curve input, not something shed into an
    /// LP panel, so its absence from the panel is correct rather than a defect.
    #[must_use]
    pub fn audit_unquotable(&self, store: &IdentityStore) -> UnquotableAudit {
        let mut tradeable = 0usize;
        let mut cross_walked = 0usize;
        let mut unquotable = Vec::new();
        // Resolve against the live panels only when the curated check fails, and reuse one
        // list of book ids for the whole sweep.
        let book_ids: Vec<String> = {
            let books = self.books.read().expect("aggregation books lock poisoned");
            let mut ids: Vec<String> = books.keys().cloned().collect();
            ids.sort();
            ids
        };
        let snapshots: Vec<Arc<PublishedBook>> =
            book_ids.iter().filter_map(|id| self.snapshot(id)).collect();
        for def in &store.instruments {
            if !matches!(
                def.definition,
                InstrumentFamily::Bond(_) | InstrumentFamily::BondFuture(_)
            ) {
                continue;
            }
            tradeable += 1;
            let query = query_identifiers_from_def(def);
            if let Some(basis) = curated_basis(&query) {
                // An LP prices this from the curated universe. Count the ones that only
                // got there through a cross-reference: those are the namespace
                // disagreements this cross-walk exists to bridge, and a rising count is
                // the early warning that the registry and the wire are drifting apart.
                if basis != CrosswalkBasis::Exact {
                    cross_walked += 1;
                }
                continue;
            }
            if let Some(basis) = snapshots
                .iter()
                .find_map(|p| p.line(&query).map(|(_, b)| b))
            {
                // Live evidence: a venue is quoting it right now, even though it is
                // outside the curated set.
                if basis != CrosswalkBasis::Exact {
                    cross_walked += 1;
                }
                continue;
            }
            unquotable.push(UnquotableInstrument {
                instrument_id: def.instrument_id.clone(),
                name: def.name.clone(),
                kind: def.definition.kind(),
                cusip: query.cusip,
                isin: query.isin,
            });
        }
        unquotable.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
        UnquotableAudit {
            tradeable,
            cross_walked,
            unquotable,
        }
    }

    /// Run [`Self::audit_unquotable`] and surface the result: a `warn` naming every
    /// tradeable-but-unquotable instrument, and nothing louder than `debug` when the
    /// tradeable and quotable universes coincide. Quiet when clean, by design.
    fn report_unquotable(&self, store: &IdentityStore) {
        let audit = self.audit_unquotable(store);
        if audit.unquotable.is_empty() {
            tracing::debug!(
                tradeable = audit.tradeable,
                cross_walked = audit.cross_walked,
                "tradeable/quotable audit clean: every tradeable instrument is quotable"
            );
            return;
        }
        tracing::warn!(
            tradeable = audit.tradeable,
            cross_walked = audit.cross_walked,
            unquotable = audit.unquotable.len(),
            instruments = %audit.summary(),
            "TRADEABLE BUT UNQUOTABLE: no liquidity provider can price these instruments — \
             a hedge on one cannot fill on a named LP and will backstop to the composite"
        );
    }

    /// Diagnose why a composite lookup for `instrument_id` found nothing, so a caller can
    /// log an **unfed book** (covered but no fresh/well-formed line) distinctly from a
    /// **key-mismatch / uncovered** instrument (no enabled book's scope admits the id) —
    /// turning the otherwise silent `None` from [`Self::resolve_rfq_composite`] into an
    /// actionable signal. Off the pinned pricer (short read locks), for a `debug`-level log.
    #[must_use]
    pub fn diagnose_composite_miss(&self, instrument_id: &str) -> CompositeMiss {
        let covered = {
            let books = self.books.read().expect("aggregation books lock poisoned");
            books.iter().any(|(_, engine)| {
                let cfg = engine.cfg.lock().expect("book cfg lock poisoned");
                scope_admits(&cfg.scope, instrument_id)
            })
        };
        if covered {
            CompositeMiss::CoveredButNoLine
        } else {
            CompositeMiss::NoCoveringBook
        }
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
        for book_id in self.covering_books(instrument_id) {
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

/// Why a composite lookup found nothing (see [`AggregationHub::diagnose_composite_miss`]).
/// Distinguishes the two failure modes the previously-silent `None` conflated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositeMiss {
    /// No enabled book's scope admits the id — a genuinely uncovered instrument or a
    /// symbol↔`instrument_id` key mismatch (the FIX symbol is not the canonical id).
    NoCoveringBook,
    /// A book covers the id but publishes no fresh, well-formed line for it right now
    /// (below quorum / all members stale / a degenerate crossed two-way) — an "unfed" book.
    CoveredButNoLine,
}

impl CompositeMiss {
    /// A stable snake_case label for structured logs.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            CompositeMiss::NoCoveringBook => "no_covering_book",
            CompositeMiss::CoveredButNoLine => "covered_but_no_line",
        }
    }
}

/// Whether a book's instrument scope admits `instrument_id`.
fn scope_admits(scope: &Scope, instrument_id: &str) -> bool {
    match scope {
        Scope::AllMembersQuote => true,
        Scope::Explicit(ids) => ids.iter().any(|i| i == instrument_id),
    }
}

/// Whether `scope` admits **any identifier `query` carries** — its own id, its CUSIP, or
/// its ISIN. An `Explicit` scope is a list of literal wire ids, so a security the firm
/// trades under a registry slug is not listed there even when the book's LPs quote it
/// under its CUSIP. Comparison stays literal (case-insensitive only for the
/// cross-references, which are defined over `[0-9A-Z]`); nothing is approximated.
fn scope_admits_any(scope: &Scope, query: &IdentifierSet) -> bool {
    match scope {
        Scope::AllMembersQuote => true,
        Scope::Explicit(ids) => {
            if ids.iter().any(|i| i == &query.instrument_id) {
                return true;
            }
            let cusip = query.cusip.trim();
            let isin = query.isin.trim();
            ids.iter().any(|i| {
                let i = i.trim();
                (!cusip.is_empty() && i.eq_ignore_ascii_case(cusip))
                    || (!isin.is_empty() && i.eq_ignore_ascii_case(isin))
            })
        }
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
    use crate::config::reference_data::InstrumentDef;

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

    /// A **crossed** composite must still fill a hedge on a real, named LP.
    ///
    /// A two-member book below the divergence-gating quorum (the deployed `ust` shape)
    /// consolidates `best_bid = max(bids)` and `best_offer = min(offers)`, so two members
    /// whose mids sit further apart than their own spreads produce `best_bid >
    /// best_offer`. `resolve_rfq_composite` rightly refuses to quote that two-way out —
    /// but the panel underneath is fine: both members show real, firm, one-sided prices,
    /// and a shed hits exactly one of them. Before the fix `best_fill` inherited the
    /// composite guard and returned `None` here, so every auto-hedge backstopped to the
    /// synthetic COMPOSITE venue and street-side LP analytics never recorded a fill.
    #[test]
    fn a_crossed_composite_still_fills_on_a_named_lp() {
        use crate::services::auto_hedge::LpHedgeSource;
        let hub = hub_with(def("b", &["LP-1", "LP-2"], params(true, 1, 60_000)));
        // LP-1 marks the bond a full point above LP-2: max bid 92.70 > min offer 92.02.
        assert!(hub.ingest(&lp_quote("LP-1", "CUSIP-X", 92.70, 92.74, NOW)));
        assert!(hub.ingest(&lp_quote("LP-2", "CUSIP-X", 91.98, 92.02, NOW)));
        let inst = &hub.snapshot("b").expect("book").snapshot.instruments[0];
        assert!(
            inst.best_bid > inst.best_offer,
            "the fixture must actually be crossed: {} vs {}",
            inst.best_bid,
            inst.best_offer
        );

        // The composite two-way is (correctly) refused — it is not quotable.
        assert!(
            hub.resolve_rfq_composite("CUSIP-X").is_none(),
            "a crossed composite must not be quoted out as a two-way"
        );

        // ...but the panel is executable, and a shed fills on the best side of it.
        // Reducing a LONG sells into the highest bid: LP-1 at 92.70.
        let sell = hub.best_fill("CUSIP-X", 1_000_000.0, 5_000_000.0);
        let sell = sell.expect("a crossed composite must not starve a real LP fill");
        assert_eq!(sell.lp_id, "LP-1");
        assert_eq!(sell.price.to_bits(), 92.70_f64.to_bits());

        // Reducing a SHORT buys at the lowest offer: LP-2 at 92.02.
        let buy = hub
            .best_fill("CUSIP-X", -1_000_000.0, 5_000_000.0)
            .expect("fills the other side too");
        assert_eq!(buy.lp_id, "LP-2");
        assert_eq!(buy.price.to_bits(), 92.02_f64.to_bits());
    }

    /// A stale member is still not executable — the fix removes the composite-level
    /// guard, not the per-member freshness one.
    #[test]
    fn a_stale_member_is_never_filled_on() {
        use crate::services::auto_hedge::LpHedgeSource;
        let hub = hub_with(def("b", &["LP-1", "LP-2"], params(false, 1, 30_000)));
        // LP-1 shows the better bid but aged out 120 s ago; LP-2 is fresh.
        assert!(hub.ingest(&lp_quote("LP-1", "CUSIP-S", 99.90, 99.94, NOW - 120 * S)));
        assert!(hub.ingest(&lp_quote("LP-2", "CUSIP-S", 99.50, 99.54, NOW)));
        let fill = hub
            .best_fill("CUSIP-S", 1_000_000.0, 1_000_000.0)
            .expect("the fresh member fills");
        assert_eq!(fill.lp_id, "LP-2", "a stale line is not executable");
        assert_eq!(fill.price.to_bits(), 99.50_f64.to_bits());
    }

    /// No covering book, or a covered book with nothing published, is an honest miss —
    /// the executor backstops rather than being handed a fabricated fill.
    #[test]
    fn an_unfed_or_uncovered_instrument_yields_no_fill() {
        use crate::services::auto_hedge::LpHedgeSource;
        let hub = hub_with(def("b", &["LP-1"], params(false, 1, 60_000)));
        assert!(hub.best_fill("NOT-QUOTED", 1.0, 1.0).is_none());
    }

    #[test]
    fn inbound_kill_switch_drops_ingest_and_leaves_composite_unchanged() {
        use crate::services::pricing_control::PricingControl;
        let control = PricingControl::new(true, true);
        let hub = AggregationHub::with_control(Clock::manual(NOW), Arc::clone(&control));
        let mut store = IdentityStore::default();
        store
            .aggregated_books
            .push(def("b", &["LP-1", "LP-2"], params(false, 1, 60_000)));
        hub.reconcile(&store);

        // Inbound enabled: the first LP tick is accepted and forms a composite.
        assert!(hub.ingest(&lp_quote("LP-1", "CUSIP-A", 99.90, 100.10, NOW)));

        // Halt inbound ingest: the next LP tick is dropped (returns false), no composite
        // mutation — the halted LP-2 line does not enter and does not set a new best.
        control.set(true, false);
        assert!(!hub.ingest(&lp_quote("LP-2", "CUSIP-A", 99.99, 100.00, NOW)));
        let inst = &hub.snapshot("b").expect("book").snapshot.instruments[0];
        assert_eq!(
            inst.contributions.len(),
            1,
            "the halted LP-2 tick did not enter the book"
        );
        assert_eq!(inst.best_offer.to_bits(), 100.10_f64.to_bits());

        // Re-enable: the next tick ingests live (no stale replay of the dropped one).
        control.set(true, true);
        assert!(hub.ingest(&lp_quote("LP-2", "CUSIP-A", 99.99, 100.00, NOW)));
        let inst2 = &hub.snapshot("b").expect("book").snapshot.instruments[0];
        assert_eq!(inst2.contributions.len(), 2);
        assert_eq!(inst2.best_offer.to_bits(), 100.00_f64.to_bits());
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

    // ---------------------------------------------------------------------------
    // The identifier cross-walk
    // ---------------------------------------------------------------------------

    /// A real curated US Treasury, whose CUSIP is what an LP puts on the wire and whose
    /// ISIN is the cross-reference a registry entry would carry.
    fn curated_ust() -> celnet_refdata::GovBondSpec {
        celnet_refdata::government_universe()
            .into_iter()
            .find(|s| s.cusip.is_some())
            .expect("the curated universe ships US Treasuries")
    }

    /// A registry entry for `id` carrying the given cross-references — the shape of a
    /// bond an admin defines under the firm's own slug.
    fn bond_def(id: &str, cusip: &str, isin: &str) -> InstrumentDef {
        use crate::config::reference_data::{BondDef, CivilDate, ExternalId};
        let mut external_ids = Vec::new();
        if !cusip.is_empty() {
            external_ids.push(ExternalId {
                scheme: "cusip".to_string(),
                value: cusip.to_string(),
            });
        }
        if !isin.is_empty() {
            external_ids.push(ExternalId {
                scheme: "isin".to_string(),
                value: isin.to_string(),
            });
        }
        InstrumentDef {
            instrument_id: id.to_string(),
            name: format!("registry entry {id}"),
            description: "cross-walk fixture".to_string(),
            currency: "USD".to_string(),
            external_ids,
            definition: InstrumentFamily::Bond(BondDef {
                issuer: "US Treasury".to_string(),
                coupon_rate: 0.04,
                coupon_type: "fixed".to_string(),
                coupon_frequency: "semi_annual".to_string(),
                day_count: "act_act".to_string(),
                issue_date: None,
                dated_date: None,
                first_coupon_date: None,
                maturity_date: CivilDate {
                    year: 2030,
                    month: 6,
                    day: 30,
                },
                redemption: 100.0,
                calendars: vec!["united_states".to_string()],
            }),
        }
    }

    /// A hub whose book is fed by `members`, reconciled against a registry holding
    /// `instruments` — so the identity map the cross-walk queries is populated.
    fn hub_with_registry(members: &[&str], instruments: Vec<InstrumentDef>) -> Arc<AggregationHub> {
        let hub = AggregationHub::new(Clock::manual(NOW));
        let mut store = IdentityStore::default();
        store
            .aggregated_books
            .push(def("b", members, params(false, 1, 60_000)));
        store.instruments = instruments;
        hub.reconcile(&store);
        hub
    }

    /// **THE DEFECT.** The panel is keyed by the CUSIP the LPs stream; the registry knows
    /// the same security under a slug. Before the cross-walk the exact-match lookup missed,
    /// `best_fill` returned `None`, and the hedge backstopped to the synthetic COMPOSITE
    /// venue — recording a fill no LP ever made. It must now fill on a NAMED LP.
    #[test]
    fn a_hedge_on_a_registry_slug_fills_on_the_named_lp_quoting_its_cusip() {
        use crate::services::auto_hedge::LpHedgeSource;
        let spec = curated_ust();
        let cusip = spec.cusip.clone().expect("US row has a CUSIP");
        let hub = hub_with_registry(
            &["LP-1", "LP-2"],
            vec![bond_def("firm-slug-for-the-note", &cusip, &spec.isin)],
        );
        // The venues quote the security under its CUSIP, which is NOT the registry id.
        assert!(hub.ingest(&lp_quote("LP-1", &cusip, 99.90, 100.10, NOW)));
        assert!(hub.ingest(&lp_quote("LP-2", &cusip, 99.95, 100.05, NOW)));

        // Shedding a long sells into a bid: the best bid is LP-2's 99.95.
        let fill = hub
            .best_fill("firm-slug-for-the-note", 1_000_000.0, 1_000_000.0)
            .expect("the cross-walk must find the panel the LPs are quoting");
        assert_eq!(fill.lp_id, "LP-2", "filled on a real, named LP");
        assert_eq!(fill.price.to_bits(), 99.95_f64.to_bits());
        // …and the same security reached by its ISIN alone resolves to the same panel.
        let by_isin = hub
            .resolve_member_panel("b", &spec.isin)
            .expect("ISIN cross-walks onto the CUSIP-keyed panel");
        assert_eq!(by_isin.len(), 2);
    }

    /// The precedence is stable and observable end to end: an exact `instrument_id` hit
    /// outranks the cross-references even when they name a DIFFERENT quoted line.
    #[test]
    fn an_exact_instrument_id_outranks_the_cross_references() {
        let spec = curated_ust();
        let cusip = spec.cusip.clone().expect("US row has a CUSIP");
        // The registry entry is keyed by "DIRECT" but cross-references the UST's CUSIP.
        let hub = hub_with_registry(&["LP-1"], vec![bond_def("DIRECT", &cusip, &spec.isin)]);
        assert!(hub.ingest(&lp_quote("LP-1", "DIRECT", 50.0, 50.5, NOW)));
        assert!(hub.ingest(&lp_quote("LP-1", &cusip, 99.9, 100.1, NOW)));

        let members = hub
            .resolve_member_panel("b", "DIRECT")
            .expect("the exact id resolves");
        assert_eq!(members.len(), 1);
        assert_eq!(
            members[0].bid.to_bits(),
            50.0_f64.to_bits(),
            "the exact `instrument_id` line won, not the CUSIP cross-reference's"
        );
    }

    /// An id in nobody's namespace must stay unresolved — the old behaviour, kept. A
    /// hedge that cannot find a venue records an honest miss; it never approximates onto
    /// a different bond just because one is nearby.
    #[test]
    fn an_unresolvable_instrument_never_approximates_onto_another_bond() {
        use crate::services::auto_hedge::LpHedgeSource;
        let spec = curated_ust();
        let cusip = spec.cusip.clone().expect("US row has a CUSIP");
        // The registry advertises a security with NO cross-references at all.
        let hub = hub_with_registry(&["LP-1"], vec![bond_def("nobody-quotes-me", "", "")]);
        assert!(hub.ingest(&lp_quote("LP-1", &cusip, 99.90, 100.10, NOW)));

        assert!(
            hub.best_fill("nobody-quotes-me", 1_000_000.0, 1_000_000.0)
                .is_none(),
            "an unresolvable id must not fill on an unrelated bond's panel"
        );
        assert!(hub.resolve_member_panel("b", "nobody-quotes-me").is_none());
        // A plausible-but-wrong CUSIP prefix is not a match either.
        assert!(
            hub.resolve_member_panel("b", &cusip[..cusip.len() - 1])
                .is_none()
        );
    }

    /// The curated-universe check the audit uses applies the SAME documented precedence
    /// as the live lookup, and refuses to resolve anything it does not know exactly.
    #[test]
    fn the_curated_basis_follows_the_documented_precedence() {
        let spec = curated_ust();
        let cusip = spec.cusip.clone().expect("US row has a CUSIP");
        assert_eq!(
            curated_basis(&IdentifierSet::from_id(&spec.instrument_id)),
            Some(CrosswalkBasis::Exact)
        );
        assert_eq!(
            curated_basis(&IdentifierSet {
                instrument_id: "firm-slug".to_string(),
                cusip: cusip.clone(),
                isin: spec.isin.clone(),
            }),
            Some(CrosswalkBasis::Cusip),
            "CUSIP outranks ISIN when both would hit"
        );
        assert_eq!(
            curated_basis(&IdentifierSet {
                instrument_id: "firm-slug".to_string(),
                cusip: String::new(),
                isin: spec.isin.clone(),
            }),
            Some(CrosswalkBasis::Isin)
        );
        assert_eq!(curated_basis(&IdentifierSet::from_id("acme-5y-corp")), None);
        assert_eq!(curated_basis(&IdentifierSet::default()), None);
    }

    /// The guard: a bond the firm advertises as tradeable that no venue can price is
    /// reported by name; a bond the LPs actually quote is not. It must be quiet when the
    /// tradeable and quotable universes coincide, or the desk learns to ignore it.
    #[test]
    fn the_audit_names_tradeable_instruments_no_venue_can_quote() {
        let spec = curated_ust();
        let cusip = spec.cusip.clone().expect("US row has a CUSIP");
        let hub = hub_with_registry(
            &["LP-1"],
            vec![
                // (a) curated, reached by its own id — quotable, exactly.
                bond_def(&spec.instrument_id, "", ""),
                // (b) a firm slug that cross-walks onto the curated row by CUSIP.
                bond_def("firm-slug", &cusip, &spec.isin),
                // (c) an invented security no LP prices — the defect shape.
                bond_def("acme-5y-corp", "", "US000402AA77"),
            ],
        );
        let mut store = IdentityStore {
            instruments: vec![
                bond_def(&spec.instrument_id, "", ""),
                bond_def("firm-slug", &cusip, &spec.isin),
                bond_def("acme-5y-corp", "", "US000402AA77"),
            ],
            ..Default::default()
        };
        let audit = hub.audit_unquotable(&store);
        assert_eq!(audit.tradeable, 3);
        assert_eq!(
            audit.cross_walked, 1,
            "only the firm slug needed a cross-ref"
        );
        assert_eq!(audit.unquotable.len(), 1);
        assert_eq!(audit.unquotable[0].instrument_id, "acme-5y-corp");
        assert_eq!(audit.unquotable[0].isin, "US000402AA77");
        assert!(!audit.is_clean());
        assert!(audit.summary().contains("acme-5y-corp"));

        // Drop the unquotable one and the audit goes quiet.
        store.instruments.pop();
        let clean = hub.audit_unquotable(&store);
        assert!(clean.is_clean(), "no false positives on a healthy registry");
        assert_eq!(clean.tradeable, 2);
        assert!(clean.summary().is_empty());
    }

    /// The guard applied to the REAL boot universe, not a fixture: exactly what
    /// `celnet-server` registers on a pristine store — the rates seed plus the full
    /// curated government-bond and Treasury-futures complex.
    ///
    /// Every tradeable instrument the firm advertises must be one some liquidity provider
    /// can price. A bond in this registry that no LP quotes cannot fill on a named venue;
    /// it backstops to the synthetic composite and books a fill that never happened. That
    /// is the defect this whole cross-walk exists to close, so the shipped universe
    /// asserts it, permanently.
    #[test]
    fn the_real_boot_universe_is_entirely_quotable() {
        let hub = AggregationHub::new(Clock::manual(NOW));
        let mut store = IdentityStore::default();
        store.ensure_seed_instruments();
        store.ensure_seed_government_bonds();
        hub.reconcile(&store);
        let audit = hub.audit_unquotable(&store);
        assert!(
            audit.tradeable > 100,
            "the boot universe must actually be seeded, got {} tradeable",
            audit.tradeable
        );
        assert!(
            audit.is_clean(),
            "the boot registry advertises {} of {} tradeable instruments that no liquidity \
             provider can quote: {}",
            audit.unquotable.len(),
            audit.tradeable,
            audit.summary()
        );
    }
}
