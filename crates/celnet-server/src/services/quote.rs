//! The RFQ lifecycle service: `RequestQuote` → `Quote` (and
//! `RequestMultiDealerQuote` → `MultiDealerQuote`, the ranked LP panel),
//! `AcceptQuote` → `Execution`, `RejectQuote` → `RejectAck`.
//!
//! This is the request-for-quote half of the trader workflow. A client sends a
//! [`celnet_proto::QuoteRequest`] carrying an [`celnet_proto::Instrument`], a
//! [`celnet_proto::Conventions`], and a client-supplied **idempotency key**; the
//! service:
//!
//! 1. prices the instrument through [`crate::pricer`] against the instrument's
//!    market context (carried in the request via the edge's published market
//!    state — see below), producing a mid and the full Greek set;
//! 2. turns the mid into a tradable two-way bid/offer via the [`crate::spread`]
//!    model;
//! 3. assigns a stable, **unguessable** `quote_id` (a fresh monotonic counter
//!    folded through a per-process-keyed `splitmix64` bijection — unique but not
//!    enumerable), stamps a publication time and a `valid_until` last-look
//!    deadline, and stores the quote keyed by both `quote_id` and the client
//!    idempotency key.
//!
//! # Idempotency
//!
//! The idempotency key makes a retried request safe: a second `RequestQuote`
//! carrying a key already seen returns the **same** stored [`celnet_proto::Quote`]
//! (same `quote_id`, same prices), never re-pricing and never issuing a new id. A
//! retried `AcceptQuote` on an already-booked quote returns the **same**
//! [`celnet_proto::Execution`], so a network retry can never double-book.
//!
//! # Trust model (caller-gated, requester-bound accept, unguessable id)
//!
//! **Caller gate (item B §2).** Every RFQ RPC (`RequestQuote`,
//! `RequestMultiDealerQuote`, `AcceptQuote`, `RejectQuote`) passes the same
//! session-aware authorization boundary the risk reads do
//! ([`crate::services::access::authorize_caller`] with [`RequiredAuthority::ReadAny`]):
//! the caller is resolved from the request's `session_token` (validated against the
//! shared [`SessionRegistry`]) + asserted `principal`, and an unauthenticated caller
//! is denied by default under [`AccessMode::Enforce`](celnet_entitlements::AccessMode)
//! (admitted, audited, under the explicit permissive demo mode). The access mode is
//! read live from the shared [`PositionStore`], so gRPC and the WS mirror — which
//! dispatch onto the SAME trait methods, the caller riding in the unary body —
//! enforce one coherent policy with no router change.
//!
//! **Requester-bound accept (item B §2).** Beyond the request-matched idempotency
//! key, the stored quote records the requesting caller's identity
//! ([`RequesterBinding`]). When the requester was an **authenticated** user, an
//! `AcceptQuote` is admitted only from that SAME authenticated user — a different
//! authenticated principal is refused `permission_denied`, closing "any party that
//! learns a `quote_id` books another requester's quote". An **anonymous**-requested
//! quote (the permissive demo path) keeps the legacy idempotency-only behaviour
//! unchanged.
//!
//! `quote_id` is **not** an authority token. `AcceptQuote` is request-matched to
//! the originating idempotency key: an accept whose `idempotency_key` does not
//! equal the key the quote was minted under is refused with `invalid_argument`, so
//! a party that merely learns a `quote_id` cannot accept (or hijack the booking
//! of) another requester's live quote. An accept-retry returns the stored
//! execution only when both the key **and** the traded side match; a side flip is
//! a different intent and is refused with `failed_precondition`.
//!
//! `RejectQuote` carries no key on the wire (the contract has only `quote_id` +
//! free-text `reason`), so it cannot be request-matched the same way. Instead,
//! every `quote_id` is **unguessable** — minted by folding a strictly-fresh
//! monotonic counter through a `splitmix64` bijection keyed by a per-process
//! secret — so the id space is sparse and unenumerable: only a party that received
//! the quote (and thus holds its id) can decline it. A re-reject is idempotent.
//!
//! # Multi-dealer panel (RFQ-to-many)
//!
//! `RequestMultiDealerQuote` runs the same priced quote through the `celnet-rfq`
//! `MultiDealerEngine` over the edge's LP panel — the native maker auto-pricer
//! plus the configured deterministic synthetic demo/test dealers
//! ([`LpPanelConfig`]; live LP connectivity is ENV, never claimed in-repo) — and
//! **pins** the ranked rows onto the stored quote record. An `AcceptQuote`
//! carrying a row's `lp_id` then books exactly that pinned line (its price,
//! validity, and LP attribution), side- and line-matched on retry; an empty
//! `lp_id` stays the single-dealer path byte-identical.
//!
//! # Market context
//!
//! An RFQ does not carry a `MarketContext` on the wire (the request is a trader
//! intent, not a market snapshot); the maker prices it against its own live
//! market. The service reads the engine's live [`celnet_engine::MarketState`]
//! through the [`CoreLink`] and projects it into the
//! [`celnet_proto::MarketContext`] the pricer consumes, so a quote always reflects
//! the maker's current market.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::tick::TickSource;

use celnet_proto::quote_service_server::QuoteService;
use celnet_proto::{
    AttributionRecord, BookId, DealerQuote, Execution, MarketContext, MultiDealerQuote, Owner,
    Quote, QuoteAccept, QuoteReject, QuoteRequest, RatesQuote, RatesQuoteRequest, RejectAck, Side,
    TwoWayPrice, owner,
};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use celnet_entitlements::{Action, AssetClass};

use celnet_core::carry::CarryInputs;
use celnet_risk_normalize::PositionRisk;

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::pricer::{
    ConventionSet, CryptoSettlementStyle, Priced, cost_of_carry, decode_settlement_style,
    is_cross_asset, price_instrument,
};
use crate::readiness::ReadinessGate;
use crate::services::access::{
    RequiredAuthority, ResolvedCaller, authorize_caller, resolve_caller,
};
use crate::services::error_status::{
    link_error_to_status, price_error_to_status, rates_price_error_to_status,
};
use crate::services::forward::{Serve, route_underlying, serve_mode};
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::services::risk::federate::Fleet;
use crate::services::risk::store::{BookedPosition, PositionStore, limit_breached_status};
use crate::services::sessions::SessionRegistry;
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

use celnet_router::ReplicaId;

/// How long an issued quote stays valid for a last-look accept, in nanoseconds
/// (5 seconds — the typical OTC last-look window).
const QUOTE_VALIDITY_NANOS: i64 = 5_000_000_000;

/// The per-source deadline the multi-dealer RFQ engine imposes on each panel
/// source: a source that has not answered within this window is dropped (treated
/// as a non-responder), so a wedged LP can never hang the panel. The native
/// in-process source answers synchronously and is never affected; this bounds only
/// the external FIX LP legs (when an ENV-configured panel is present).
const RFQ_PANEL_DEADLINE: std::time::Duration = std::time::Duration::from_millis(250);

/// The deterministic synthetic LP panel an edge fans a multi-dealer RFQ across,
/// in addition to the always-present native maker auto-pricer.
///
/// **Honest boundary:** live LP connectivity (real bank sessions over WAN FIX)
/// is ENV — designed and seamed in-repo, validated at deploy, never claimed
/// in-repo. The in-repo panel is the native maker plus `synthetic_lps`
/// **deterministic synthetic** dealers (`SYNTH-LP-1` … `SYNTH-LP-N`), each
/// quoting around the SAME edge-priced mid with fixed per-dealer spread/skew
/// offsets ([`synthetic_lp_two_way`]) — a labeled demo/test panel that exercises
/// the full aggregation → ranking → pinning → booking path with real ranked
/// quotes, never faked fills.
///
/// `synthetic_lps == 0` (the default) keeps the panel native-only, so every
/// single-dealer path is byte-identical to the panel-less edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LpPanelConfig {
    /// How many deterministic synthetic demo/test dealers join the panel beside
    /// the native maker.
    pub synthetic_lps: u32,
}

impl LpPanelConfig {
    /// The deploy-time env knob naming the synthetic demo/test panel breadth.
    pub const ENV_VAR: &str = "CELNET_DEMO_LPS";

    /// Read the panel breadth from [`Self::ENV_VAR`]; absent or unparseable ⇒
    /// `0` (native-only — the byte-identical single-dealer edge).
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_env_or(0)
    }

    /// Read the panel breadth from [`Self::ENV_VAR`] with an explicit default for
    /// when the variable is absent/unparseable (the demo edge boots a 3-LP panel
    /// by default so the live e2e suites exercise the multi-dealer path).
    #[must_use]
    pub fn from_env_or(default_lps: u32) -> Self {
        let synthetic_lps = std::env::var(Self::ENV_VAR)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(default_lps);
        Self { synthetic_lps }
    }
}

/// The stable audit `lp_id` of synthetic demo/test dealer `k` (1-based).
fn synthetic_lp_id(k: u32) -> String {
    format!("SYNTH-LP-{k}")
}

/// The deterministic `(mid, half_spread)` synthetic demo/test dealer `k`
/// (1-based) quotes around the SAME edge-priced maker mid.
///
/// Per-dealer offsets are fixed by construction so the ranked panel is
/// reproducible and independently checkable:
/// * **spread** — dealer `k` quotes `5%·k` wider than the maker half-spread
///   (`half_k = half·(1 + 0.05·k)`): a synthetic dealer never charges less risk
///   compensation than the engine's own spread model;
/// * **skew** — dealer `k` shades its mid by a quarter maker half-spread, odd
///   dealers up (`+0.25·half` ⇒ a stronger bid), even dealers down
///   (`−0.25·half` ⇒ a cheaper offer). For any panel of ≥ 2 the engine's
///   best-bid/best-offer law therefore selects `SYNTH-LP-1` (bid
///   `mid − 0.80·half`) and `SYNTH-LP-2` (offer `mid + 0.85·half`)
///   deterministically, and the panel touch is never crossed.
fn synthetic_lp_two_way(k: u32, mid: f64, half_spread: f64) -> (f64, f64) {
    let widen = 1.0 + 0.05 * f64::from(k);
    let shade = 0.25 * half_spread;
    let skew = if k % 2 == 1 { shade } else { -shade };
    (mid + skew, half_spread * widen)
}

/// The attribution stamped on a non-native dealer panel line — a synthetic demo dealer
/// OR a real aggregated-book member LP (Phase 2b): the LP's own auto-pricer seat quoted
/// it, and the client's requesting seat (when supplied on the originating request) holds
/// a booking of it — mirroring how the maker line is attributed by
/// [`super::attribution::resolve`]. A booked dealer line's execution therefore carries the
/// LP identity, never an anonymous fill.
fn lp_line_attribution(lp_id: &str, held_by: Option<BookId>) -> AttributionRecord {
    AttributionRecord {
        quoted_by: Some(BookId {
            book: lp_id.to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::AutoPricer(lp_id.to_owned())),
            }),
        }),
        held_by,
        won: None,
        lp_count: None,
    }
}

/// **The identity bound to a quote at request time** (item B §2): the resolved
/// caller that requested the quote, so an [`QuoteService::accept_quote`] can be
/// required to come from the SAME caller.
///
/// * [`RequesterBinding::Authenticated`] — the requester held a valid session token;
///   the wrapped string is their stable [`AuthenticatedUser::user_id`]. An accept is
///   admitted only from an authenticated caller with the SAME id.
/// * [`RequesterBinding::Anonymous`] — the requester carried no authenticated session
///   (the permissive demo path); the accept keeps the legacy idempotency-only
///   behaviour, no binding check.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RequesterBinding {
    /// The requester authenticated as this stable user id.
    Authenticated(String),
    /// The requester carried no authenticated session (permissive/demo path).
    Anonymous,
}

/// The [`RequesterBinding`] for a resolved caller: an authenticated user binds to
/// its stable id; an unauthenticated caller is [`RequesterBinding::Anonymous`]
/// (binding is keyed on the SERVER-validated session, never a client-asserted
/// principal — an asserted principal is not an authenticated identity).
fn caller_binding(caller: &ResolvedCaller) -> RequesterBinding {
    caller.user().map_or(RequesterBinding::Anonymous, |u| {
        RequesterBinding::Authenticated(u.user_id.clone())
    })
}

