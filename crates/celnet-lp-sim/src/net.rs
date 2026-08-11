//! The **network feed mode** of the `lp-sim` binary (`--addr`): push the LP-SIM
//! panel's live two-ways to a running celnet server's `LiquidityFeedService.LpFeed`
//! ingest, so the consolidated composite surfaces to GUI subscribers of the target
//! aggregated book — rather than the composite being printed locally.
//!
//! The feed is a **client-streaming** gRPC call: the fleet's per-member,
//! per-instrument top-of-book is produced on a fixed interval and streamed as
//! [`LpQuote`]s until the process is stopped (or, in `--once` mode, one round then a
//! half-close to collect the [`LpFeedAck`]). The connect / reconnect lifecycle is
//! supervised in-process (a dropped connection re-dials after a short backoff),
//! mirroring how the FIX simulator supervises its initiator.
//!
//! The local in-process mode (the crate's default) stays fully synchronous; the
//! tokio runtime here is built only when `--addr` is passed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use celnet_aggregation::VenueFeed;
use celnet_proto::auth_service_client::AuthServiceClient;
use celnet_proto::liquidity_feed_service_client::LiquidityFeedServiceClient;
use celnet_proto::{ListAggregatedBooksRequest, LoginRequest, LpQuote};
use tonic::transport::{Channel, Endpoint};

use crate::books::{StreamPlan, resolve_from_descs};
use crate::lp::Fault;
use crate::lpsim::LpQuoteSnapshot;
use crate::quoted::QuotedLine;
use crate::rng::{child_seed, unit01};
use crate::{LpSimConfig, SimLp, build_fleet};

/// The seeded admin account the server ensures on first boot — the out-of-the-box
/// service login so the daemon authenticates against a fresh UAT box with no extra
/// provisioning. (The value mirrors the server's `SEED_ADMIN_EMAIL` /
/// `SEED_ADMIN_PASSWORD`; deployments override via `--user`/`--password`.)
pub const DEFAULT_SERVICE_EMAIL: &str = "admin@celnet.com";
/// The seeded admin password (see [`DEFAULT_SERVICE_EMAIL`]).
pub const DEFAULT_SERVICE_PASSWORD: &str = "password";

/// The reconnect backoff after a dropped / failed feed connection.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// Produce ONE round of wire [`LpQuote`]s: every member's current top-of-book for
/// every selected line at `now_nanos`, stamped with the canonical `instrument_id`.
/// Pure and deterministic for a fixed `(fleet, lines, now_nanos)` — the unit test
/// asserts its shape without a network.
#[must_use]
pub fn lp_quotes_round(fleet: &[SimLp], lines: &[QuotedLine], now_nanos: i64) -> Vec<LpQuote> {
    let mut out = Vec::with_capacity(fleet.len() * lines.len());
    for member in fleet {
        for line in lines {
            if let Some(q) = member.top_of_book(&line.instrument, now_nanos) {
                let snap = LpQuoteSnapshot::from_quote(&q, line);
                out.push(LpQuote {
                    lp_name: snap.lp_name,
                    instrument_id: snap.instrument_id,
                    bid: snap.bid,
                    offer: snap.offer,
                    bid_size: snap.bid_size,
                    offer_size: snap.offer_size,
                    ts_nanos: snap.ts,
                });
            }
        }
    }
    out
}

/// Run the network feed to `addr` (e.g. `http://127.0.0.1:50051`): build the fleet,
/// then connect + stream rounds every `interval_secs`, supervising reconnects. In
/// `--once` mode a single round is streamed and the returned [`LpFeedAck`] accepted
/// count is reported. Blocks the calling thread on a private tokio runtime.
///
/// # Errors
/// Returns a message only for an unrecoverable setup failure (the runtime cannot be
/// built). Connection failures are logged and retried (continuous mode) or surfaced
/// after one attempt (`--once`).
pub fn run_network_feed(
    cfg: &LpSimConfig,
    selection: &[QuotedLine],
    addr: &str,
    interval_secs: u64,
    once: bool,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build tokio runtime: {e}"))?;
    runtime.block_on(feed_loop(cfg, selection, addr, interval_secs, once))
}

