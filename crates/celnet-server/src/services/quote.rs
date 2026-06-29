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
    Quote, QuoteAccept, QuoteReject, QuoteRequest, RejectAck, Side, TwoWayPrice, owner,
};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use celnet_entitlements::{Action, AssetClass};

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::pricer::{ConventionSet, price_instrument};
use crate::readiness::ReadinessGate;
use crate::services::access::{
    RequiredAuthority, ResolvedCaller, authorize_caller, resolve_caller,
};
use crate::services::forward::{Serve, route_underlying, serve_mode};
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::services::risk::federate::Fleet;
use crate::services::risk::store::PositionStore;
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

/// The attribution stamped on a synthetic dealer's panel line: the synthetic LP's
/// own auto-pricer seat quoted it, and the client's requesting seat (when supplied
/// on the originating request) holds a booking of it — mirroring how the maker
/// line is attributed by [`super::attribution::resolve`]. A booked dealer line's
/// execution therefore carries the LP identity, never an anonymous fill.
fn synthetic_lp_attribution(lp_id: &str, held_by: Option<BookId>) -> AttributionRecord {
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
            .map_err(|e| Status::unavailable(e.to_string()))?;
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
        let conv = ConventionSet::decode(&wire_conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

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
        let priced = price_instrument(&instrument, &effective_market, &conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let two_way = self.spread.two_way(priced.greeks.price, &priced.greeks);

        let now = self.clock.now_nanos();
        let quote_id = self.mint_quote_id();
        let quote = Quote {
            quote_id,
            idempotency_key: req.idempotency_key.clone(),
            price: Some(two_way),
            greeks: Some(priced.greeks.into()),
            conventions: Some(wire_conv),
            resolved_strike: priced.resolved_strike,
            epoch_nanos: now,
            valid_until_nanos: now + QUOTE_VALIDITY_NANOS,
            correlation_id: req.correlation_id,
            surface_version: echo_version,
            // Stamp the who's-trading chain: the maker auto-pricer is the
            // `quoted_by` (the engine priced and showed this line); the client's
            // requesting seat, when supplied, becomes the `held_by`. So the flow is
            // attributable request→quote→booking and never anonymous.
            attribution: Some(super::attribution::resolve(req.attribution.as_ref())),
            // MC standard error for an MC-priced product (clamped cliquet); `None`
            // for closed-form products. Surfaced on the Quote so the WS/SDK quote
            // path discloses the same uncertainty the gRPC PriceResponse does.
            price_std_error: priced.std_error,
        };

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
                },
            );
            if !req.idempotency_key.is_empty() {
                store.by_key.insert(req.idempotency_key.clone(), quote_id);
            }
        }

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
        // The synthetic demo/test dealers join only on a serving (in-process)
        // edge: each is a deterministic in-process quoter over the SAME edge mid
        // with its fixed per-dealer spread/skew offsets, stamped with the quote's
        // epoch and last-look window. In a distributed topology the accept routes
        // back to the issuing backend (which owns the booking store), so a
        // forwarding edge keeps the panel native-only — the backend's own line —
        // exactly as before.
        if !matches!(serve_mode(self.fleet.as_ref()), Serve::Forward(_)) {
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

        // Project the ranked panel rows onto wire `DealerQuote`s. Every row quotes
        // the SAME resolved line (the maker-priced strike); the native row carries
        // the edge-priced greeks / MC std-error (the pricing belongs to the maker;
        // the engine owns only the aggregation), while a synthetic dealer row
        // carries only its quoted price and its own auto-pricer attribution (an LP
        // discloses a price, not its greeks).
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
                        Some(synthetic_lp_attribution(&row.lp_id, held_by.clone()))
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
        Ok(Response::new(multi))
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
    use crate::services::sessions::AuthenticatedUser;
    use celnet_entitlements::AccessMode;

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
                desk_id: Some("g10".to_owned()),
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
        QuoteRequest {
            idempotency_key: key.to_owned(),
            instrument: Some(vanilla_call(1.12)),
            conventions: Some(wire_conventions()),
            correlation_id: None,
            surface_version: None,
            attribution: None,
            session_token,
            principal,
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
}