/// A booked, immutable quote record kept for idempotent re-request and accept.
#[derive(Debug, Clone)]
struct QuoteRecord {
    quote: Quote,
    /// The instrument the quote priced (echoed into the booking on accept).
    instrument: celnet_proto::Instrument,
    /// The execution booked from this quote, if it has been accepted. Storing it
    /// makes `AcceptQuote` idempotent: a retry returns the same booking.
    execution: Option<Execution>,
    /// Whether the quote has been declined (rejected). A rejected quote can no
    /// longer be accepted; a re-reject is idempotent.
    rejected: bool,
    /// The multi-dealer panel rows pinned for this `quote_id` (empty until a
    /// `RequestMultiDealerQuote` ran a panel over the quote). An `AcceptQuote`
    /// naming a non-native `lp_id` books against the matching pinned row — the
    /// exact price/validity/attribution the client was shown — never a re-price.
    dealers: Vec<DealerQuote>,
    /// The dealer line the booking traded on: the normalized `lp_id` (an empty
    /// accept normalizes to the native maker). Meaningful only once `execution`
    /// is set; polices accept-retries (a retry naming a different line is a
    /// different trade intent, not a retry).
    booked_lp_id: String,
    /// **The caller that requested this quote** (item B §2): an `AcceptQuote` is
    /// bound to this requester when it was authenticated — a different authenticated
    /// principal is refused. An anonymous requester keeps idempotency-only accepts.
    requester: RequesterBinding,
    /// The **class-correct** pre-trade risk leaf captured at quote time (ADR-0016 A1;
    /// generalized under ADR-0021 uniform-asset-class): the BUY-side exposure this quote
    /// would book, so an `AcceptQuote` can run the pre-trade limit gate against the
    /// shared position book without re-pricing (a `Side=SELL` accept negates the
    /// notional). FX vanilla carries a canonical-vanilla [`BookedPosition`], a
    /// cross-asset (equity/commodity/linear-crypto) vanilla its cost-of-carry
    /// [`PositionRisk`] — each gated through the SAME unified aggregate. `None` for an
    /// instrument that carries no derivable canonical leaf (a non-vanilla product, a
    /// zero notional, or the deferred inverse-crypto seam) — an explicit no-gate
    /// outcome, never a fabricated zero.
    pre_trade: Option<PreTradeLeaf>,
}

/// The class-correct pre-trade risk leaf a quote captures at request time, so an
/// [`QuoteService::accept_quote`] can gate the booking against the shared position book
/// without re-pricing. Generalized across asset classes (ADR-0021 uniform-asset-class):
/// a quote of any priceable class carries its OWN class-correct exposure into the
/// pre-trade gate — never a silent zero for a non-FX quote (guardrail #2).
#[derive(Debug, Clone)]
enum PreTradeLeaf {
    /// FX vanilla — the canonical-vanilla [`BookedPosition`] gated through the FX
    /// [`PositionStore::evaluate_pre_trade`] path (byte-identical to the prior FX-only
    /// leaf).
    FxVanilla(BookedPosition),
    /// Cross-asset vanilla (equity / commodity / linear digital-asset) — its normalized
    /// cost-of-carry [`PositionRisk`] gated through the class-parametric
    /// [`PositionStore::evaluate_pre_trade_position`], re-canonicalized through the
    /// position's own asset-class leaf so the gate sees its real class-correct exposure.
    CrossAsset(PositionRisk),
}

/// The in-memory RFQ store: idempotency-key → quote_id, and quote_id → record.
///
/// Bounded only by the RFQ rate over a deployment lifetime (a production edge
/// expires old quotes; the records here are small and the test workload is
/// finite). Guarded by an async [`Mutex`] held only for the brief map operations —
/// never across a price call.
#[derive(Debug, Default)]
struct QuoteStore {
    by_key: HashMap<String, u64>,
    by_id: HashMap<u64, QuoteRecord>,
}

/// The RFQ service over the [`CoreLink`] and the readiness gate.
#[derive(Debug)]
pub struct QuoteEdge {
    link: Arc<CoreLink>,
    gate: Arc<ReadinessGate>,
    spread: SpreadModel,
    clock: Clock,
    next_quote_id: AtomicU64,
    next_execution_id: AtomicU64,
    /// A per-process secret folded into every minted `quote_id` so ids are
    /// **unguessable** (not a dense monotonic sequence a third party can
    /// enumerate). Combined with the idempotency-key match on accept, this is the
    /// in-band trust model: `quote_id` alone is not an authority token — accepting
    /// requires the originating key, and rejecting requires possession of the
    /// (unguessable) id. Seeded from the OS-random [`RandomState`], never the
    /// pricing path, so it does not touch the deterministic core.
    quote_id_secret: u64,
    store: Mutex<QuoteStore>,
    /// The shared versioned marked-surface registry: a `QuoteRequest` carrying a
    /// pinned `surface_version` prices against the marked surface (and echoes it).
    surface_book: Arc<SurfaceBook>,
    /// The connected backend fleet for owned-pair forwarding; `None` ⇒ in-process
    /// (quote locally). Reuses the SAME `Fleet` the risk federation connects.
    fleet: Option<Arc<Fleet>>,
    /// Distributed mode only: `quote_id → issuing backend replica`, recorded when a
    /// `RequestQuote` is forwarded, so a later `AcceptQuote` / `RejectQuote` (which
    /// carry only `quote_id`, no pair, on the wire) routes back to the **same**
    /// backend that minted the id (a `quote_id` is minted per-backend under that
    /// backend's secret). Bounded by the live-quote rate over a session, like the
    /// backend's own `QuoteStore`.
    quote_owners: Mutex<HashMap<u64, ReplicaId>>,
    /// The synthetic demo/test LP panel joined to the native maker on a
    /// `RequestMultiDealerQuote` (see [`LpPanelConfig`]; `0` ⇒ native-only,
    /// byte-identical single-dealer behaviour).
    panel: LpPanelConfig,
    /// The edge-wide session registry the RFQ caller gate validates `session_token`
    /// against — the SAME authentication state every other gated edge shares (item
    /// B §2). Defaulted to an empty registry by the constructors; the boot path
    /// installs the shared one via [`QuoteEdge::with_session_access`].
    sessions: Arc<SessionRegistry>,
    /// The shared live position book, read only for its runtime-settable
    /// [`PositionStore::access_mode`] — the ONE access posture the whole edge gates
    /// under (gRPC + WS, every service). Named `access_store` to keep the RFQ
    /// [`QuoteStore`] field (`store`) distinct.
    access_store: Arc<PositionStore>,
    /// The optional edge-wide aggregated-book engine hub (D3 / Phase 2b). When an inbound
    /// RFQ's instrument falls in an admin-defined book's scope and that book has a live
    /// composite, the quote is priced against the book's ALREADY-TIERED composite (and its
    /// member-LP lines rank the multi-dealer panel) instead of the `CELNET_DEMO_LPS`
    /// synthetic panel. `None` (the constructors' default) ⇒ every RFQ prices through the
    /// options engine + synthetic panel exactly as before — byte-identical when no book
    /// covers the instrument. Installed by the boot path via
    /// [`QuoteEdge::with_aggregation_hub`]; the SAME hub the ingest / stream / auth edges
    /// share.
    aggregation_hub: Option<Arc<crate::services::aggregation::AggregationHub>>,
}

impl QuoteEdge {
    /// Construct the RFQ service in the in-process topology (quotes locally,
    /// native-only multi-dealer panel).
    #[must_use]
    pub fn new(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
    ) -> Self {
        Self::with_fleet(
            link,
            gate,
            spread,
            clock,
            surface_book,
            None,
            LpPanelConfig::default(),
        )
    }

    /// Construct the RFQ service with an optional connected backend [`Fleet`]
    /// (`Some(fleet)` ⇒ distributed: forward `RequestQuote` by the instrument's
    /// pair, and route the matching `AcceptQuote` / `RejectQuote` back to the
    /// issuing backend; `None` ⇒ in-process, exactly [`QuoteEdge::new`]'s prior
    /// behaviour) and an explicit synthetic LP-panel breadth for the
    /// multi-dealer path (resolved once at edge boot — env or caller-chosen).
    #[must_use]
    pub fn with_fleet(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        fleet: Option<Arc<Fleet>>,
        panel: LpPanelConfig,
    ) -> Self {
        // Mint a process-unique unguessable secret from the OS-seeded hasher. This
        // is a control-plane identity concern, *not* a pricing path, so OS entropy
        // is appropriate (the determinism / counter-RNG guardrail governs pricing,
        // which never consumes this value).
        let quote_id_secret = RandomState::new().build_hasher().finish();
        let sessions = Arc::new(SessionRegistry::new(clock.clone()));
        Self {
            link,
            gate,
            spread,
            clock,
            next_quote_id: AtomicU64::new(1),
            next_execution_id: AtomicU64::new(1),
            quote_id_secret,
            store: Mutex::new(QuoteStore::default()),
            surface_book,
            fleet,
            quote_owners: Mutex::new(HashMap::new()),
            panel,
            sessions,
            access_store: Arc::new(PositionStore::new()),
            aggregation_hub: None,
        }
    }

    /// Install the edge-wide [`SessionRegistry`] (validates the RFQ caller's
    /// `session_token`) and the shared [`PositionStore`] (the runtime-settable
    /// access-mode authority), so the RFQ caller gate (item B §2) enforces under the
    /// SAME posture every other gated edge does. Builder-style, mirroring
    /// [`RiskEdge::with_sessions`](crate::services::risk::RiskEdge::with_sessions);
    /// the constructors otherwise default to an empty registry + a private store, so
    /// existing test constructors keep compiling (an empty registry rejects every
    /// token, which is correct for a no-auth test edge).
    #[must_use]
    pub fn with_session_access(
        mut self,
        sessions: Arc<SessionRegistry>,
        access_store: Arc<PositionStore>,
    ) -> Self {
        self.sessions = sessions;
        self.access_store = access_store;
        self
    }

    /// Install the edge-wide aggregated-book engine hub so an inbound RFQ on an instrument
    /// an admin-defined book covers prices against that book's ALREADY-TIERED composite
    /// (and ranks its member-LP lines on the multi-dealer panel) instead of the
    /// synthetic-demo panel (Phase 2b). Absent ⇒ the byte-identical synthetic-panel
    /// behaviour when no book is configured. Builder-style, mirroring
    /// [`StreamEdge::with_aggregation_hub`](crate::services::stream::StreamEdge) — the boot
    /// path installs the SAME hub the LP ingest and stream edges share.
    #[must_use]
    pub fn with_aggregation_hub(
        mut self,
        hub: Arc<crate::services::aggregation::AggregationHub>,
    ) -> Self {
        self.aggregation_hub = Some(hub);
        self
    }

    /// Resolve the aggregated-book composite an inbound RFQ's instrument prices against
    /// (Phase 2b): the first admin-defined book whose scope admits the instrument's key
    /// and that has a live, well-formed composite line. `None` when no hub is installed,
    /// the instrument yields no book key, or no book covers it — the RFQ then prices
    /// through the options engine + synthetic panel exactly as before.
    fn book_composite(
        &self,
        instrument: &celnet_proto::Instrument,
    ) -> Option<crate::services::aggregation::RfqComposite> {
        let hub = self.aggregation_hub.as_ref()?;
        let key = book_instrument_key(instrument)?;
        hub.resolve_rfq_composite(&key)
    }

    /// Resolve the aggregated-book composite for a **specific caller**, applying the
    /// caller's pricing group when it resolves to one (`docs/FI-PRICING-GROUPS-DESIGN.md`
    /// §5). A grouped caller prices off the book's **raw** composite through its group's
    /// effective RFS/RFQ pipeline (`share_pipeline ? esp : rfq`); an ungrouped caller
    /// falls back to [`Self::book_composite`] — the book-default-tiered composite,
    /// byte-identical to before pricing groups. Resolution is by user id, then desk
    /// fallback (FIX-connection callers are the deferred rates-RFS seam).
    fn book_composite_for_caller(
        &self,
        instrument: &celnet_proto::Instrument,
        caller: &ResolvedCaller,
    ) -> Option<crate::services::aggregation::RfqComposite> {
        let hub = self.aggregation_hub.as_ref()?;
        let key = book_instrument_key(instrument)?;
        if let Some(user) = caller.user() {
            let resolver = hub.pricing_groups();
            if let Some(group) = resolver.resolve_for_user(&user.user_id, &user.desk_ids) {
                return hub.resolve_rfq_composite_priced(&key, group.rfq_effective_pipeline());
            }
        }
        hub.resolve_rfq_composite(&key)
    }

    /// Mint the next **unguessable** `quote_id`: a strictly-fresh monotonic counter
    /// folded through the public-domain `splitmix64` bijection keyed by the
    /// per-process secret. `splitmix64` is a bijection over `u64`, so distinct
    /// counters yield distinct ids (no collision) while the secret makes the id
    /// space sparse and unenumerable. The lone counter that would map to `0` is
    /// skipped so every id is `>= 1` (a valid, non-sentinel quote id).
    fn mint_quote_id(&self) -> u64 {
        loop {
            let n = self.next_quote_id.fetch_add(1, Ordering::Relaxed);
            let id = TickSource::splitmix64(self.quote_id_secret ^ n);
            if id != 0 {
                return id;
            }
        }
    }