/// The async connect → stream → (reconnect) supervision loop.
async fn feed_loop(
    cfg: &LpSimConfig,
    selection: &[QuotedLine],
    addr: &str,
    interval_secs: u64,
    once: bool,
) -> Result<(), String> {
    loop {
        match connect_and_stream(cfg, selection, addr, interval_secs, once).await {
            Ok(accepted) => {
                if once {
                    tracing::info!(accepted, "lp-sim: server accepted quotes (once)");
                    println!("[lp-sim] server accepted {accepted} quote(s)");
                    return Ok(());
                }
                // A clean end of a continuous stream (server closed) — re-dial.
                tracing::warn!("lp-sim: feed stream ended; reconnecting");
            }
            Err(e) => {
                if once {
                    return Err(e);
                }
                tracing::warn!(error = %e, "lp-sim: feed connection error; reconnecting");
                eprintln!("[lp-sim] feed error: {e} — reconnecting in {RECONNECT_BACKOFF:?}");
            }
        }
        tokio::time::sleep(RECONNECT_BACKOFF).await;
    }
}

/// Dial `addr` and stream the fleet's quotes; returns the server's accepted count
/// when the stream ends (only reached in `--once` mode, or on a server-side close).
async fn connect_and_stream(
    cfg: &LpSimConfig,
    selection: &[QuotedLine],
    addr: &str,
    interval_secs: u64,
    once: bool,
) -> Result<u64, String> {
    let mut client = LiquidityFeedServiceClient::connect(addr.to_owned())
        .await
        .map_err(|e| format!("connect {addr}: {e}"))?;
    tracing::info!(
        addr,
        members = cfg.members,
        "lp-sim: connected; streaming LpFeed"
    );

    // The fleet is rebuilt per connection so a reconnect reproduces the same
    // deterministic feed from the seed.
    let fleet = build_fleet(cfg, selection);
    let lines = selection.to_vec();
    let interval = Duration::from_secs(interval_secs.max(1));
    let start = std::time::Instant::now();

    // The client-streaming request: an unfold generator yields the next `LpQuote`,
    // sleeping `interval` between rounds; `--once` ends the stream after one round.
    let state = FeedState {
        fleet,
        lines,
        cursor: 0,
        pending: Vec::new(),
        round_done_once: false,
        interval,
        once,
        start,
    };
    let request = futures_util::stream::unfold(state, |mut st| async move {
        st.next_quote().await.map(|q| (q, st))
    });

    let ack = client
        .lp_feed(request)
        .await
        .map_err(|e| format!("lp_feed stream: {e}"))?
        .into_inner();
    Ok(ack.accepted)
}

/// The unfold generator state driving the client-streaming request.
struct FeedState {
    fleet: Vec<SimLp>,
    lines: Vec<QuotedLine>,
    /// The buffered quotes of the current round, drained front-to-back.
    pending: Vec<LpQuote>,
    /// Read cursor into `pending`.
    cursor: usize,
    /// In `--once` mode, whether the single round has already been emitted.
    round_done_once: bool,
    interval: Duration,
    once: bool,
    start: std::time::Instant,
}

impl FeedState {
    /// The next `LpQuote` to stream, or `None` to end the stream (only in `--once`
    /// mode, after one full round). Sleeps `interval` between rounds in continuous
    /// mode.
    async fn next_quote(&mut self) -> Option<LpQuote> {
        loop {
            if self.cursor < self.pending.len() {
                let q = self.pending[self.cursor].clone();
                self.cursor += 1;
                return Some(q);
            }
            // Current round drained. In `--once` mode, one round only.
            if self.once && self.round_done_once {
                return None;
            }
            if self.round_done_once {
                // Pace continuous rounds by the interval.
                tokio::time::sleep(self.interval).await;
            }
            let now_nanos = self.now_nanos();
            self.pending = lp_quotes_round(&self.fleet, &self.lines, now_nanos);
            self.cursor = 0;
            self.round_done_once = true;
            if self.pending.is_empty() {
                // Nothing to quote (no modellable instruments) — end rather than spin.
                return None;
            }
        }
    }