    /// Reject the call if the edge is not accepting traffic.
    fn require_ready(&self) -> Result<(), Status> {
        if self.gate.is_ready() {
            Ok(())
        } else {
            Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ))
        }
    }

    /// Project the live engine market state into the wire market context the
    /// pricer consumes.
    async fn live_market(&self) -> Result<MarketContext, Status> {
        let snap = self
            .link
            .market_snapshot()
            .await
            .map_err(|e| link_error_to_status(&e))?;
        Ok(MarketContext::fx(
            snap.spot,
            snap.atm_vol,
            snap.r_dom,
            snap.r_for,
        ))
    }
}

/// Does an incoming request match the request the stored idempotency record was
/// minted for? An idempotency key is only honoured when replayed with the *same*
/// instrument and conventions; a key reused for a different payload is a collision.
fn idempotency_record_matches(
    rec: &QuoteRecord,
    instrument: Option<&celnet_proto::Instrument>,
    conventions: Option<&celnet_proto::Conventions>,
) -> bool {
    instrument == Some(&rec.instrument) && conventions == rec.quote.conventions.as_ref()
}

/// The status returned when an idempotency key is reused for a different request
/// payload (a key-collision) — `invalid_argument`, so the client learns the key
/// is ambiguous rather than silently receiving a stale, mismatched quote.
fn idempotency_conflict(key: &str) -> Status {
    Status::invalid_argument(format!(
        "idempotency key {key:?} was already used for a different request \
         (instrument/conventions mismatch); reuse a key only for an identical retry"
    ))
}

/// The status returned when an `AcceptQuote` carries an idempotency key that does
/// not match the key the quote was minted under — `invalid_argument`. The accept
/// must be request-matched to the originating key; a `quote_id` alone is not an
/// authority token, so a party that merely learns the id cannot accept another
/// requester's quote. The key value is never echoed (avoid leaking the secret).
fn accept_key_mismatch(quote_id: u64) -> Status {
    Status::invalid_argument(format!(
        "accept for quote {quote_id} did not carry the originating idempotency key; \
         an accept must be request-matched to the key the quote was issued under"
    ))
}

/// A stable, log-safe label for the caller that requested a quote: an
/// authenticated user's email, else `anonymous` (the permissive demo path).
/// **Never** logs a session token.
fn requester_label(caller: &ResolvedCaller) -> String {
    caller
        .user()
        .map(|u| u.email.clone())
        .unwrap_or_else(|| "anonymous".to_owned())
}

/// A cheap, human-readable one-line instrument summary for the structured quote
/// log — asset class + symbol + product family + tenor (+ the vanilla
/// strike/type for the common FX case).
///
/// Built at the request EDGE where allocation is fine; it is never constructed on
/// the pinned zero-alloc pricing hot core.
fn summarize_instrument(instrument: Option<&celnet_proto::Instrument>) -> String {
    let Some(inst) = instrument else {
        return "<none>".to_owned();
    };
    let (asset, symbol) = underlying_label(inst.underlying.as_ref());
    let product = inst
        .product
        .as_ref()
        .map_or("<none>", crate::pricer::product_name);
    let tenor = format!("{:.4}y", inst.expiry_years);
    let detail = vanilla_detail(inst.product.as_ref());
    format!("{asset}:{symbol} {product} {tenor}{detail}")
}

/// The asset-class tag + symbol of an instrument's underlying: rendered fully for
/// the first-class FX arm, and coarsely (asset tag only) for the cross-asset arms
/// (whose exact symbol is not needed to make a quote log investigable).
fn underlying_label(underlying: Option<&celnet_proto::Underlying>) -> (&'static str, String) {
    let Some(u) = underlying else {
        return ("none", String::new());
    };
    if let Some(p) = u.as_fx() {
        ("fx", format!("{}{}", p.base, p.quote))
    } else if u.as_metal().is_some() {
        ("metal", String::new())
    } else if u.as_equity().is_some() {
        ("equity", String::new())
    } else if u.as_commodity().is_some() {
        ("commodity", String::new())
    } else if u.as_digital_asset().is_some() {
        ("crypto", String::new())
    } else {
        ("?", String::new())
    }
}

/// The ` type@strike` detail for a plain vanilla (the common FX case); empty for
/// every other product family (whose shape the `product` family label already
/// conveys). A delta-specified strike is not resolved here (that is the pricer's
/// job) — the family label plus the `call`/`put` suffix keep the summary cheap.
fn vanilla_detail(product: Option<&celnet_proto::instrument::Product>) -> String {
    use celnet_proto::instrument::Product;
    let Some(Product::Vanilla(v)) = product else {
        return String::new();
    };
    let opt = match celnet_proto::OptionType::try_from(v.option_type) {
        Ok(celnet_proto::OptionType::Call) => "call",
        Ok(celnet_proto::OptionType::Put) => "put",
        _ => "?",
    };
    match v.strike.as_ref().and_then(|s| s.spec.as_ref()) {
        Some(celnet_proto::strike_or_delta::Spec::Strike(k)) => format!(" {opt}@{k}"),
        _ => format!(" {opt}"),
    }
}

/// Build the BUY-side FX-vanilla pre-trade template (ADR-0016 A1) a quote would book —
/// the same risk leaf the RFS click-to-trade sink records: the pair + option + the
/// marked [`VanillaInputs`](celnet_types::VanillaInputs) (the resolved strike + vol at
/// the priced market) + the quoted conventions, at the instrument's base-currency
/// notional (a quote-ccy notional converts to base at spot). `None` for a non-vanilla /
/// non-FX / zero-notional instrument — which carries no canonical-vanilla risk leaf and
/// so is never limit-gated (honest scope; guardrail #2 — never a faked vanilla leaf).
fn fx_pre_trade_template(
    instrument: &celnet_proto::Instrument,
    market: &MarketContext,
    conv: &ConventionSet,
    priced: &crate::pricer::Priced,
    surface_version: u64,
) -> Option<BookedPosition> {
    use celnet_proto::instrument::Product;

    let Some(Product::Vanilla(vanilla)) = instrument.product.as_ref() else {
        return None;
    };
    let option = celnet_types::OptionType::from(
        celnet_proto::OptionType::try_from(vanilla.option_type).ok()?,
    );
    let wire_underlying = instrument.underlying.as_ref()?;
    let pair = celnet_proto::convert::validate_fx_underlying(wire_underlying)
        .ok()?
        .as_fx()?;
    let inputs = celnet_types::VanillaInputs::new(
        market.spot,
        priced.resolved_strike,
        priced.vol,
        instrument.expiry_years,
        market.r_dom(),
        market.r_for(),
    );
    // The absolute base-currency notional (a quote-ccy notional converts to base at
    // spot); the traded side signs it at accept time.
    let abs_base = instrument.quantity.as_ref().map_or(0.0, |q| {
        if q.base_ccy {
            q.notional
        } else if market.spot != 0.0 {
            q.notional / market.spot
        } else {
            0.0
        }
    });
    if abs_base == 0.0 {
        return None;
    }
    Some(BookedPosition {
        position_id: 0,
        pair,
        option,
        notional_base: abs_base.abs(),
        inputs,
        quoted_delta: conv.delta,
        premium_style: conv.premium,
        surface_version,
    })
}

/// Build the BUY-side **cross-asset** pre-trade risk leaf (ADR-0021 uniform-asset-class)
/// a quote would book — the equity / commodity / linear digital-asset analogue of
/// [`fx_pre_trade_template`]. It re-expresses the priced position as a normalized
/// cost-of-carry [`PositionRisk`] over the SAME market carry and resolved strike/vol the
/// pricer used ([`cost_of_carry`] — the single source, so the canonicalized leaf's
/// greeks match the quoted greeks), at the instrument's base-leg notional. The
/// [`QuoteService::accept_quote`] gate then re-canonicalizes it through the position's
/// own asset-class leaf, so a cross-asset quote charges its real class-correct exposure
/// against the shared limit tree — not a silent zero.
///
/// `None` (an explicit no-gate outcome, never a fabricated exposure — guardrail #2) for:
/// * a non-vanilla product (the leaf pricers price vanilla; an exotic cross-asset leaf
///   is out of canonical scope, exactly as for FX);
/// * an FX / metal underlying (those stay on the FX path via [`fx_pre_trade_template`]);
/// * the **inverse** (coin-margined `1/S_T`) crypto arm — its additive-seam numeraire
///   collapse is a deliberately deferred follow-up in `celnet-risk-normalize` (its
///   `CryptoLeaf` is linear-only), so routing it through the linear leaf would mis-state
///   the settlement leg; the LINEAR crypto arm is fully covered;
/// * a zero-notional / non-priceable-carry instrument.
fn cross_asset_pre_trade_leaf(
    instrument: &celnet_proto::Instrument,
    market: &MarketContext,
    priced: &Priced,
) -> Option<PositionRisk> {
    use celnet_proto::instrument::Product;

    let Some(Product::Vanilla(vanilla)) = instrument.product.as_ref() else {
        return None;
    };
    let wire_underlying = instrument.underlying.as_ref()?;
    // Decode the wire underlying to its domain form and keep ONLY the cross-asset arms
    // the pricer routes through the cost-of-carry leaves; FX / metal are handled by
    // `fx_pre_trade_template` (the SAME `is_cross_asset` split the pricing dispatch uses).
    let underlying = celnet_types::Underlying::try_from(wire_underlying.clone()).ok()?;
    if !is_cross_asset(&underlying) {
        return None;
    }
    // Inverse crypto: an explicit `None` leaf (see the doc — the deferred inverse seam),
    // never a linear-leaf mis-statement. Decoded through the pricer's own settlement map.
    if matches!(underlying, celnet_types::Underlying::DigitalAsset(_))
        && !matches!(
            decode_settlement_style(instrument.settlement_style),
            Ok(CryptoSettlementStyle::Linear)
        )
    {
        return None;
    }
    let option = celnet_types::OptionType::from(
        celnet_proto::OptionType::try_from(vanilla.option_type).ok()?,
    );
    // The net cost-of-carry the position was PRICED under — read from the SAME market arm
    // through the SAME arithmetic the pricer uses, so the canonical leaf matches the quote.
    let carry = cost_of_carry(market).ok()?;
    let inputs = CarryInputs::new(
        market.spot,
        priced.resolved_strike,
        priced.vol,
        instrument.expiry_years,
        underlying.clone(),
        carry,
    );
    // The absolute base-leg notional (a quote-ccy notional converts to base at spot); the
    // traded side signs it at accept time — mirrors the FX template exactly.
    let abs_base = instrument.quantity.as_ref().map_or(0.0, |q| {
        if q.base_ccy {
            q.notional
        } else if market.spot != 0.0 {
            q.notional / market.spot
        } else {
            0.0
        }
    });
    if abs_base == 0.0 {
        return None;
    }
    Some(PositionRisk::carry(
        underlying,
        option,
        abs_base.abs(),
        inputs,
    ))
}

/// Capture the **class-correct** pre-trade risk leaf a quote would book (ADR-0016 A1,
/// generalized under ADR-0021), dispatching on the instrument's asset class: FX vanilla
/// → the canonical-vanilla [`BookedPosition`] (unchanged); a cross-asset
/// (equity/commodity/linear-crypto) vanilla → its cost-of-carry [`PositionRisk`]. The
/// two share the one unified aggregate at the gate. `None` when no canonical leaf is
/// derivable (an explicit no-gate outcome, never a fabricated zero).
fn pre_trade_leaf(
    instrument: &celnet_proto::Instrument,
    market: &MarketContext,
    conv: &ConventionSet,
    priced: &Priced,
    surface_version: u64,
) -> Option<PreTradeLeaf> {
    // FX vanilla first (the byte-identical canonical-vanilla path); its `None` covers
    // every non-FX / non-vanilla / delta-key / zero-notional FX case, and the cross-asset
    // builder's own `is_cross_asset` guard makes the fall-through disjoint (FX/metal are
    // never cross-asset), so the two arms never both fire.
    if let Some(booked) = fx_pre_trade_template(instrument, market, conv, priced, surface_version) {
        return Some(PreTradeLeaf::FxVanilla(booked));
    }
    cross_asset_pre_trade_leaf(instrument, market, priced).map(PreTradeLeaf::CrossAsset)
}

/// The aggregated-book instrument key an RFQ instrument resolves to — the SAME opaque id
/// scheme LP feeds push and book scopes list: a commodity or equity underlying's symbol
/// ticker (an ISIN/CUSIP-style code — the id an aggregated book keys a composite line on),
/// or an FX pair's market form `BASEQUOTE`. `None` when the underlying carries no such key
/// (or an empty ticker).
///
/// An instrument that yields no key never resolves to a book and prices through the
/// options engine as before; and even for a key that IS produced, the RFQ path only
/// diverges when an admin has actually configured a book whose scope admits it AND that
/// book has a live composite — so FX stays byte-identical unless an admin explicitly
/// stands up a book over the pair (Phase 2b).
fn book_instrument_key(instrument: &celnet_proto::Instrument) -> Option<String> {
    let underlying = instrument.underlying.as_ref()?;
    if let Some(pair) = underlying.as_fx() {
        return Some(format!("{}{}", pair.base, pair.quote));
    }
    if let Some(commodity) = underlying.as_commodity() {
        return commodity
            .symbol
            .as_ref()
            .map(|s| s.ticker.clone())
            .filter(|t| !t.is_empty());
    }
    if let Some(equity) = underlying.as_equity() {
        return equity
            .symbol
            .as_ref()
            .map(|s| s.ticker.clone())
            .filter(|t| !t.is_empty());
    }
    None
}

/// The priced ingredients of a `RequestQuote`, produced either from the options engine
/// (the default path) or from an admin-defined aggregated book's already-tiered composite
/// (Phase 2b). Lets [`QuoteService::request_quote`] build the stored [`Quote`] from ONE
/// place regardless of the pricing source; the book path carries no option greeks /
/// vol-surface / canonical option leaf, so those fields are `None`.
struct QuotePricing {
    two_way: TwoWayPrice,
    greeks: Option<celnet_proto::Greeks>,
    resolved_strike: f64,
    echo_version: Option<u64>,
    price_std_error: Option<f64>,
    pre_trade: Option<PreTradeLeaf>,
}

#[tonic::async_trait]
impl QuoteService for QuoteEdge {
    async fn request_quote(
        &self,
        request: Request<QuoteRequest>,
    ) -> Result<Response<Quote>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        // Caller gate (item B §2): resolve the caller from the unary body's
        // `session_token` + asserted `principal` and authorize under the live access
        // mode BEFORE the distributed-forward branch, so a forwarding edge is gated
        // too. The resolved caller is also the requester bound onto the stored quote
        // (so a later accept must come from the same authenticated principal).
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "QuoteService/RequestQuote",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        let requester = caller_binding(&caller);