    /// The valuation clock in epoch nanoseconds (real wall time so the server's
    /// staleness decay sees a monotonically advancing observation instant).
    fn now_nanos(&self) -> i64 {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        i64::try_from(since_epoch.as_nanos()).unwrap_or(i64::MAX)
            + i64::try_from(self.start.elapsed().as_nanos()).unwrap_or(0)
    }
}

// ===========================================================================
// Book-aware network feed — the deployed daemon (`--book-poll`)
// ===========================================================================

/// The service credentials the daemon logs in with (only `ListAggregatedBooks`
/// needs auth; the `LpFeed` ingest itself is an unauthenticated backend feed).
#[derive(Debug, Clone)]
pub struct LoginCredentials {
    /// The login email.
    pub email: String,
    /// The plaintext password (checked against the server's Argon2id hash).
    pub password: String,
}

impl Default for LoginCredentials {
    fn default() -> Self {
        Self {
            email: DEFAULT_SERVICE_EMAIL.to_string(),
            password: DEFAULT_SERVICE_PASSWORD.to_string(),
        }
    }
}

/// The seeded schedule of occasional, transient LP faults applied at stream time.
///
/// At most **one** member is faulted in any round (the one with the lowest seeded
/// draw, and only if that draw falls under [`probability`](Self::probability)), so at
/// least `members − 1` fresh contributors always remain — the surviving panel is
/// never crossed and divergence gating stays decidable. A faulted member emits either
/// an [`Fault::Outlier`] (a divergent print the server's MAD gate excludes and
/// reports) or an [`Fault::Stale`] (a frozen observation the server staleness-decays
/// then drops). Deterministic in `(seed, member, round)`, so a re-run reproduces the
/// exact fault tape.
#[derive(Debug, Clone)]
pub struct FaultSchedule {
    /// Whether faults are injected at all.
    pub enabled: bool,
    /// Per-round probability that the round's flakiest member is faulted.
    pub probability: f64,
    /// The signed price-point displacement of an injected outlier print (its
    /// magnitude should exceed the book's divergence tolerance so the gate excludes
    /// it).
    pub outlier_shift: f64,
    /// How far back (seconds) an injected stale print's observation ts is frozen —
    /// beyond the book's hard max-age so the server drops it.
    pub stale_age_secs: f64,
    /// The fault-tape seed (independent of the price seed).
    pub seed: u64,
}

impl Default for FaultSchedule {
    fn default() -> Self {
        Self {
            enabled: true,
            probability: 0.18,
            outlier_shift: 0.9,
            stale_age_secs: 90.0,
            seed: 0x00FA_0175_2026_0723,
        }
    }
}

impl FaultSchedule {
    /// A schedule that never injects a fault (broadcast/local demo, or tests).
    #[must_use]
    pub fn off() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Pick the single member (if any) that is faulted this round, and the fault it
    /// emits. Returns `None` when faults are disabled, the panel is too small to keep
    /// ≥ 3 survivors, or no member's draw falls under [`probability`](Self::probability).
    #[must_use]
    fn round_fault(
        &self,
        n: usize,
        price_seed: u64,
        round: u64,
        now_nanos: i64,
    ) -> Option<(usize, Fault)> {
        // Need ≥ 3 survivors after excluding the one faulted member for the server's
        // median-consensus gate to stay decidable.
        if !self.enabled || n < 4 {
            return None;
        }
        // The member with the lowest per-round draw is the candidate; fault it only if
        // that draw is under the probability threshold.
        let mut best: (f64, usize) = (f64::INFINITY, 0);
        for i in 0..n {
            let ms = child_seed(price_seed, i) ^ self.seed;
            let u = unit01(ms, round);
            if u < best.0 {
                best = (u, i);
            }
        }
        if best.0 >= self.probability {
            return None;
        }
        let i = best.1;
        let ms = child_seed(price_seed, i) ^ self.seed;
        // Independent draws choose outlier-vs-stale and the outlier sign.
        if unit01(ms, round ^ 0xF00D) < 0.5 {
            let sign = if unit01(ms, round ^ 0xBEEF) < 0.5 {
                -1.0
            } else {
                1.0
            };
            Some((
                i,
                Fault::Outlier {
                    shift: sign * self.outlier_shift,
                },
            ))
        } else {
            let age = (self.stale_age_secs * 1e9) as i64;
            Some((
                i,
                Fault::Stale {
                    frozen_at_nanos: now_nanos.saturating_sub(age),
                },
            ))
        }
    }
}