        // Structured quote-request edge event (allocation-OK edge; never the hot
        // core). Records WHO asked, the idempotency key, and a cheap instrument
        // summary so LOGIN→QUOTE activity is investigable. No token is logged.
        let requester_disp = requester_label(&caller);
        let instrument_summary = summarize_instrument(req.instrument.as_ref());
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            requester = %requester_disp,
            idempotency_key = %req.idempotency_key,
            instrument = %instrument_summary,
            "quote requested"
        );

        // Distributed: forward RequestQuote to the backend that owns the instrument's
        // pair, record `quote_id → issuing backend` so the matching accept/reject
        // (which carry no pair) route back to the same backend, and return the
        // backend's `Quote` verbatim.
        if let Serve::Forward(fleet) = serve_mode(self.fleet.as_ref()) {
            let instrument = req
                .instrument
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("missing `instrument`"))?;
            let pair = route_underlying(instrument.underlying.as_ref())?;
            let (replica, client) = fleet.owner_of_pair(pair)?;
            let mut svc =
                celnet_proto::quote_service_client::QuoteServiceClient::new(client.channel());
            let quote = svc.request_quote(req).await?.into_inner();
            // Remember which backend issued this quote_id so accept/reject route home.
            self.quote_owners
                .lock()
                .await
                .insert(quote.quote_id, replica);
            return Ok(Response::new(quote));
        }

        // Idempotency: a key already seen returns the stored quote verbatim — but
        // only when the request payload (instrument + conventions) matches the one
        // the key was minted for. Reusing a key for a *different* instrument is a
        // client error (a key-collision), answered with `invalid_argument` rather
        // than the mismatched stale quote.
        if !req.idempotency_key.is_empty() {
            let store = self.store.lock().await;
            if let Some(&qid) = store.by_key.get(&req.idempotency_key)
                && let Some(rec) = store.by_id.get(&qid)
            {
                if idempotency_record_matches(
                    rec,
                    req.instrument.as_ref(),
                    req.conventions.as_ref(),
                ) {
                    let (bid, offer) = rec
                        .quote
                        .price
                        .as_ref()
                        .map_or((f64::NAN, f64::NAN), |p| (p.bid, p.offer));
                    tracing::info!(
                        class = celnet_observability::LogClass::Security.label(),
                        requester = %requester_disp,
                        idempotency_key = %req.idempotency_key,
                        quote_id = rec.quote.quote_id,
                        bid,
                        offer,
                        replay = true,
                        "quote returned"
                    );
                    return Ok(Response::new(rec.quote.clone()));
                }
                return Err(idempotency_conflict(&req.idempotency_key));
            }
        }

        let instrument = req
            .instrument
            .ok_or_else(|| Status::invalid_argument("missing `instrument`"))?;
        let wire_conv = req
            .conventions
            .ok_or_else(|| Status::invalid_argument("missing `conventions`"))?;

        // Aggregated-book RFQ: if the instrument falls in an admin-defined book's scope
        // and that book has a live composite, price the single-dealer quote against the
        // book composite's best-bid/offer. For a caller in a **pricing group**, that is
        // the group's RFS/RFQ feature pipeline run over the book's RAW composite (its own
        // outbound price off the same raw liquidity); otherwise it is the book's
        // already-tiered composite (the book's tiering — reused, never re-implemented
        // here). A book composite is a consolidated
        // market price, not an option premium, so it carries no greeks / vol-surface /
        // canonical option leaf: greeks, std-error, surface echo and the pre-trade leaf
        // are `None` (an explicit no-gate outcome, never a fabricated zero). Otherwise —
        // no book covers it — fall through to the options-engine path, byte-identical to
        // before, so nothing regresses when no book is configured.
        let pricing = if let Some(comp) = self.book_composite_for_caller(&instrument, &caller) {
            tracing::info!(
                class = celnet_observability::LogClass::Security.label(),
                requester = %requester_disp,
                idempotency_key = %req.idempotency_key,
                book = %comp.book_id,
                bid = comp.best_bid,
                offer = comp.best_offer,
                "quote sourced from aggregated book"
            );
            QuotePricing {
                two_way: TwoWayPrice {
                    bid: comp.best_bid,
                    offer: comp.best_offer,
                },
                greeks: None,
                resolved_strike: 0.0,
                echo_version: None,
                price_std_error: None,
                pre_trade: None,
            }
        } else {
            let conv = ConventionSet::decode(&wire_conv).map_err(|e| price_error_to_status(&e))?;

            let market = self.live_market().await?;
            // Resolve the optional pinned `surface_version`: an honoured pin prices the
            // quote against the marked surface and is echoed on the `Quote`; an unknown
            // pinned version is refused (the pin cannot be honoured).
            let PinnedVol {
                market: effective_market,
                echo_version,
            } = resolve_pinned_vol(
                &self.surface_book,
                req.surface_version,
                &instrument,
                &market,
            )?;
            let priced = match price_instrument(&instrument, &effective_market, &conv) {
                Ok(priced) => priced,
                Err(e) => {
                    tracing::warn!(
                        class = celnet_observability::LogClass::Security.label(),
                        requester = %requester_disp,
                        idempotency_key = %req.idempotency_key,
                        reason = %e,
                        "quote rejected"
                    );
                    return Err(price_error_to_status(&e));
                }
            };

            let two_way = self.spread.two_way(priced.greeks.price, &priced.greeks);

            // ADR-0016 A1 (generalized under ADR-0021): capture the CLASS-CORRECT
            // pre-trade risk leaf this quote would book (the BUY-side exposure), so a
            // later `AcceptQuote` can run the pre-trade limit gate against the shared
            // position book without re-pricing. FX vanilla → the canonical-vanilla leaf
            // (unchanged); cross-asset (equity/commodity/linear-crypto) → its
            // cost-of-carry leaf — each routed through the SAME unified aggregate. An
            // instrument with no derivable canonical leaf carries `None` (an explicit
            // no-gate outcome, never a fake zero).
            let pre_trade = pre_trade_leaf(
                &instrument,
                &effective_market,
                &conv,
                &priced,
                echo_version.unwrap_or(0),
            );

            QuotePricing {
                two_way,
                // MC standard error for an MC-priced product (clamped cliquet); `None`
                // for closed-form products. Surfaced on the Quote so the WS/SDK quote path
                // discloses the same uncertainty the gRPC PriceResponse does.
                price_std_error: priced.std_error,
                greeks: Some(priced.greeks.into()),
                resolved_strike: priced.resolved_strike,
                echo_version,
                pre_trade,
            }
        };

        let now = self.clock.now_nanos();
        let quote_id = self.mint_quote_id();
        let quote = Quote {
            quote_id,
            idempotency_key: req.idempotency_key.clone(),
            price: Some(pricing.two_way),
            greeks: pricing.greeks,
            conventions: Some(wire_conv),
            resolved_strike: pricing.resolved_strike,
            epoch_nanos: now,
            valid_until_nanos: now + QUOTE_VALIDITY_NANOS,
            correlation_id: req.correlation_id,
            surface_version: pricing.echo_version,
            // Stamp the who's-trading chain: the maker auto-pricer is the
            // `quoted_by` (the engine priced and showed this line); the client's
            // requesting seat, when supplied, becomes the `held_by`. So the flow is
            // attributable request→quote→booking and never anonymous.
            attribution: Some(super::attribution::resolve(req.attribution.as_ref())),
            price_std_error: pricing.price_std_error,
        };
        let pre_trade = pricing.pre_trade;

        // Store under both keys (id always; idempotency key when present).
        {
            let mut store = self.store.lock().await;
            // Re-check the key under the write lock to win a concurrent first-quote
            // race: if another caller stored the same key meanwhile, return theirs
            // (or reject a colliding key reused for a different request payload).
            if !req.idempotency_key.is_empty()
                && let Some(&qid) = store.by_key.get(&req.idempotency_key)
                && let Some(rec) = store.by_id.get(&qid)
            {
                if idempotency_record_matches(rec, Some(&instrument), Some(&wire_conv)) {
                    return Ok(Response::new(rec.quote.clone()));
                }
                return Err(idempotency_conflict(&req.idempotency_key));
            }
            store.by_id.insert(
                quote_id,
                QuoteRecord {
                    quote: quote.clone(),
                    instrument,
                    execution: None,
                    rejected: false,
                    dealers: Vec::new(),
                    booked_lp_id: String::new(),
                    // Bind the requesting caller (item B §2): a later accept by a
                    // different authenticated principal is refused.
                    requester,
                    pre_trade,
                },
            );
            if !req.idempotency_key.is_empty() {
                store.by_key.insert(req.idempotency_key.clone(), quote_id);
            }
        }

        let (bid, offer) = quote
            .price
            .as_ref()
            .map_or((f64::NAN, f64::NAN), |p| (p.bid, p.offer));
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            requester = %requester_disp,
            idempotency_key = %req.idempotency_key,
            quote_id = quote.quote_id,
            bid,
            offer,
            "quote returned"
        );
        Ok(Response::new(quote))
    }

    async fn request_multi_dealer_quote(
        &self,
        request: Request<QuoteRequest>,
    ) -> Result<Response<MultiDealerQuote>, Status> {
        // Multi-dealer (RFQ-to-many) aggregation through the real `celnet-rfq`
        // `MultiDealerEngine`: this edge's auto-pricer is the native in-process LP
        // (so a market always exists), fanned concurrently with the configured
        // deterministic synthetic dealer panel ([`LpPanelConfig`]); the two-sided
        // responses are ranked (best-bid / best-offer, deterministic tie-break,
        // timeout/last-look) into the audited panel, and the ranked rows are
        // **pinned** onto the stored quote record so a later `AcceptQuote`
        // carrying any row's `lp_id` books exactly that dealer's line. The honest
        // boundary holds: live WAN LP endpoints are ENV-selected and absent
        // in-process — the in-repo panel is the native dealer plus labeled
        // synthetic demo/test dealers quoting around the same edge mid, real
        // ranked quotes, never faked fills or placeholder rows.
        let req = request.into_inner();
        let correlation_id = req.correlation_id;
        // Caller gate (item B §2): authorize the multi-dealer RFQ under the live
        // access mode. The inner `request_quote` re-resolves + re-gates the same
        // caller (idempotent) and records the requester binding; gating here too
        // keeps every one of the four RFQ RPCs gated at its own entry, audited under
        // this RPC's name.
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "QuoteService/RequestMultiDealerQuote",
            RequiredAuthority::ReadAny,
            correlation_id,
        )?;
        // Phase 2b: if the RFQ's instrument is covered by an admin-defined aggregated
        // book, the panel is the book's real contributing member LPs (ranked beside the
        // native line) instead of the synthetic demo dealers. Resolved from `req` before
        // it is consumed by the inner `request_quote` — which itself prices the native
        // line off the SAME book's already-tiered composite (single source of truth).
        let book = req
            .instrument
            .as_ref()
            .and_then(|instrument| self.book_composite(instrument));
        // Reuse the full single-dealer pricing/idempotency/pinning path verbatim, so
        // the native dealer line is byte-identical to the `RequestQuote` it mirrors
        // and the quote is stored (keyed by `quote_id`) for accept/reject.
        let quote = self.request_quote(Request::new(req)).await?.into_inner();

        let lp_id = super::attribution::MAKER_AUTO_PRICER_ID.to_owned();
        let two_way = quote.price.unwrap_or(TwoWayPrice {
            bid: 0.0,
            offer: 0.0,
        });
        // The native source quotes the same two-way (mid ± half-spread) the maker
        // already priced; the engine's ranking algebra then selects the touch.
        let mid = (two_way.bid + two_way.offer) / 2.0;
        let half_spread = (two_way.offer - two_way.bid) / 2.0;
        // Map the quote's epoch / last-look window onto the panel source's clock
        // (the engine ranks against `now_nanos`); both are the same logical clock.
        let epoch = u64::try_from(quote.epoch_nanos).unwrap_or(0);
        let valid_for =
            u64::try_from((quote.valid_until_nanos - quote.epoch_nanos).max(0)).unwrap_or(0);

        let mut sources: Vec<Box<dyn celnet_rfq::QuoteSource>> =
            vec![Box::new(celnet_rfq::InternalPricerSource::new(
                lp_id.clone(),
                mid,
                half_spread,
                epoch,
                valid_for,
            ))];
        // The extra dealer lines join only on a serving (in-process) edge (in a
        // distributed topology the accept routes back to the issuing backend, which owns
        // the booking store, so a forwarding edge keeps the panel native-only). When an
        // aggregated book covers the instrument (Phase 2b) the panel is the book's REAL
        // contributing member LPs — one deterministic in-process source per FRESH member
        // line (each LP's own raw two-way) — ranked beside the native tiered line. A
        // stale / crossed / non-finite member is excluded (it does not set a live price).
        // Otherwise the synthetic demo/test dealers join, each a deterministic quoter over
        // the SAME edge mid with its fixed per-dealer spread/skew offsets — exactly as
        // before, so a no-book edge is byte-identical.
        if !matches!(serve_mode(self.fleet.as_ref()), Serve::Forward(_)) {
            if let Some(comp) = &book {
                for member in &comp.members {
                    if member.stale
                        || !(member.bid.is_finite()
                            && member.offer.is_finite()
                            && member.bid <= member.offer)
                    {
                        continue;
                    }
                    let member_mid = 0.5 * (member.bid + member.offer);
                    let member_half = 0.5 * (member.offer - member.bid);
                    sources.push(Box::new(celnet_rfq::InternalPricerSource::new(
                        member.lp_name.clone(),
                        member_mid,
                        member_half,
                        epoch,
                        valid_for,
                    )));
                }
            } else {
                for k in 1..=self.panel.synthetic_lps {
                    let (synth_mid, synth_half) = synthetic_lp_two_way(k, mid, half_spread);
                    sources.push(Box::new(celnet_rfq::InternalPricerSource::new(
                        synthetic_lp_id(k),
                        synth_mid,
                        synth_half,
                        epoch,
                        valid_for,
                    )));
                }
            }
        }
        let engine = celnet_rfq::MultiDealerEngine::new(sources);
        // The RFQ descriptor is the aggregation key (audit correlation); the price
        // is the source's job (already injected via the native source's mid).
        let rfq = celnet_rfq::RfqRequest::new(
            quote.quote_id.to_string(),
            celnet_types::CcyPair::new(celnet_types::Ccy::EUR, celnet_types::Ccy::USD),
            celnet_types::OptionType::Call,
            quote.resolved_strike,
            celnet_types::Tenor::Overnight,
        );
        let now = u64::try_from(self.clock.now_nanos()).unwrap_or(0);
        let ranked = engine
            .request(&rfq, RFQ_PANEL_DEADLINE, now)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Project the ranked panel rows onto wire `DealerQuote`s. The native row carries
        // the edge-priced greeks / MC std-error (the pricing belongs to the maker; the
        // engine owns only the aggregation), while a non-native dealer row — a synthetic
        // demo dealer or a real aggregated-book member LP — carries only its quoted price
        // and its own auto-pricer attribution (an LP discloses a price, not its greeks).
        let held_by = quote.attribution.as_ref().and_then(|a| a.held_by.clone());
        let dealers: Vec<DealerQuote> = ranked
            .rows
            .iter()
            .map(|row| {
                let is_native = row.lp_id == lp_id;
                DealerQuote {
                    lp_id: row.lp_id.clone(),
                    price: Some(row.price.to_wire()),
                    greeks: if is_native { quote.greeks } else { None },
                    resolved_strike: quote.resolved_strike,
                    valid_until_nanos: i64::try_from(row.valid_until_nanos).unwrap_or(i64::MAX),
                    attribution: if is_native {
                        quote.attribution.clone()
                    } else {
                        Some(lp_line_attribution(&row.lp_id, held_by.clone()))
                    },
                    price_std_error: if is_native {
                        quote.price_std_error
                    } else {
                        None
                    },
                }
            })
            .collect();

        // Pin the issued panel rows onto the stored quote record so an accept
        // naming any `lp_id` books exactly the line the client was shown. The
        // sources are deterministic over the stored quote, so an idempotent
        // re-request re-pins identical rows. (A forwarding edge stores no local
        // record — the issuing backend owns the booking — so this is a no-op
        // there, matching the accept routing.)
        {
            let mut store = self.store.lock().await;
            if let Some(rec) = store.by_id.get_mut(&quote.quote_id) {
                rec.dealers = dealers.clone();
            }
        }

        let multi = MultiDealerQuote {
            quote_id: quote.quote_id,
            idempotency_key: quote.idempotency_key,
            dealers,
            best_bid_lp_id: ranked.lp_won_bid.unwrap_or_default(),
            best_offer_lp_id: ranked.lp_won_offer.unwrap_or_default(),
            conventions: quote.conventions,
            epoch_nanos: quote.epoch_nanos,
            correlation_id,
            surface_version: quote.surface_version,
        };
        // Panel RESULT edge event: the inner `request_quote` already logged the
        // "quote requested"/"quote returned" pair for the native line; this records
        // the aggregated panel outcome (breadth + the ranked touch winners).
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            quote_id = multi.quote_id,
            dealers = multi.dealers.len(),
            best_bid_lp = %multi.best_bid_lp_id,
            best_offer_lp = %multi.best_offer_lp_id,
            "multi-dealer panel returned"
        );
        Ok(Response::new(multi))
    }

    async fn request_rates_quote(
        &self,
        request: Request<RatesQuoteRequest>,
    ) -> Result<Response<RatesQuote>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        // A pure fixed-income price-discovery calculation — the caller supplies the
        // whole market (`curve_set`), exactly as `PricingService.PriceRates` does,
        // so it reads no live market, stores nothing and books nothing (every
        // replica computes the identical quote). This is the taker's two-way,
        // mirroring the FIX venue auto-quote (`services::fix::on_rates_quote_request`);
        // the stateful maker-side FI booking flow that binds a caller/requester is
        // `RfqDeskService`, so — like `PriceRates` — this calculation RPC needs no
        // per-caller gate beyond the readiness admission above.
        let curve_set = req
            .curve_set
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing `curve_set`"))?;
        let instrument = req
            .instrument
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing `instrument`"))?;
        if !(req.notional.is_finite() && req.notional > 0.0) {
            return Err(Status::invalid_argument(
                "`notional` must be a positive, finite RFQ size",
            ));
        }
        let side = Side::try_from(req.side)
            .map_err(|_| Status::invalid_argument("unknown `side` value"))?;

        // The one FI pricing path: the two-way is struck around the side-independent
        // fair level (par rate for OIS/IRS/FRA, clean price for a cash bond), and
        // `result` is the full `price_rates` risk — the SAME engine numbers the
        // outright `PriceRates` RPC returns (no second pricing implementation).
        let two_way = crate::rates_pricing::quote_rates_two_way(curve_set, instrument, side)
            .map_err(|e| rates_price_error_to_status(&e))?;
        let (bid, offer) = (two_way.bid, two_way.offer);

        let now = self.clock.now_nanos();
        let quote_id = self.mint_quote_id();
        let quote = RatesQuote {
            quote_id,
            idempotency_key: req.idempotency_key.clone(),
            price: Some(TwoWayPrice { bid, offer }),
            result: Some(two_way.result),
            notional: req.notional,
            epoch_nanos: now,
            valid_until_nanos: now + QUOTE_VALIDITY_NANOS,
            correlation_id: req.correlation_id,
        };

        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            idempotency_key = %req.idempotency_key,
            quote_id = quote.quote_id,
            bid,
            offer,
            "rates quote returned"
        );
        Ok(Response::new(quote))
    }

    async fn accept_quote(
        &self,
        request: Request<QuoteAccept>,
    ) -> Result<Response<Execution>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let acc = request.into_inner();

        // Caller gate (item B §2): resolve + authorize the accepting caller under the
        // live access mode BEFORE the distributed-forward branch (`QuoteAccept` has
        // no `correlation_id` field ⇒ `None`). The resolved caller is then bound
        // against the quote's recorded requester below.
        let caller = resolve_caller(
            &self.sessions,
            acc.session_token.as_deref(),
            acc.principal.clone(),
        )?;
        // Accepting a quote books an execution — the strongest write on the FX
        // path — so it requires the Execute·FxOptions capability, not mere read
        // access. The capability gate resolves an authenticated session (a body
        // principal cannot self-grant; finding #3): under Enforce a caller without
        // Execute is denied, while logged-in GUI/Excel users (who inject their
        // session token on every unary) keep trading. Mirrors the FI accept/book
        // gates (RfqDeskService/AcceptDeskQuote, RiskService/BookRatesPosition).
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "QuoteService/AcceptQuote",
            RequiredAuthority::Capability(Action::Execute, AssetClass::FxOptions),
            None,
        )?;

        // Distributed: route the accept to the SAME backend that issued the quote
        // (recorded on the forwarded RequestQuote). The accept/reject wire carries
        // only `quote_id` (no pair), and a `quote_id` is minted under the issuing
        // backend's own secret, so it must go home. An unknown id ⇒ not_found
        // (this edge never saw the originating RequestQuote).
        if let Serve::Forward(fleet) = serve_mode(self.fleet.as_ref()) {
            let replica = self
                .quote_owners
                .lock()
                .await
                .get(&acc.quote_id)
                .copied()
                .ok_or_else(|| {
                    Status::not_found(format!(
                        "unknown quote_id {} (not issued through this edge)",
                        acc.quote_id
                    ))
                })?;
            let client = fleet.client_for(replica)?;
            let mut svc =
                celnet_proto::quote_service_client::QuoteServiceClient::new(client.channel());
            return Ok(Response::new(svc.accept_quote(acc).await?.into_inner()));
        }

        let mut store = self.store.lock().await;
        let rec = store
            .by_id
            .get(&acc.quote_id)
            .ok_or_else(|| Status::not_found(format!("unknown quote_id {}", acc.quote_id)))?
            .clone();

        // Requester binding (item B §2): when the quote was requested by an
        // AUTHENTICATED caller, an accept is admitted only from that SAME
        // authenticated principal — a different authenticated user is refused
        // `permission_denied`, beyond the idempotency-key match below. This closes
        // "any party that learns a quote_id books another requester's quote". An
        // ANONYMOUS-requested quote (the permissive demo path) carries no binding,
        // so the legacy idempotency-only behaviour is unchanged.
        if let RequesterBinding::Authenticated(requester_id) = &rec.requester
            && caller_binding(&caller) != RequesterBinding::Authenticated(requester_id.clone())
        {
            return Err(Status::permission_denied(format!(
                "accept by a different principal than the one that requested quote {}; \
                 an accept must come from the requesting caller",
                acc.quote_id
            )));
        }

        // Request-matched idempotency: an accept must carry the *same* idempotency
        // key the quote was minted under. The `quote_id` alone is not an authority
        // token; binding the accept to the originating key stops any party that
        // merely learns a `quote_id` from accepting another requester's live quote,
        // and makes an accept-retry safe only for the genuine originator. (Mirrors
        // the request-matched contract `RequestQuote` already enforces.)
        if acc.idempotency_key != rec.quote.idempotency_key {
            return Err(accept_key_mismatch(acc.quote_id));
        }

        // The dealer line this accept trades: an empty `lp_id` (or the native
        // maker's own id) is the single-dealer quote — byte-identical to a
        // single-dealer accept — so it normalizes to the maker line.
        let requested_line: &str = if acc.lp_id.is_empty() {
            super::attribution::MAKER_AUTO_PRICER_ID
        } else {
            &acc.lp_id
        };

        // Idempotent accept: a retry returns the already-booked execution — but
        // only when the retry's traded side AND dealer line match the booked
        // ones. A second accept that flips the side, or names a different dealer
        // line, is a different trade intent, not a retry, so it is refused
        // (`failed_precondition`) rather than handed a booking it did not ask
        // for. The key already matched above.
        if let Some(exec) = &rec.execution {
            let retry_side = Side::try_from(acc.side).unwrap_or(Side::Buy);
            let booked_side = Side::try_from(exec.side).unwrap_or(Side::Buy);
            let effective_retry_side = if retry_side == Side::Sell {
                Side::Sell
            } else {
                Side::Buy
            };
            if effective_retry_side != booked_side {
                return Err(Status::failed_precondition(format!(
                    "quote {} already booked on side {booked_side:?}; \
                     accept-retry requested side {effective_retry_side:?}",
                    acc.quote_id
                )));
            }
            if requested_line != rec.booked_lp_id {
                return Err(Status::failed_precondition(format!(
                    "quote {} already booked on dealer line {:?}; \
                     accept-retry requested line {requested_line:?}",
                    acc.quote_id, rec.booked_lp_id
                )));
            }
            return Ok(Response::new(exec.clone()));
        }

        // A declined (rejected) quote can no longer be accepted.
        if rec.rejected {
            return Err(Status::failed_precondition(format!(
                "quote {} was rejected; cannot accept",
                acc.quote_id
            )));
        }

        // Multi-dealer panel-line selection: in an RFQ-to-many flow the
        // `quote_id` keys the aggregate request and `lp_id` disambiguates which
        // dealer's line is being lifted/hit. The native maker line books the
        // stored single-dealer quote (byte-identical to a single-dealer accept);
        // any other `lp_id` must name a row of the panel **pinned** for this
        // `quote_id` when it was issued — the accept then books exactly the
        // price/validity/attribution the client was shown, never a re-price. A
        // line that was never on this quote's panel is not bookable on this edge,
        // so it is refused rather than silently booked against the native price.
        let panel_row: Option<&DealerQuote> =
            if requested_line == super::attribution::MAKER_AUTO_PRICER_ID {
                None
            } else {
                Some(
                    rec.dealers
                        .iter()
                        .find(|d| d.lp_id == requested_line)
                        .ok_or_else(|| {
                            Status::failed_precondition(format!(
                                "quote {} has no bookable dealer line for lp_id {:?} on this edge",
                                acc.quote_id, acc.lp_id
                            ))
                        })?,
                )
            };

        // Last-look: an accept after the traded line's validity deadline is
        // rejected as expired — the engine law (`valid_until_nanos` lapsed ⇒ the
        // line is no longer liftable), applied to the exact line being booked.
        let now = self.clock.now_nanos();
        let line_valid_until =
            panel_row.map_or(rec.quote.valid_until_nanos, |row| row.valid_until_nanos);
        if now > line_valid_until {
            return Err(Status::deadline_exceeded(format!(
                "quote {} expired at {line_valid_until} (now {now})",
                acc.quote_id
            )));
        }

        // Resolve the traded side and the lifted/hit premium of the traded line's
        // two-way: the pinned panel row's price for a dealer line, the stored
        // single-dealer quote's otherwise.
        let side = Side::try_from(acc.side).unwrap_or(Side::Buy);
        let line_price = match panel_row {
            Some(row) => row.price,
            None => rec.quote.price,
        };
        let price = line_price.unwrap_or(TwoWayPrice {
            bid: 0.0,
            offer: 0.0,
        });
        // BUY (or two-way default) lifts the offer; SELL hits the bid.
        let (traded_side, traded_premium) = match side {
            Side::Sell => (Side::Sell, price.bid),
            _ => (Side::Buy, price.offer),
        };

        // Pre-trade limit gate (ADR-0016 A1, generalized under ADR-0021): an accept books
        // the strongest write, so before pinning the execution consult the SAME limit tree
        // + current-book aggregation the RFS click-to-trade sink enforces — for whichever
        // asset class the quote carries. A hard breach refuses the accept with a typed
        // `failed_precondition` `LimitBreached` status (uniform across every booking
        // front-end — guardrail #11) and books nothing; store state is never mutated on a
        // reject. A quote with no derivable canonical leaf carries `None`, so there is
        // nothing to gate. The traded side signs the captured BUY-side notional (a `SELL`
        // accept negates it).
        if let Some(leaf) = rec.pre_trade.as_ref() {
            let result = match leaf {
                PreTradeLeaf::FxVanilla(booked) => {
                    let mut booked = *booked;
                    if traded_side == Side::Sell {
                        booked.notional_base = -booked.notional_base;
                    }
                    self.access_store.evaluate_pre_trade(&booked)?
                }
                PreTradeLeaf::CrossAsset(position) => {
                    let mut position = position.clone();
                    if traded_side == Side::Sell {
                        position.notional_base = -position.notional_base;
                    }
                    self.access_store.evaluate_pre_trade_position(&position)?
                }
            };
            if result.decision == celnet_limits::PreTradeDecision::Reject {
                return Err(limit_breached_status(&result));
            }
        }

        let execution_id = self.next_execution_id.fetch_add(1, Ordering::Relaxed);
        let execution = Execution {
            execution_id,
            quote_id: acc.quote_id,
            side: traded_side as i32,
            traded_premium,
            instrument: Some(rec.instrument.clone()),
            epoch_nanos: now,
            // Carry the traded line's attribution chain onto the booking: the
            // pinned dealer row's (so a dealer-line fill is attributed to that
            // LP), or the single-dealer quote's for the native line.
            attribution: match panel_row {
                Some(row) => row.attribution.clone(),
                None => rec.quote.attribution.clone(),
            },
        };

        // Book it back into the record (with the traded line) so a retry is
        // idempotent and line-matched.
        let booked_line = requested_line.to_owned();
        if let Some(stored) = store.by_id.get_mut(&acc.quote_id) {
            stored.execution = Some(execution.clone());
            stored.booked_lp_id = booked_line;
        }

        Ok(Response::new(execution))
    }

    async fn reject_quote(
        &self,
        request: Request<QuoteReject>,
    ) -> Result<Response<RejectAck>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let rej = request.into_inner();

        // Caller gate (item B §2): authorize the rejecting caller under the live
        // access mode BEFORE the distributed-forward branch (`QuoteReject` has no
        // `correlation_id` field ⇒ `None`). Possession of the unguessable `quote_id`
        // still authorizes WHICH quote may be declined (module trust model); this
        // gate enforces that the caller is admitted at all under Enforce.
        let caller = resolve_caller(
            &self.sessions,
            rej.session_token.as_deref(),
            rej.principal.clone(),
        )?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "QuoteService/RejectQuote",
            RequiredAuthority::ReadAny,
            None,
        )?;

        // Distributed: route the reject to the SAME backend that issued the quote
        // (recorded on the forwarded RequestQuote), for the same reason as accept.
        if let Serve::Forward(fleet) = serve_mode(self.fleet.as_ref()) {
            let replica = self
                .quote_owners
                .lock()
                .await
                .get(&rej.quote_id)
                .copied()
                .ok_or_else(|| {
                    Status::not_found(format!(
                        "unknown quote_id {} (not issued through this edge)",
                        rej.quote_id
                    ))
                })?;
            let client = fleet.client_for(replica)?;
            let mut svc =
                celnet_proto::quote_service_client::QuoteServiceClient::new(client.channel());
            return Ok(Response::new(svc.reject_quote(rej).await?.into_inner()));
        }

        let mut store = self.store.lock().await;
        // The reject is authorised by possession of the (unguessable) quote_id —
        // see the module "Trust model" doc: the id space is sparse/unenumerable, so
        // only a party that received the quote can decline it. A reject on an
        // unknown quote is a client error; a reject on a booked quote is refused (it
        // already traded).
        let rec = store
            .by_id
            .get_mut(&rej.quote_id)
            .ok_or_else(|| Status::not_found(format!("unknown quote_id {}", rej.quote_id)))?;
        if rec.execution.is_some() {
            return Err(Status::failed_precondition(format!(
                "quote {} already booked; cannot reject",
                rej.quote_id
            )));
        }
        // Mark the quote declined so it can no longer be accepted. A re-reject is
        // idempotent (the flag is already set). A reject never books a trade, so
        // the contract returns a purpose-typed acknowledgement, not an execution.
        rec.rejected = true;
        Ok(Response::new(RejectAck {
            quote_id: rej.quote_id,
            epoch_nanos: self.clock.now_nanos(),
        }))
    }
}

#[cfg(test)]
mod tests {
    //! Item B §2: the RFQ caller gate + the requester-bound accept.
    //!
    //! These build an in-process [`QuoteEdge`] over a real [`CoreLink`] (so an
    //! admitted RFQ actually prices), an injected [`SessionRegistry`] (so we mint
    //! authenticated users without an AuthService round-trip), and a shared
    //! [`PositionStore`] whose runtime access mode the gate reads.
    use super::*;
    use crate::config::identity::Role;
    use crate::config::identity::{AggregatedBookDef, AggregationParams, IdentityStore, Scope};
    use crate::services::aggregation::AggregationHub;
    use crate::services::sessions::AuthenticatedUser;
    use celnet_entitlements::AccessMode;
    use celnet_proto::LpQuote;
    use celnet_tiering::{Guardrails, SpreadUnit, StalePolicy, StrategySpec, TieringConfig};

    /// A real EURUSD-fixture core so an admitted RFQ prices a genuine quote.
    fn test_link() -> Arc<CoreLink> {
        let initial = celnet_engine::testing::make_state(
            1.10,
            celnet_conventions::resolve(
                celnet_types::CcyPair::parse("EURUSD").unwrap(),
                celnet_types::Tenor::Years(1),
            )
            .record,
        );
        CoreLink::start(initial, None)
    }