/// The tunables of a book-aware feed run (beyond the fleet [`LpSimConfig`]).
#[derive(Debug, Clone)]
pub struct BookFeedOptions {
    /// How often to poll `ListAggregatedBooks` and re-resolve the streaming plan.
    pub book_poll: Duration,
    /// How often to emit a fresh round of two-ways for the current plan.
    pub quote_interval: Duration,
    /// The service login used to authenticate the book poll.
    pub credentials: LoginCredentials,
    /// The occasional-fault schedule applied at stream time.
    pub faults: FaultSchedule,
    /// Emit a single round against the first resolved plan, then exit.
    pub once: bool,
}

/// Authenticate against `AuthService.Login` on `channel`, returning the bearer
/// `session_token`. Reuses the same generated gRPC client the rest of the estate
/// logs in with — no hand-rolled auth.
async fn login(channel: Channel, creds: &LoginCredentials) -> Result<String, String> {
    let mut auth = AuthServiceClient::new(channel);
    let resp = auth
        .login(LoginRequest {
            email: creds.email.clone(),
            password: creds.password.clone(),
            correlation_id: None,
        })
        .await
        .map_err(|s| format!("login as {}: {}", creds.email, s.message()))?
        .into_inner();
    Ok(resp.session_token)
}

/// Poll `AuthService.ListAggregatedBooks` on `channel` with the bearer `token`,
/// returning the current roster.
async fn list_books(
    channel: Channel,
    token: &str,
) -> Result<Vec<celnet_proto::AggregatedBookDesc>, String> {
    let mut auth = AuthServiceClient::new(channel);
    let resp = auth
        .list_aggregated_books(ListAggregatedBooksRequest {
            session_token: token.to_string(),
            correlation_id: None,
        })
        .await
        .map_err(|s| format!("list aggregated books: {}", s.message()))?
        .into_inner();
    Ok(resp.books)
}

/// Produce one round of wire [`LpQuote`]s for exactly the `(member, instrument)`
/// pairs in `plan`, at `now_nanos`. `by_id` resolves a plan's canonical
/// `instrument_id` to the line that prices it; `by_name` resolves a member name to
/// its [`SimLp`]. At most one member is transiently faulted this round per `faults`.
/// Pure and deterministic for a fixed `(fleet, plan, now, round)`.
#[must_use]
pub fn plan_quotes_round(
    cfg: &LpSimConfig,
    fleet: &[SimLp],
    by_id: &BTreeMap<&str, &QuotedLine>,
    plan: &StreamPlan,
    now_nanos: i64,
    round: u64,
    faults: &FaultSchedule,
) -> Vec<LpQuote> {
    // Name → (index, member) for the plan's member lookups.
    let by_name: BTreeMap<&str, (usize, &SimLp)> = fleet
        .iter()
        .enumerate()
        .map(|(i, m)| (m.venue().as_str(), (i, m)))
        .collect();

    // At most one member is faulted this round; build its transient faulted clone once.
    let flaky: Option<(usize, SimLp)> = faults
        .round_fault(fleet.len(), cfg.seed, round, now_nanos)
        .map(|(idx, fault)| (idx, fleet[idx].clone().with_fault(fault)));

    let mut out = Vec::with_capacity(plan.len());
    for key in plan.keys() {
        let Some(&(idx, member)) = by_name.get(key.lp_name.as_str()) else {
            continue;
        };
        let Some(line) = by_id.get(key.instrument_id.as_str()) else {
            continue;
        };
        let instrument = &line.instrument;
        let quote = match &flaky {
            Some((fidx, faulted)) if *fidx == idx => faulted.top_of_book(instrument, now_nanos),
            _ => member.top_of_book(instrument, now_nanos),
        };
        if let Some(q) = quote {
            let snap = LpQuoteSnapshot::from_quote(&q, line);
            out.push(LpQuote {
                lp_name: snap.lp_name,
                instrument_id: snap.instrument_id,
                bid: snap.bid,
                offer: snap.offer,
                bid_size: snap.bid_size,
                offer_size: snap.offer_size,
                ts_nanos: snap.ts,
            });
        }
    }
    out
}

/// The plan the streaming generator reads each round, shared with the poll task.
type SharedPlan = Arc<Mutex<Arc<StreamPlan>>>;

/// Run the **book-aware** network feed to `addr`: authenticate, poll the enabled
/// aggregated books on `opts.book_poll`, resolve the `(LP-SIM member × instrument)`
/// streams the sim owns, and push those two-ways to the server's `LpFeed` ingest,
/// picking up book creates/edits/deletes automatically. `universe` is the sim's full
/// quotable set (cash bonds + listed Treasury futures); each book selects from it.
/// Blocks the calling thread on a private tokio runtime; supervises reconnects.
///
/// # Errors
/// Returns a message only for an unrecoverable setup failure (the runtime cannot be
/// built) or, in `--once` mode, the first connection/login/list error.
pub fn run_book_aware_feed(
    cfg: &LpSimConfig,
    universe: &[QuotedLine],
    addr: &str,
    opts: &BookFeedOptions,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build tokio runtime: {e}"))?;
    runtime.block_on(book_feed_supervise(cfg, universe, addr, opts))
}

/// The connect → login → poll+stream → (reconnect) supervision loop.
async fn book_feed_supervise(
    cfg: &LpSimConfig,
    universe: &[QuotedLine],
    addr: &str,
    opts: &BookFeedOptions,
) -> Result<(), String> {
    loop {
        match book_feed_session(cfg, universe, addr, opts).await {
            Ok(()) => {
                if opts.once {
                    return Ok(());
                }
                tracing::warn!("lp-sim: book-aware feed stream ended; reconnecting");
            }
            Err(e) => {
                if opts.once {
                    return Err(e);
                }
                tracing::warn!(error = %e, "lp-sim: book-aware feed error; reconnecting");
                eprintln!(
                    "[lp-sim] book-aware feed error: {e} — reconnecting in {RECONNECT_BACKOFF:?}"
                );
            }
        }
        tokio::time::sleep(RECONNECT_BACKOFF).await;
    }
}