    /// A EURUSD vanilla-call wire instrument at an absolute strike, 1Y.
    fn vanilla_call(strike: f64) -> celnet_proto::Instrument {
        celnet_proto::Instrument {
            underlying: Some(celnet_proto::Underlying::fx(celnet_proto::CcyPair {
                base: "EUR".to_owned(),
                quote: "USD".to_owned(),
            })),
            tenor: Some(celnet_proto::Tenor {
                unit: celnet_proto::tenor::Unit::Years as i32,
                count: 1,
                broken_date: None,
            }),
            expiry_years: 1.0,
            quantity: Some(celnet_proto::Quantity {
                notional: 1_000_000.0,
                base_ccy: true,
            }),
            side: celnet_proto::Side::TwoWay as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(celnet_proto::instrument::Product::Vanilla(
                celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(celnet_proto::StrikeOrDelta {
                        spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                    }),
                },
            )),
            ..Default::default()
        }
    }

    fn wire_conventions() -> celnet_proto::Conventions {
        celnet_proto::Conventions {
            delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
            atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
            premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
            cut: celnet_proto::Cut::NewYork1000 as i32,
            day_count: celnet_proto::DayCount::Act365Fixed as i32,
            settlement: celnet_proto::Settlement::Deliverable as i32,
        }
    }

    /// The grant-all wire principal every client asserts by default.
    fn grant_all() -> celnet_proto::EntitlementPrincipal {
        celnet_proto::EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![],
        }
    }

    /// A ready in-process quote edge under `mode`, returning the edge + the shared
    /// session registry (to mint users) + the shared access store.
    fn edge_under(mode: AccessMode) -> (QuoteEdge, Arc<SessionRegistry>, Arc<PositionStore>) {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let clock = Clock::system();
        let sessions = Arc::new(SessionRegistry::new(clock.clone()));
        let access_store = Arc::new(PositionStore::new());
        access_store.set_access_mode(mode);
        let edge = QuoteEdge::new(
            test_link(),
            gate,
            SpreadModel::default(),
            clock,
            Arc::new(SurfaceBook::new()),
        )
        .with_session_access(Arc::clone(&sessions), Arc::clone(&access_store));
        (edge, sessions, access_store)
    }

    /// Mint a session token for a trader user with a stable id.
    fn token_for(sessions: &SessionRegistry, user_id: &str) -> String {
        sessions
            .issue(AuthenticatedUser {
                user_id: user_id.to_owned(),
                email: format!("{user_id}@celnet.com"),
                display_name: user_id.to_owned(),
                role: Role::Trader,
                desk_ids: vec!["g10".to_owned()],
                all_desks: false,
                role_caps: crate::config::identity::default_trader_bundle(),
                cap_grants: Vec::new(),
                cap_denies: Vec::new(),
            })
            .expect("session issues")
            .token
    }

    fn quote_request(
        key: &str,
        session_token: Option<String>,
        principal: Option<celnet_proto::EntitlementPrincipal>,
    ) -> QuoteRequest {
        quote_request_for(key, vanilla_call(1.12), session_token, principal)
    }

    /// A quote request over an explicit instrument (so the cross-asset arms can be
    /// exercised through the SAME request path as the FX vanilla).
    fn quote_request_for(
        key: &str,
        instrument: celnet_proto::Instrument,
        session_token: Option<String>,
        principal: Option<celnet_proto::EntitlementPrincipal>,
    ) -> QuoteRequest {
        QuoteRequest {
            idempotency_key: key.to_owned(),
            instrument: Some(instrument),
            conventions: Some(wire_conventions()),
            correlation_id: None,
            surface_version: None,
            attribution: None,
            session_token,
            principal,
        }
    }

    /// A BRENT/USD commodity vanilla-call wire instrument at an absolute strike, 1Y — a
    /// CROSS-ASSET arm the pricer routes through the cost-of-carry leaves (not the FX
    /// path). It prices against the edge's live FX-derived market (the FX two-rate carry
    /// arm supplies the cost-of-carry `b = r − r_for`), so an admitted RFQ produces a
    /// genuine cross-asset quote with a non-zero delta.
    fn commodity_call(strike: f64) -> celnet_proto::Instrument {
        celnet_proto::Instrument {
            underlying: Some(celnet_proto::Underlying::commodity(
                celnet_proto::CommodityRef::new(celnet_proto::Symbol::new("BRENT", ""), "USD"),
            )),
            tenor: Some(celnet_proto::Tenor {
                unit: celnet_proto::tenor::Unit::Years as i32,
                count: 1,
                broken_date: None,
            }),
            expiry_years: 1.0,
            quantity: Some(celnet_proto::Quantity {
                notional: 1_000_000.0,
                base_ccy: true,
            }),
            side: celnet_proto::Side::TwoWay as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(celnet_proto::instrument::Product::Vanilla(
                celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(celnet_proto::StrikeOrDelta {
                        spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                    }),
                },
            )),
            ..Default::default()
        }
    }

    // --- §2 caller gate -------------------------------------------------------

    /// Under `Enforce`, every one of the four RFQ RPCs denies an unauthenticated
    /// caller (no token, no principal) and admits once a valid `session_token` (or
    /// the grant-all principal) is presented. This is the headline §2 gate.
    #[tokio::test]
    async fn enforce_denies_unauthenticated_then_admits_authenticated() {
        let (edge, sessions, _store) = edge_under(AccessMode::Enforce);

        // RequestQuote: deny (deny-by-default ⇒ Unauthenticated).
        let err = edge
            .request_quote(Request::new(quote_request("k-deny", None, None)))
            .await
            .expect_err("unauthenticated RequestQuote denied under Enforce");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);

        // RequestMultiDealerQuote: deny.
        let err = edge
            .request_multi_dealer_quote(Request::new(quote_request("k-md-deny", None, None)))
            .await
            .expect_err("unauthenticated RequestMultiDealerQuote denied under Enforce");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);

        // AcceptQuote: deny (gated before the not_found lookup).
        let err = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: 12345,
                idempotency_key: "k".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect_err("unauthenticated AcceptQuote denied under Enforce");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);

        // RejectQuote: deny.
        let err = edge
            .reject_quote(Request::new(QuoteReject {
                quote_id: 12345,
                reason: "no".to_owned(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect_err("unauthenticated RejectQuote denied under Enforce");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);

        // Admit with a valid session token: the RFQ prices and returns a quote.
        let token = token_for(&sessions, "alice");
        let quote = edge
            .request_quote(Request::new(quote_request("k-ok-token", Some(token), None)))
            .await
            .expect("an authenticated RequestQuote is admitted and prices")
            .into_inner();
        assert!(quote.quote_id >= 1, "a real quote id was minted");

        // Admit with the asserted grant-all principal (no token) — the audited
        // explicit-grant default every client sends, accepted under Enforce.
        let quote2 = edge
            .request_quote(Request::new(quote_request(
                "k-ok-grant-all",
                None,
                Some(grant_all()),
            )))
            .await
            .expect("a grant-all RequestQuote is admitted under Enforce")
            .into_inner();
        assert!(quote2.quote_id >= 1);
    }

    /// Under `Permissive`, an absent caller is admitted (the audited demo path), so
    /// the RFQ prices — the legacy edge is unchanged.
    #[tokio::test]
    async fn permissive_admits_absent_caller() {
        let (edge, _sessions, _store) = edge_under(AccessMode::Permissive);
        let quote = edge
            .request_quote(Request::new(quote_request("perm", None, None)))
            .await
            .expect("permissive admits an absent caller")
            .into_inner();
        assert!(quote.quote_id >= 1);
    }

    // --- §2 requester-bound accept -------------------------------------------

    /// An AUTHENTICATED requester binds the quote: an accept by a DIFFERENT
    /// authenticated user is refused `permission_denied`; the SAME user succeeds.
    #[tokio::test]
    async fn accept_is_bound_to_the_authenticated_requester() {
        let (edge, sessions, _store) = edge_under(AccessMode::Enforce);
        let alice = token_for(&sessions, "alice");
        let bob = token_for(&sessions, "bob");

        // Alice requests a quote.
        let quote = edge
            .request_quote(Request::new(quote_request(
                "bind-key",
                Some(alice.clone()),
                None,
            )))
            .await
            .expect("Alice's RequestQuote prices")
            .into_inner();

        // Bob (a different authenticated principal) tries to accept it with the
        // SAME idempotency key — refused on the requester binding, before booking.
        let err = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "bind-key".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: Some(bob),
                principal: None,
            }))
            .await
            .expect_err("Bob must not book Alice's requested quote");
        assert_eq!(
            err.code(),
            tonic::Code::PermissionDenied,
            "a different authenticated principal is refused: {}",
            err.message()
        );

        // Alice (the requester) accepts the same quote — admitted and booked.
        let exec = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "bind-key".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: Some(alice),
                principal: None,
            }))
            .await
            .expect("Alice — the requesting caller — books her own quote")
            .into_inner();
        assert_eq!(exec.quote_id, quote.quote_id);
    }

    /// Accepting a quote books an execution, so under Enforce it requires the
    /// `Execute·FxOptions` capability — not mere read access. A grant-all *body
    /// principal* with no session is admitted for the read-side RequestQuote
    /// (ReadAny — see `enforce_denies_unauthenticated_then_admits_authenticated`),
    /// but must be DENIED here: a body principal cannot self-grant a capability
    /// (finding #3), so only an authenticated session that holds Execute may accept.
    /// The capability gate fires before the quote lookup, so the denial is
    /// `unauthenticated`, not `not_found`.
    #[tokio::test]
    async fn accept_under_enforce_requires_execute_capability_not_just_a_principal() {
        let (edge, _sessions, _store) = edge_under(AccessMode::Enforce);
        let denied = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: 1, // never looked up — the capability gate refuses first
                idempotency_key: "k-grant-all-accept".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: Some(grant_all()),
            }))
            .await
            .expect_err("a grant-all principal cannot satisfy the Execute capability");
        assert_eq!(denied.code(), tonic::Code::Unauthenticated);
    }

    /// An ANONYMOUS-requested quote (permissive, no authenticated requester) keeps
    /// the legacy idempotency-only accept behaviour: any caller carrying the right
    /// key books it (no requester-binding check), and a wrong key is refused on the
    /// key mismatch — not on a binding.
    #[tokio::test]
    async fn anonymous_requester_keeps_idempotency_only_accept() {
        let (edge, _sessions, _store) = edge_under(AccessMode::Permissive);

        // Anonymous request (no token, no principal — admitted under Permissive).
        let quote = edge
            .request_quote(Request::new(quote_request("anon-key", None, None)))
            .await
            .expect("permissive anonymous request prices")
            .into_inner();

        // A wrong key is refused on the idempotency-key mismatch (InvalidArgument),
        // NOT a binding (there is no authenticated requester to bind to).
        let err = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "wrong-key".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect_err("a mismatched key is refused");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        // The right key books it — anonymous, idempotency-only, unchanged.
        let exec = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "anon-key".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect("the right idempotency key books an anonymous-requested quote")
            .into_inner();
        assert_eq!(exec.quote_id, quote.quote_id);
    }

    /// FRONT-END 3 (`QuoteService::AcceptQuote`, ADR-0016 A1): accepting a quote whose
    /// booking would blow a hard firm-wide Delta limit is **rejected** with a
    /// `failed_precondition` `LimitBreached` status, uniform with every other FX booking
    /// front-end (guardrail #11). The check consults the same shared position book.
    #[tokio::test]
    async fn accept_quote_rejects_a_hard_limit_blown_booking() {
        let (edge, _sessions, store) = edge_under(AccessMode::Permissive);
        // A firm Delta cap of 1 base unit — the 1mm EURUSD call's delta blows it hard.
        store.set_limit(
            celnet_limits::LimitScope::Firm,
            celnet_limits::LimitSpec::hard(celnet_limits::LimitMetric::Delta, 1.0),
        );

        let quote = edge
            .request_quote(Request::new(quote_request("lim-key", None, None)))
            .await
            .expect("the quote prices")
            .into_inner();

        let err = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "lim-key".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect_err("a hard-limit-blown accept must be rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("limit breached"),
            "the reject carries the uniform LimitBreached reason, got {:?}",
            err.message()
        );
    }

    /// With no limit configured (the default) an accept books unchanged — the gate is
    /// inert until a cap is set, so the pre-gate booking path is byte-identical.
    #[tokio::test]
    async fn accept_quote_without_limits_books_unchanged() {
        let (edge, _sessions, _store) = edge_under(AccessMode::Permissive);
        let quote = edge
            .request_quote(Request::new(quote_request("ok-key", None, None)))
            .await
            .expect("the quote prices")
            .into_inner();
        let exec = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "ok-key".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect("a no-limit accept books")
            .into_inner();
        assert_eq!(exec.quote_id, quote.quote_id);
    }

    // --- ADR-0021 cross-asset pre-trade leaf (the divergence-closer) ----------

    /// ADR-0021 (uniform-asset-class): a CROSS-ASSET (commodity) quote carries its
    /// CLASS-CORRECT risk into the SAME pre-trade gate — a firm Delta cap the commodity
    /// option's own delta blows is tripped, computed from the KNOWN position through the
    /// cost-of-carry leaf. Before the generalization a non-FX quote carried NO leaf and
    /// so was a silent zero that no cap could ever gate; this proves the gate now sees
    /// the real cross-asset exposure (the headline validation).
    #[tokio::test]
    async fn accept_cross_asset_quote_rejects_a_hard_limit_blown_booking() {
        let (edge, _sessions, store) = edge_under(AccessMode::Permissive);
        // A firm Delta cap of 1 base unit — the 1mm commodity call's delta blows it hard.
        store.set_limit(
            celnet_limits::LimitScope::Firm,
            celnet_limits::LimitSpec::hard(celnet_limits::LimitMetric::Delta, 1.0),
        );

        let quote = edge
            .request_quote(Request::new(quote_request_for(
                "ca-lim",
                commodity_call(1.12),
                None,
                None,
            )))
            .await
            .expect("the cross-asset quote prices")
            .into_inner();

        let err = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "ca-lim".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect_err("a hard-limit-blown cross-asset accept must be rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("limit breached"),
            "the reject carries the uniform LimitBreached reason, got {:?}",
            err.message()
        );
    }

    /// A cross-asset quote whose booked delta sits WITHIN a generous firm cap books —
    /// proving the gate reads a finite, position-derived exposure (not an always-reject
    /// nor an always-accept), and that a no-limit cross-asset accept is byte-identical to
    /// the pre-gate path (the gate stays inert until a cap actually binds).
    #[tokio::test]
    async fn accept_cross_asset_quote_books_within_cap_and_without_limits() {
        // Within a generous cap: the commodity delta is far below 10^9 base units.
        let (edge, _sessions, store) = edge_under(AccessMode::Permissive);
        store.set_limit(
            celnet_limits::LimitScope::Firm,
            celnet_limits::LimitSpec::hard(celnet_limits::LimitMetric::Delta, 1.0e9),
        );
        let quote = edge
            .request_quote(Request::new(quote_request_for(
                "ca-ok",
                commodity_call(1.12),
                None,
                None,
            )))
            .await
            .expect("the cross-asset quote prices")
            .into_inner();
        let exec = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "ca-ok".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect("a within-cap cross-asset accept books")
            .into_inner();
        assert_eq!(exec.quote_id, quote.quote_id);

        // No limits: the cross-asset accept books unchanged (inert-gate byte-identity).
        let (edge2, _s2, _st2) = edge_under(AccessMode::Permissive);
        let quote2 = edge2
            .request_quote(Request::new(quote_request_for(
                "ca-nolim",
                commodity_call(1.12),
                None,
                None,
            )))
            .await
            .expect("the cross-asset quote prices")
            .into_inner();
        let exec2 = edge2
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote2.quote_id,
                idempotency_key: "ca-nolim".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect("a no-limit cross-asset accept books")
            .into_inner();
        assert_eq!(exec2.quote_id, quote2.quote_id);
    }

    // --- Phase 2b: RFQ priced against the admin-defined aggregated book ---------

    /// A BRENT-style commodity vanilla-call over an EXPLICIT ticker — the ticker is the
    /// aggregated-book instrument key ([`book_instrument_key`] maps a commodity underlying's
    /// symbol ticker onto the opaque `instrument_id` a book scopes on), so an RFQ for this
    /// instrument resolves to a book listing that id.
    fn commodity_named(ticker: &str, strike: f64) -> celnet_proto::Instrument {
        celnet_proto::Instrument {
            underlying: Some(celnet_proto::Underlying::commodity(
                celnet_proto::CommodityRef::new(celnet_proto::Symbol::new(ticker, ""), "USD"),
            )),
            tenor: Some(celnet_proto::Tenor {
                unit: celnet_proto::tenor::Unit::Years as i32,
                count: 1,
                broken_date: None,
            }),
            expiry_years: 1.0,
            quantity: Some(celnet_proto::Quantity {
                notional: 1_000_000.0,
                base_ccy: true,
            }),
            side: celnet_proto::Side::TwoWay as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(celnet_proto::instrument::Product::Vanilla(
                celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(celnet_proto::StrikeOrDelta {
                        spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                    }),
                },
            )),
            ..Default::default()
        }
    }

    /// A hub running ONE enabled book over `instrument_id` with a Flat ±`half_bps`
    /// **price-bps** outbound tiering config, seeded with each `(lp, bid, offer)` member
    /// quote. The hub clock is manual and every ingest is stamped at that instant, so the
    /// composite is fresh regardless of the (separate) edge clock. Mirrors
    /// `aggregation::tests` construction.
    fn flat_book_hub(
        instrument_id: &str,
        half_bps: f64,
        members: &[(&str, f64, f64)],
    ) -> Arc<AggregationHub> {
        const HUB_NOW: i64 = 1_700_000_000_000_000_000;
        let hub = AggregationHub::new(Clock::manual(HUB_NOW));
        let mut store = IdentityStore::default();
        store.aggregated_books.push(AggregatedBookDef {
            id: "agg-book".to_string(),
            name: "agg-book".to_string(),
            member_connection_ids: members.iter().map(|(lp, _, _)| (*lp).to_string()).collect(),
            instrument_scope: Scope::Explicit(vec![instrument_id.to_string()]),
            params: AggregationParams {
                staleness_tau_ms: 30_000,
                max_quote_age_ms: 86_400_000,
                divergence_gating: false,
                min_contributors: 1,
                depth_levels: 1,
            },
            enabled: true,
            tiering: Some(TieringConfig {
                unit: SpreadUnit::PriceBps,
                strategies: vec![StrategySpec::FlatMarkup {
                    half_spread: half_bps,
                }],
                guardrails: Guardrails::new(0.0, 1_000.0, 1_000.0, 1e-9),
                stale_policy: StalePolicy::Suppress,
            }),
        });
        hub.reconcile(&store);
        for (lp, bid, offer) in members {
            assert!(
                hub.ingest(&LpQuote {
                    lp_name: (*lp).to_string(),
                    instrument_id: instrument_id.to_string(),
                    bid: *bid,
                    offer: *offer,
                    bid_size: 1_000_000.0,
                    offer_size: 2_000_000.0,
                    ts_nanos: HUB_NOW,
                }),
                "member {lp} ingested into the book"
            );
        }
        hub
    }

    /// A ready in-process quote edge under `mode` WITH an aggregated-book hub installed.
    fn edge_with_hub(mode: AccessMode, hub: Arc<AggregationHub>) -> QuoteEdge {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let clock = Clock::system();
        let sessions = Arc::new(SessionRegistry::new(clock.clone()));
        let access_store = Arc::new(PositionStore::new());
        access_store.set_access_mode(mode);
        QuoteEdge::new(
            test_link(),
            gate,
            SpreadModel::default(),
            clock,
            Arc::new(SurfaceBook::new()),
        )
        .with_session_access(sessions, access_store)
        .with_aggregation_hub(hub)
    }

    /// (1) An RFQ for an instrument IN a book's scope prices against that book's
    /// ALREADY-TIERED composite — the two-way is the tiered 99.30/99.80 for a 99.55
    /// composite mid (Flat ±25 price-bps; the SAME arithmetic `celnet-tiering`'s own
    /// `flat_price_bps` oracle asserts, hand-check 99.55 ∓ 0.25) — and it DIFFERS from the
    /// synthetic-demo panel (the options-engine premium a no-hub edge produces).
    #[tokio::test]
    async fn rfq_prices_against_the_books_tiered_composite() {
        let hub = flat_book_hub("BND-5Y", 25.0, &[("LP-1", 99.50, 99.60)]);
        let edge = edge_with_hub(AccessMode::Permissive, hub);
        let instrument = commodity_named("BND-5Y", 1.12);

        let quote = edge
            .request_quote(Request::new(quote_request_for(
                "book-1",
                instrument.clone(),
                None,
                None,
            )))
            .await
            .expect("book-covered RFQ prices")
            .into_inner();
        let price = quote.price.expect("two-way");
        assert!((price.bid - 99.30).abs() < 1e-9, "bid={}", price.bid);
        assert!((price.offer - 99.80).abs() < 1e-9, "offer={}", price.offer);
        // A composite market price, not an option premium: no greeks / std-error / surface.
        assert!(
            quote.greeks.is_none(),
            "a book quote carries no option greeks"
        );
        assert!(quote.surface_version.is_none());

        // It differs from the synthetic-demo panel: a no-hub edge prices the SAME commodity
        // as an option premium — nowhere near the 99.80 composite offer.
        let no_hub = edge_under(AccessMode::Permissive).0;
        let engine_quote = no_hub
            .request_quote(Request::new(quote_request_for(
                "eng-1", instrument, None, None,
            )))
            .await
            .expect("options-engine RFQ prices")
            .into_inner();
        let engine_price = engine_quote.price.expect("two-way");
        assert!(
            (engine_price.offer - 99.80).abs() > 1.0,
            "the engine premium ({}) must differ from the book composite offer (99.80)",
            engine_price.offer
        );
        assert!(
            engine_quote.greeks.is_some(),
            "the options-engine path still carries greeks"
        );
    }

    /// (2) On the book-sourced path, ranking / pinning / booking + idempotent AcceptQuote
    /// still hold: the panel is the book's REAL member LPs (never the synthetic dealers),
    /// the tightest member wins both touches, an accept books exactly that member's line,
    /// and a retry returns the SAME execution (no double-book).
    #[tokio::test]
    async fn multi_dealer_ranks_and_books_book_member_lines() {
        // LP-2 (99.52/99.58) is tighter than LP-1 (99.50/99.60) ⇒ wins bid AND offer.
        let hub = flat_book_hub(
            "BND-5Y",
            25.0,
            &[("LP-1", 99.50, 99.60), ("LP-2", 99.52, 99.58)],
        );
        let edge = edge_with_hub(AccessMode::Permissive, hub);
        let instrument = commodity_named("BND-5Y", 1.12);

        let panel = edge
            .request_multi_dealer_quote(Request::new(quote_request_for(
                "md-book", instrument, None, None,
            )))
            .await
            .expect("book multi-dealer RFQ")
            .into_inner();

        // The panel is the book's real member LPs (+ the native tiered line) — never the
        // synthetic SYNTH-LP-* dealers.
        let lp_ids: Vec<&str> = panel.dealers.iter().map(|d| d.lp_id.as_str()).collect();
        assert!(
            lp_ids.contains(&"LP-1") && lp_ids.contains(&"LP-2"),
            "member LPs on the panel: {lp_ids:?}"
        );
        assert!(
            !lp_ids.iter().any(|id| id.starts_with("SYNTH-LP-")),
            "no synthetic dealers on a book panel: {lp_ids:?}"
        );
        assert_eq!(panel.best_bid_lp_id, "LP-2", "LP-2 is the best bid");
        assert_eq!(panel.best_offer_lp_id, "LP-2", "LP-2 is the best offer");

        // Book the winning member's OFFER (Buy lifts LP-2's 99.58).
        let exec = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: panel.quote_id,
                idempotency_key: "md-book".to_owned(),
                side: Side::Buy as i32,
                lp_id: "LP-2".to_owned(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect("books the LP-2 line")
            .into_inner();
        assert!(
            (exec.traded_premium - 99.58).abs() < 1e-9,
            "premium={}",
            exec.traded_premium
        );
        // The fill is attributed to LP-2 (never an anonymous fill).
        let quoted_by = exec
            .attribution
            .as_ref()
            .and_then(|a| a.quoted_by.as_ref())
            .expect("booked line attribution");
        assert_eq!(quoted_by.book, "LP-2");

        // Idempotent AcceptQuote: same key/side/line ⇒ the SAME execution, never a
        // double-book.
        let retry = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: panel.quote_id,
                idempotency_key: "md-book".to_owned(),
                side: Side::Buy as i32,
                lp_id: "LP-2".to_owned(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect("retry returns the same booking")
            .into_inner();
        assert_eq!(retry.execution_id, exec.execution_id);
        assert!((retry.traded_premium - 99.58).abs() < 1e-9);

        // An empty-lp_id accept on the SAME quote_id is a different dealer line (the native
        // tiered firm line) than the already-booked LP-2 line ⇒ refused (anti-double-book).
        let conflict = edge
            .accept_quote(Request::new(QuoteAccept {
                quote_id: panel.quote_id,
                idempotency_key: "md-book".to_owned(),
                side: Side::Buy as i32,
                lp_id: String::new(),
                session_token: None,
                principal: None,
            }))
            .await
            .expect_err("a different dealer line on a booked quote is refused");
        assert_eq!(conflict.code(), tonic::Code::FailedPrecondition);
    }

    /// (3) Regression guard: an instrument with NO covering book falls back to the existing
    /// options-engine path byte-identically — the hub's mere presence never changes the
    /// price of an uncovered instrument. An FX EURUSD RFQ resolves no book (the only book
    /// scopes "BND-5Y"), so the with-hub and no-hub quotes are bit-for-bit equal.
    #[tokio::test]
    async fn no_covering_book_falls_back_byte_identically() {
        let hub = flat_book_hub("BND-5Y", 25.0, &[("LP-1", 99.50, 99.60)]);
        let with_hub = edge_with_hub(AccessMode::Permissive, hub);
        let no_hub = edge_under(AccessMode::Permissive).0;

        let q_hub = with_hub
            .request_quote(Request::new(quote_request("fb-hub", None, None)))
            .await
            .expect("fx prices with the hub present")
            .into_inner();
        let q_bare = no_hub
            .request_quote(Request::new(quote_request("fb-bare", None, None)))
            .await
            .expect("fx prices without a hub")
            .into_inner();

        let ph = q_hub.price.expect("two-way");
        let pb = q_bare.price.expect("two-way");
        assert_eq!(ph.bid.to_bits(), pb.bid.to_bits(), "bid byte-identical");
        assert_eq!(
            ph.offer.to_bits(),
            pb.offer.to_bits(),
            "offer byte-identical"
        );
        assert_eq!(
            q_hub.resolved_strike.to_bits(),
            q_bare.resolved_strike.to_bits(),
            "resolved strike byte-identical"
        );
        assert!(
            q_hub.greeks.is_some(),
            "the uncovered instrument still prices through the options engine (greeks present)"
        );
    }
}