/// One connected session: dial + login, do an initial poll, spawn the background
/// book poller, then run the `LpFeed` client stream until it ends.
async fn book_feed_session(
    cfg: &LpSimConfig,
    universe: &[QuotedLine],
    addr: &str,
    opts: &BookFeedOptions,
) -> Result<(), String> {
    let channel = Endpoint::from_shared(addr.to_owned())
        .map_err(|e| format!("invalid endpoint {addr}: {e}"))?
        .connect()
        .await
        .map_err(|e| format!("connect {addr}: {e}"))?;

    let token = login(channel.clone(), &opts.credentials).await?;
    tracing::info!(addr, user = %opts.credentials.email, "lp-sim: authenticated; book-aware feed");

    // The members we impersonate and the instruments we can price.
    let members: BTreeSet<String> = (0..cfg.members.max(1))
        .map(|i| cfg.member_venue(i))
        .collect();
    let priceable: BTreeSet<String> = universe
        .iter()
        .map(|l| l.instrument_id().to_string())
        .collect();

    // Resolve the initial plan synchronously so the first stream round is correct.
    let shared: SharedPlan = Arc::new(Mutex::new(Arc::new(StreamPlan::default())));
    match list_books(channel.clone(), &token).await {
        Ok(books) => {
            let plan = resolve_from_descs(&books, &members, &priceable);
            log_plan("initial", &StreamPlan::default(), &plan);
            *shared.lock().expect("plan mutex") = Arc::new(plan);
        }
        Err(e) => {
            eprintln!("[lp-sim] initial book poll failed: {e} — starting empty, will retry");
        }
    }

    // Background poller (skipped in --once: the initial plan is streamed once).
    let poll_handle = if opts.once {
        None
    } else {
        let ch = channel.clone();
        let mem = members.clone();
        let pri = priceable.clone();
        let sh = Arc::clone(&shared);
        let creds = opts.credentials.clone();
        let poll = opts.book_poll;
        Some(tokio::spawn(async move {
            book_poll_loop(ch, token, creds, mem, pri, sh, poll).await;
        }))
    };

    // The client-streaming feed: reads the current plan each round.
    let result = run_plan_stream(channel, cfg, universe, Arc::clone(&shared), opts).await;

    if let Some(h) = poll_handle {
        h.abort();
    }
    result
}

/// The background loop that re-polls the books and republishes the resolved plan,
/// logging the add/remove diff. Re-logins on an auth error so an expired session
/// self-heals without dropping the feed stream.
async fn book_poll_loop(
    channel: Channel,
    mut token: String,
    creds: LoginCredentials,
    members: BTreeSet<String>,
    priceable: BTreeSet<String>,
    shared: SharedPlan,
    poll: Duration,
) {
    loop {
        tokio::time::sleep(poll).await;
        match list_books(channel.clone(), &token).await {
            Ok(books) => {
                let next = resolve_from_descs(&books, &members, &priceable);
                let prev = { shared.lock().expect("plan mutex").clone() };
                if next != *prev {
                    log_plan("poll", &prev, &next);
                    *shared.lock().expect("plan mutex") = Arc::new(next);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "lp-sim: book poll failed; re-authenticating");
                if let Ok(fresh) = login(channel.clone(), &creds).await {
                    token = fresh;
                }
            }
        }
    }
}

/// Log a plan transition as a compact add/remove diff line.
fn log_plan(phase: &str, prev: &StreamPlan, next: &StreamPlan) {
    let diff = prev.diff(next);
    eprintln!(
        "[lp-sim] {phase} plan: {} stream(s) over {} member(s) × {} instrument(s) (+{} / -{})",
        next.len(),
        next.members().len(),
        next.instruments().len(),
        diff.added.len(),
        diff.removed.len(),
    );
}

/// Drive the `LpFeed` client-streaming RPC from the shared plan. Owns everything the
/// generator touches (so the stream is `'static`); ends when the server closes it.
async fn run_plan_stream(
    channel: Channel,
    cfg: &LpSimConfig,
    universe: &[QuotedLine],
    shared: SharedPlan,
    opts: &BookFeedOptions,
) -> Result<(), String> {
    let mut client = LiquidityFeedServiceClient::new(channel);
    let state = PlanFeedState {
        cfg: cfg.clone(),
        fleet: build_fleet(cfg, universe),
        lines: universe.to_vec(),
        shared,
        faults: opts.faults.clone(),
        interval: opts.quote_interval.max(Duration::from_secs(1)),
        once: opts.once,
        round: 0,
        pending: Vec::new(),
        cursor: 0,
        started: false,
        start: std::time::Instant::now(),
    };
    let request = futures_util::stream::unfold(state, |mut st| async move {
        st.next_quote().await.map(|q| (q, st))
    });
    let ack = client
        .lp_feed(request)
        .await
        .map_err(|e| format!("lp_feed stream: {e}"))?
        .into_inner();
    tracing::info!(accepted = ack.accepted, "lp-sim: book-aware feed accepted");
    if opts.once {
        println!("[lp-sim] server accepted {} quote(s)", ack.accepted);
    }
    Ok(())
}

/// The unfold generator state for the book-aware client stream. Owns the fleet, the
/// full universe, and the shared plan handle.
struct PlanFeedState {
    cfg: LpSimConfig,
    fleet: Vec<SimLp>,
    lines: Vec<QuotedLine>,
    shared: SharedPlan,
    faults: FaultSchedule,
    interval: Duration,
    once: bool,
    round: u64,
    pending: Vec<LpQuote>,
    cursor: usize,
    started: bool,
    start: std::time::Instant,
}

impl PlanFeedState {
    /// The next `LpQuote` to stream, or `None` to end the stream (only in `--once`
    /// after one round). Buffers a full round from the current plan, then paces the
    /// next round by the interval.
    async fn next_quote(&mut self) -> Option<LpQuote> {
        loop {
            if self.cursor < self.pending.len() {
                let q = self.pending[self.cursor].clone();
                self.cursor += 1;
                return Some(q);
            }
            if self.once && self.started {
                return None;
            }
            if self.started {
                tokio::time::sleep(self.interval).await;
            }
            let now = self.now_nanos();
            let plan = { self.shared.lock().expect("plan mutex").clone() };
            let by_id: BTreeMap<&str, &QuotedLine> =
                self.lines.iter().map(|l| (l.instrument_id(), l)).collect();
            self.pending = plan_quotes_round(
                &self.cfg,
                &self.fleet,
                &by_id,
                &plan,
                now,
                self.round,
                &self.faults,
            );
            self.cursor = 0;
            self.round += 1;
            self.started = true;
            // An empty plan (no participating book yet) must NOT end the stream — keep
            // the connection open and re-poll next interval, so a book created later is
            // picked up. In --once we return None (nothing to stream this round).
            if self.pending.is_empty() {
                if self.once {
                    return None;
                }
                tokio::time::sleep(self.interval).await;
            }
        }
    }

    /// The valuation clock in epoch nanoseconds (real wall time plus elapsed, so the
    /// server's staleness decay sees a monotonically advancing instant).
    fn now_nanos(&self) -> i64 {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        i64::try_from(since_epoch.as_nanos()).unwrap_or(i64::MAX)
            + i64::try_from(self.start.elapsed().as_nanos()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universe::TreasuryBond;
    use celnet_types::BrokenDate;

    #[test]
    fn one_round_produces_a_quote_per_member_per_bond() {
        let cfg = LpSimConfig {
            members: 3,
            settlement: BrokenDate::new(2026, 4, 16),
            ..LpSimConfig::default()
        };
        let universe = crate::load_coupon_universe();
        let mut selection: Vec<TreasuryBond> = universe
            .into_iter()
            .filter(|b| {
                b.yield_model(cfg.settlement, cfg.reversion_per_sec, cfg.perturbation)
                    .is_some()
            })
            .take(2)
            .collect();
        selection.truncate(2);
        assert_eq!(selection.len(), 2, "need two modellable bonds");
        let lines = crate::universe::bond_lines(
            &selection,
            cfg.settlement,
            cfg.reversion_per_sec,
            cfg.perturbation,
        );
        let fleet = build_fleet(&cfg, &lines);
        let now = 1_700_000_000_000_000_000;
        let quotes = lp_quotes_round(&fleet, &lines, now);
        // 3 members × 2 bonds = 6 quotes, each well-formed and CUSIP-stamped.
        assert_eq!(quotes.len(), 6);
        for q in &quotes {
            assert!(!q.lp_name.is_empty());
            assert_eq!(
                q.instrument_id.len(),
                9,
                "instrument_id is the 9-char CUSIP"
            );
            assert!(q.bid.is_finite() && q.offer.is_finite());
            assert!(q.offer >= q.bid, "two-way is not crossed");
            assert_eq!(q.ts_nanos, now);
        }
        // Every member of the panel contributed.
        let venues: std::collections::HashSet<&str> =
            quotes.iter().map(|q| q.lp_name.as_str()).collect();
        assert_eq!(venues.len(), 3);
    }
}
