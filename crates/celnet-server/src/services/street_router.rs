//! The **outbound street-order router** — the seam that turns a hedge decision into a
//! `NewOrderSingle(D)` on a real socket and waits for the counterparty's
//! `ExecutionReport(8)`.
//!
//! # What this replaces
//!
//! The aggregation hub knows which panel members are *showing* a firm price
//! ([`LpHedgeSource`](crate::services::auto_hedge::LpHedgeSource)). Before this module
//! that observation was converted straight into a fill: the best standing quote became
//! a booked hedge without any socket being opened and without any counterparty ever
//! being asked whether it would trade. A quote is an invitation, not an agreement —
//! the LP may be out of size, may have moved, may not want our axe. This module asks.
//!
//! ```text
//!   booking thread                router runtime            counterparty
//!  ───────────────────────────────────────────────────────────────────────
//!   execute_external
//!     └─ route(intent) ──job──►  member actor
//!        (blocks, bounded)         └─ NewOrderSingle(D) ──────►
//!                                                    (venue matches)
//!        ◄──────answer────────      ◄────── ExecutionReport(8)
//! ```
//!
//! # The concurrency problem, and the shape chosen
//!
//! Booking is **synchronous**: `RatesPositionStore::stamp_internalise` runs on whatever
//! thread booked the fill, called from an async handler but itself an ordinary `fn`.
//! FIX routing is **inherently asynchronous**: send, await a report, honour a deadline.
//! Three bridges were available and two are unsafe here:
//!
//! * `Handle::current().block_on(..)` — **rejected**. Blocking on the *caller's* runtime
//!   from inside that runtime panics outright, and even where it does not it deadlocks
//!   as soon as the awaited work needs a worker the caller is occupying.
//! * `block_in_place` + the caller's runtime — **rejected as the primary mechanism**.
//!   It only exists on the multi-thread flavour, and it still makes the routed I/O
//!   depend on the caller's runtime having spare capacity.
//! * **A dedicated runtime the caller does not share** — chosen. [`FixStreetRouter`]
//!   owns its own Tokio runtime on its own threads. The booking thread hands a job to a
//!   member actor over a channel and blocks on a plain `std` channel with a deadline.
//!   Because the routed I/O runs on threads the caller is not on, **the caller blocking
//!   cannot starve the work it is waiting for** — the deadlock class is eliminated
//!   structurally rather than by discipline.
//!
//! The caller is nonetheless a Tokio worker most of the time, and a worker parked for a
//! venue round trip would stall the other tasks sharing it. So the blocking wait is
//! wrapped in [`tokio::task::block_in_place`] *when the caller is on a multi-thread
//! runtime* — that hands the worker's other tasks to a sibling for the duration. On a
//! current-thread runtime (tests, tooling) `block_in_place` would panic, so the wait is
//! taken directly, which is safe precisely because of the dedicated-runtime property
//! above. `block_in_place` is an optimisation here, never a correctness requirement.
//!
//! The pinned zero-alloc hot core is untouched: nothing on the pricing path enters this
//! module. Booking already allocates, logs and takes locks.
//!
//! # One session per member, held open
//!
//! A real hedge desk does not log on to re-log-off per order. Each configured member
//! gets an actor task owning one FIX session; the session is established lazily on the
//! first order and reused. A transport failure drops it and the next order re-establishes
//! it once before giving up — so a counterparty restart costs one order's latency, not a
//! permanent outage.
//!
//! # Nothing is ever silently skipped
//!
//! Every path out of [`FixStreetRouter::route`] is a stated
//! [`RouteAnswer`](crate::services::auto_hedge::RouteAnswer): a member with no
//! configured order endpoint is [`RouteAnswer::Unroutable`] carrying
//! [`NO_ENDPOINT_REASON`], a venue that never answers is `Expired` carrying the measured
//! wait, a refusal carries the venue's own `Text(58)` code. There is no branch that
//! returns "nothing happened".

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use celnet_fix::dictionary::MsgType;
use celnet_fix::framing::FrameCursor;
use celnet_fix::messages::{
    self, EXEC_FILLED, EXEC_REJECTED, ExecReportView, MarketOrderParams, ORD_STATUS_CANCELED,
    ORD_STATUS_PARTIALLY_FILLED,
};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig, SessionState};
use celnet_fix::transport::{FrameReader, write_frame};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::config::fix_connections::FixConnectionDef;
use crate::services::auto_hedge::{RouteAnswer, RoutedFill, StreetOrderIntent, StreetOrderRouter};

/// The stated reason for a panel member the operator has not given an order endpoint.
///
/// A configuration fact, deliberately distinct from every market fact: the member did
/// not refuse us and did not go quiet — we have nowhere to send it an order. Surfacing
/// it as its own reason is what stops a half-configured panel looking like an illiquid
/// street.
pub const NO_ENDPOINT_REASON: &str = "no_order_endpoint";

/// The stated reason when a member's session could not be established at all.
pub const NO_SESSION_REASON: &str = "session_unavailable";

/// The stated reason when a member's inbox is saturated — it is already working the
/// maximum number of queued orders and a further one would have to wait unboundedly.
pub const MEMBER_SATURATED_REASON: &str = "member_inbox_saturated";

/// The `Text(58)` reason code a quote-driven venue returns when it will not trade at
/// the price it was itself showing. Routed back at the LP's own ranked level, that is
/// a last look, not a refusal to deal with us — see [`classify_report`].
const NOT_MARKETABLE_CODE: &str = "NOT_MARKETABLE";

/// How long a venue has to answer one order before the attempt expires.
///
/// Generous against the slowest simulated counterparty's own response latency and
/// still short enough that a wedged venue cannot hold a booking thread. Exceeding it
/// is an outcome (`Expired`), never an error.
pub const DEFAULT_VENUE_TIMEOUT: Duration = Duration::from_millis(1_500);

/// How long a TCP connect + logon handshake has before the member is unroutable.
const SESSION_SETUP_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Slack added to the venue deadline for the caller's own channel wait, so the actor's
/// measured `Expired` is what the caller sees rather than a coarser outer timeout.
const HANDOFF_GRACE: Duration = Duration::from_millis(250);

/// Queued orders one member may have outstanding before it is reported saturated.
const MEMBER_INBOX_DEPTH: usize = 16;

/// The heartbeat interval advertised on our logon to a counterparty.
const HEARTBEAT_SECS: u32 = 30;

/// Where — and as whom — we send a panel member its orders.
///
/// Derived entirely from the operator-managed FIX connection registry (see
/// [`order_endpoints`]); this module defines no roster and no addresses of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderEndpoint {
    /// The member id — the aggregated-book `member_connection_ids` entry, which is also
    /// the `LpQuote.lp_name` its quotes arrive under and the `lp_won` a fill is
    /// attributed to.
    pub member_id: String,
    /// The `host:port` we DIAL to reach the member's order acceptor.
    pub addr: String,
    /// Our `SenderCompID` on the outbound session.
    pub sender_comp_id: String,
    /// The member's `SenderCompID` — the peer identity its acceptor stamps and the FSM
    /// checks. A mismatch fails logon, which surfaces as an unroutable member rather
    /// than a silent non-fill.
    pub target_comp_id: String,
}

/// Read the order endpoints out of the operator's FIX connection registry.
///
/// A connection contributes an endpoint exactly when it is **enabled** and carries a
/// non-empty `order_endpoint`. Everything else about it — its dialect, its inbound
/// bind address — is irrelevant here: this is the outbound half of the same managed
/// counterparty, which is why it lives on the same record the operator already edits
/// rather than in a parallel file that could drift out of step with it.
#[must_use]
pub fn order_endpoints(connections: &[FixConnectionDef]) -> Vec<OrderEndpoint> {
    connections
        .iter()
        .filter(|c| c.enabled)
        .filter_map(|c| {
            let addr = c.order_endpoint()?;
            Some(OrderEndpoint {
                member_id: c.id.clone(),
                addr: addr.to_owned(),
                sender_comp_id: c.sender_comp_id.clone(),
                // The registry's `target_comp_id` IS the counterparty's own CompID; an
                // operator who left it blank still gets a working session against a
                // venue that stamps its connection id, which is the deployed shape.
                target_comp_id: if c.target_comp_id.trim().is_empty() {
                    c.id.clone()
                } else {
                    c.target_comp_id.clone()
                },
            })
        })
        .collect()
}

/// One queued order and the channel its answer comes back on.
struct Job {
    intent: OwnedIntent,
    reply: std::sync::mpsc::SyncSender<RouteAnswer>,
}

/// An owned copy of a [`StreetOrderIntent`] — the borrowed form cannot cross to the
/// router runtime.
#[derive(Debug, Clone)]
struct OwnedIntent {
    instrument: String,
    side: u8,
    quantity: f64,
    limit_price: f64,
    ord_type: u8,
    time_in_force: u8,
}

impl OwnedIntent {
    fn from(intent: &StreetOrderIntent<'_>) -> Self {
        Self {
            instrument: intent.instrument.to_owned(),
            side: match intent.side {
                celnet_analytics::StreetSide::Buy => celnet_fix::dialect_fx::SIDE_BUY,
                celnet_analytics::StreetSide::Sell => celnet_fix::dialect_fx::SIDE_SELL,
            },
            quantity: intent.quantity,
            limit_price: intent.limit_price,
            ord_type: intent.ord_type,
            time_in_force: intent.time_in_force,
        }
    }
}

/// A live member: its endpoint and the inbox of its actor task.
struct MemberHandle {
    endpoint: OrderEndpoint,
    tx: mpsc::Sender<Job>,
}

/// The live outbound street-order router.
///
/// Construct once at boot, install the endpoints from the FIX connection registry, and
/// share it as `Arc<dyn StreetOrderRouter>`.
pub struct FixStreetRouter {
    /// The router's OWN runtime — see the module docs. Its threads are disjoint from
    /// the caller's, which is what makes the synchronous wait deadlock-free.
    ///
    /// Started **lazily**, on the first [`Self::set_endpoints`] that arms a member, and
    /// only from inside the `members` write lock (so two callers can never race to build
    /// two of them). An edge whose operator has configured no order routes runs no extra
    /// threads at all — which matters because the test suite alone constructs hundreds of
    /// edges, and a per-edge runtime is a real cost the pinned hot core must not carry.
    runtime: OnceLock<tokio::runtime::Runtime>,
    members: RwLock<HashMap<String, MemberHandle>>,
    venue_timeout: Duration,
}

impl std::fmt::Debug for FixStreetRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let members = self.members.read().map(|m| m.len()).unwrap_or_default();
        f.debug_struct("FixStreetRouter")
            .field("members", &members)
            .field("venue_timeout", &self.venue_timeout)
            .finish()
    }
}

impl FixStreetRouter {
    /// Build a router with [`DEFAULT_VENUE_TIMEOUT`] and no members yet.
    ///
    /// Infallible, and starts no threads: an unconfigured router is inert.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Self::with_timeout(DEFAULT_VENUE_TIMEOUT)
    }

    /// Build a router whose venues have `venue_timeout` to answer an order.
    #[must_use]
    pub fn with_timeout(venue_timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            members: RwLock::new(HashMap::new()),
            venue_timeout,
        })
    }

    /// The router's runtime, started on first use.
    ///
    /// Called ONLY from inside the `members` write lock, which is what makes the
    /// `OnceLock::set` race-free without a second lock (and avoids ever having to drop a
    /// losing runtime from an async context, which panics).
    ///
    /// A runtime that cannot start is reported and yields `None`. The caller then arms no
    /// members, so every order is answered `no_order_endpoint` — the same stated-reason
    /// posture as an unconfigured member, and never a fabricated fill. This is a degraded
    /// subsystem, not a failed boot: an edge that cannot hedge externally must still
    /// price, quote and warehouse, and it says so on every order it could not route.
    fn runtime(&self) -> Option<&tokio::runtime::Runtime> {
        if let Some(rt) = self.runtime.get() {
            return Some(rt);
        }
        match tokio::runtime::Builder::new_multi_thread()
            // Two workers is ample: each member's traffic is one small frame per hedge
            // and every member has its own task, so the bound is socket count, not CPU.
            .worker_threads(2)
            .enable_all()
            .thread_name("celnet-street-router")
            .build()
        {
            Ok(rt) => {
                let _ = self.runtime.set(rt);
                self.runtime.get()
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "street-order router runtime could not start — hedges cannot be routed \
                     and every street order will be answered `no_order_endpoint`"
                );
                None
            }
        }
    }

    /// Install (or refresh) the routable member set.
    ///
    /// A member whose endpoint is unchanged keeps its **existing session** — refreshing
    /// the registry must not tear down healthy sessions. A member that disappeared, or
    /// whose endpoint moved, has its actor dropped, which closes its socket.
    pub fn set_endpoints(&self, endpoints: Vec<OrderEndpoint>) {
        let mut guard = self
            .members
            .write()
            .expect("street-router members poisoned");
        // Start the runtime only when there is actually something to route to.
        let runtime = match endpoints.is_empty() {
            true => None,
            false => self.runtime(),
        };
        let mut next: HashMap<String, MemberHandle> = HashMap::with_capacity(endpoints.len());
        for endpoint in endpoints {
            if let Some(existing) = guard.remove(&endpoint.member_id)
                && existing.endpoint == endpoint
                && !existing.tx.is_closed()
            {
                next.insert(endpoint.member_id.clone(), existing);
                continue;
            }
            let Some(runtime) = runtime else {
                // No runtime ⇒ no actor can be armed. Leaving the member OUT is what
                // makes the failure visible: every order to it is then answered
                // `no_order_endpoint` rather than queued against a task that will
                // never run.
                continue;
            };
            let (tx, rx) = mpsc::channel(MEMBER_INBOX_DEPTH);
            let ep = endpoint.clone();
            let timeout = self.venue_timeout;
            runtime.spawn(member_actor(ep, rx, timeout));
            tracing::info!(
                member = %endpoint.member_id,
                addr = %endpoint.addr,
                "street-order route armed"
            );
            next.insert(endpoint.member_id.clone(), MemberHandle { endpoint, tx });
        }
        for (id, _) in guard.drain() {
            tracing::info!(member = %id, "street-order route withdrawn");
        }
        *guard = next;
    }

    /// The member ids currently routable, sorted — what an operator's panel reports as
    /// "orders can reach this LP".
    #[must_use]
    pub fn routable_members(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .members
            .read()
            .expect("street-router members poisoned")
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }
}

impl StreetOrderRouter for FixStreetRouter {
    fn route(&self, intent: &StreetOrderIntent<'_>) -> RouteAnswer {
        let tx = {
            let guard = self.members.read().expect("street-router members poisoned");
            match guard.get(intent.lp_id) {
                Some(h) => h.tx.clone(),
                // The member is on the panel (it is quoting) but the operator has given
                // it no order endpoint. Stated, never skipped.
                None => {
                    return RouteAnswer::Unroutable {
                        reason: NO_ENDPOINT_REASON.to_owned(),
                    };
                }
            }
        };

        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        let job = Job {
            intent: OwnedIntent::from(intent),
            reply: reply_tx,
        };
        if tx.try_send(job).is_err() {
            return RouteAnswer::Unroutable {
                reason: MEMBER_SATURATED_REASON.to_owned(),
            };
        }

        // The actor applies the venue deadline and reports its own measured wait; this
        // budget is only a backstop against an actor that never replies at all.
        let budget = self.venue_timeout + HANDOFF_GRACE;
        let waited = Instant::now();
        let answer = block_for(|| reply_rx.recv_timeout(budget));
        answer.unwrap_or_else(|_| RouteAnswer::Expired {
            waited_nanos: u64::try_from(waited.elapsed().as_nanos()).unwrap_or(u64::MAX),
        })
    }
}

/// Run `wait` on the current thread, releasing the caller's Tokio worker for the
/// duration when there is one to release.
///
/// See the module docs: this is a courtesy to the caller's runtime, not a correctness
/// requirement. The work being waited on runs on the router's own threads either way,
/// so taking the wait directly (the current-thread-runtime and non-async cases) cannot
/// deadlock.
fn block_for<T>(wait: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(wait)
        }
        _ => wait(),
    }
}

/// One member's session-owning task: serialise its orders over a single FIX session,
/// establishing it lazily and re-establishing it once after a transport failure.
async fn member_actor(endpoint: OrderEndpoint, mut rx: mpsc::Receiver<Job>, timeout: Duration) {
    let mut session: Option<LpSession> = None;
    while let Some(job) = rx.recv().await {
        let answer = work_order(&mut session, &endpoint, &job.intent, timeout).await;
        // A caller that gave up (its own budget elapsed) has dropped the receiver. That
        // is not an error here — the attempt is already recorded as expired.
        let _ = job.reply.send(answer);
    }
    tracing::info!(member = %endpoint.member_id, "street-order member actor stopped");
}

/// Work one order, re-establishing the session at most once on a transport failure.
async fn work_order(
    session: &mut Option<LpSession>,
    endpoint: &OrderEndpoint,
    intent: &OwnedIntent,
    timeout: Duration,
) -> RouteAnswer {
    for attempt in 0..2 {
        if session.is_none() {
            match connect(endpoint).await {
                Ok(s) => *session = Some(s),
                Err(e) => {
                    tracing::warn!(
                        member = %endpoint.member_id,
                        addr = %endpoint.addr,
                        error = %e,
                        "street-order session could not be established"
                    );
                    return RouteAnswer::Unroutable {
                        reason: NO_SESSION_REASON.to_owned(),
                    };
                }
            }
        }
        let live = session.as_mut().expect("session established above");
        match send_and_await(live, endpoint, intent, timeout).await {
            Ok(answer) => return answer,
            Err(e) => {
                // The socket died mid-order. Drop it and try once on a fresh session —
                // a counterparty restart should cost one order's latency, not an outage.
                tracing::warn!(
                    member = %endpoint.member_id,
                    error = %e,
                    retrying = attempt == 0,
                    "street-order transport failed"
                );
                *session = None;
            }
        }
    }
    RouteAnswer::Unroutable {
        reason: NO_SESSION_REASON.to_owned(),
    }
}

/// A logged-on outbound session to one member.
struct LpSession {
    session: Session<InMemoryStore>,
    reader: FrameReader<ReadHalf<TcpStream>>,
    write: WriteHalf<TcpStream>,
    cl_ord_seq: u64,
}

/// Connect and log on, returning once the session FSM reports `Active`.
async fn connect(endpoint: &OrderEndpoint) -> std::io::Result<LpSession> {
    let stream = tokio::time::timeout(SESSION_SETUP_TIMEOUT, TcpStream::connect(&endpoint.addr))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timed out"))??;
    // An order is a single small frame whose latency IS the measurement — Nagle would
    // add up to a round trip of delay to every one of them.
    stream.set_nodelay(true)?;
    let (read_half, mut write) = tokio::io::split(stream);
    let mut reader = FrameReader::new(read_half);
    let mut session = Session::new(
        SessionConfig {
            sender: endpoint.sender_comp_id.as_bytes().to_vec(),
            target: endpoint.target_comp_id.as_bytes().to_vec(),
            heart_bt_int: HEARTBEAT_SECS,
            role: Role::Initiator,
        },
        InMemoryStore::new(),
    );
    let sending_time = now_stamp();
    let logon = session.start_logon(&sending_time, false);
    write_frame(&mut write, &logon).await?;

    let deadline = tokio::time::Instant::now() + SESSION_SETUP_TIMEOUT;
    loop {
        let raw = match tokio::time::timeout_at(deadline, reader.next_frame()).await {
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "logon was not mirrored in time",
                ));
            }
            Ok(Ok(Some(bytes))) => bytes,
            Ok(Ok(None)) => {
                return Err(std::io::Error::other("peer closed before logon completed"));
            }
            Ok(Err(e)) => return Err(e),
        };
        let action = session
            .on_inbound(&raw, &sending_time)
            .map_err(|_| std::io::Error::other("session protocol error during logon"))?;
        for frame in &action.outbound {
            write_frame(&mut write, frame).await?;
        }
        if session.state() == SessionState::Active {
            return Ok(LpSession {
                session,
                reader,
                write,
                cl_ord_seq: 0,
            });
        }
    }
}

/// Send one `NewOrderSingle(D)` and collect the matching `ExecutionReport(8)`.
///
/// `Ok` carries the venue's answer — including [`RouteAnswer::Expired`] when it never
/// answered, which is a fact about the venue, not a failure of this function. `Err` is
/// reserved for a genuine transport fault, which the caller retries on a fresh session.
async fn send_and_await(
    live: &mut LpSession,
    endpoint: &OrderEndpoint,
    intent: &OwnedIntent,
    timeout: Duration,
) -> std::io::Result<RouteAnswer> {
    live.cl_ord_seq += 1;
    let cl_ord_id = format!("CN-{}-{}", endpoint.member_id, live.cl_ord_seq);
    let sending_time = now_stamp();
    let transact_time = sending_time.clone();
    let symbol = intent.instrument.as_bytes().to_vec();

    let frame = live.session.send_app(&sending_time, |hdr, enc| {
        let params = MarketOrderParams {
            cl_ord_id: cl_ord_id.as_bytes(),
            symbol: &symbol,
            // A hedge addresses the member by SYMBOL: it is dealing on the member's
            // streamed panel line, which carries no per-quote token to lift.
            quote_id: b"",
            security_type: b"",
            side: intent.side,
            qty: intent.quantity,
            price: intent.limit_price,
            ord_type: intent.ord_type,
            tif: Some(intent.time_in_force),
            transact_time: &transact_time,
        };
        messages::build_new_order_by_symbol(hdr, &params, enc)
    });

    let sent_at = Instant::now();
    write_frame(&mut live.write, &frame).await?;
    tracing::debug!(
        member = %endpoint.member_id,
        cl_ord_id = %cl_ord_id,
        instrument = %intent.instrument,
        qty = intent.quantity,
        price = intent.limit_price,
        "NewOrderSingle sent"
    );

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let raw = match tokio::time::timeout_at(deadline, live.reader.next_frame()).await {
            Err(_) => {
                return Ok(RouteAnswer::Expired {
                    waited_nanos: elapsed_nanos(sent_at),
                });
            }
            Ok(Ok(Some(bytes))) => bytes,
            Ok(Ok(None)) => return Err(std::io::Error::other("venue closed the session")),
            Ok(Err(e)) => return Err(e),
        };
        let action = live
            .session
            .on_inbound(&raw, &sending_time)
            .map_err(|_| std::io::Error::other("session protocol error"))?;
        for out in &action.outbound {
            write_frame(&mut live.write, out).await?;
        }
        if action.deliver != Some(MsgType::ExecutionReport) {
            continue;
        }
        let Ok(parsed) = FrameCursor::parse(&raw) else {
            continue;
        };
        let view = ExecReportView::new(parsed);
        // Only OUR order's report ends the wait. A report for a different `ClOrdID`
        // belongs to another order on the same session and must not be mistaken for
        // this one's answer.
        if view.cl_ord_id() != Some(cl_ord_id.as_bytes()) {
            continue;
        }
        return Ok(classify_report(&view, elapsed_nanos(sent_at)));
    }
}

/// Turn one `ExecutionReport(8)` into the answer the hedge seam records.
///
/// The three terminal states a quote-driven venue reaches are kept apart on purpose:
/// `OrdStatus=8` means the venue **would not** accept the order, `OrdStatus=4` means it
/// **could not** satisfy one it did accept, and a partial means it satisfied part. The
/// one refinement on top is the last look: a decline whose reason is
/// [`NOT_MARKETABLE_CODE`] means the venue would not trade at the price it was itself
/// showing, which — because we always price the order at the member's own ranked level
/// — is a pull, not a refusal to deal with us.
#[must_use]
fn classify_report(view: &ExecReportView<'_>, latency_nanos: u64) -> RouteAnswer {
    let text = view
        .text()
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .unwrap_or_default();
    // The venue's `Text(58)` leads with a machine-readable code, then a human detail.
    let code = text
        .split(':')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    let reason = (!code.is_empty()).then(|| code.clone());
    // Prefer the round-trip-exact premium when the venue stamped one, so a booked hedge
    // reconciles to the venue's own arithmetic to the last bit.
    let price = view.last_px_exact().or_else(|| view.last_px());
    let filled = view.last_qty().unwrap_or_default();

    match view.ord_status() {
        Some(EXEC_FILLED | ORD_STATUS_PARTIALLY_FILLED)
            if filled > 0.0 && price.is_some_and(f64::is_finite) =>
        {
            RouteAnswer::Traded(RoutedFill {
                filled,
                price: price.unwrap_or_default(),
                reason,
                latency_nanos,
            })
        }
        // A "filled" report with nothing on it is not a fill. Booking one would invent a
        // trade out of a malformed message.
        Some(EXEC_FILLED | ORD_STATUS_PARTIALLY_FILLED) => RouteAnswer::Cancelled {
            reason: reason.unwrap_or_else(|| "fill_report_carried_no_quantity".to_owned()),
            latency_nanos,
        },
        Some(ORD_STATUS_CANCELED) if code == NOT_MARKETABLE_CODE => RouteAnswer::LastLookPulled {
            reason: code,
            latency_nanos,
        },
        Some(ORD_STATUS_CANCELED) => RouteAnswer::Cancelled {
            reason: reason.unwrap_or_else(|| "venue_cancelled".to_owned()),
            latency_nanos,
        },
        Some(EXEC_REJECTED) => RouteAnswer::Rejected {
            reason: reason.unwrap_or_else(|| "venue_rejected".to_owned()),
            latency_nanos,
        },
        // An `OrdStatus(39)` outside the terminal set is a venue we do not understand.
        // Recording it as a rejection with that stated reason is honest; guessing which
        // terminal state it meant would not be.
        other => RouteAnswer::Rejected {
            reason: format!(
                "unrecognised_ord_status_{}",
                other.map_or_else(|| "absent".to_owned(), |b| (b as char).to_string())
            ),
            latency_nanos,
        },
    }
}

/// Elapsed nanoseconds since `t0`, saturating.
fn elapsed_nanos(t0: Instant) -> u64 {
    u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// A `SendingTime(52)` stamp from the wall clock, in the estate's one FIX timestamp
/// format (`YYYYMMDD-HH:MM:SS.sss`).
fn now_stamp() -> Vec<u8> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
    crate::services::fix::utc_timestamp(nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::fix_connections::AcceptorKind;

    fn conn(id: &str, endpoint: Option<&str>, enabled: bool) -> FixConnectionDef {
        FixConnectionDef {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: AcceptorKind::FixedIncomeStream,
            bind_addr: "127.0.0.1:0".to_owned(),
            sender_comp_id: "CELNET".to_owned(),
            target_comp_id: id.to_owned(),
            enabled,
            desk: "rates".to_owned(),
            order_endpoint: endpoint.map(str::to_owned),
        }
    }

    /// Only an ENABLED connection carrying a non-blank endpoint is routable. A blank
    /// string is not an address and must not become one.
    #[test]
    fn only_enabled_connections_with_an_address_are_routable() {
        let defs = vec![
            conn("jpm-sim", Some("127.0.0.1:5704"), true),
            conn("citigroup-sim", None, true),
            conn("traderweb-sim", Some("   "), true),
            conn("marketaccess-sim", Some("127.0.0.1:5701"), false),
        ];
        let eps = order_endpoints(&defs);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].member_id, "jpm-sim");
        assert_eq!(eps[0].addr, "127.0.0.1:5704");
        assert_eq!(eps[0].sender_comp_id, "CELNET");
        assert_eq!(eps[0].target_comp_id, "jpm-sim");
    }

    /// A member with no configured endpoint is UNROUTABLE with that stated reason — it
    /// is never silently skipped, and it is never reported as a market outcome.
    #[test]
    fn a_member_without_an_endpoint_is_unroutable_with_a_stated_reason() {
        let router = FixStreetRouter::new();
        let answer = router.route(&StreetOrderIntent {
            lp_id: "nobody-sim",
            instrument: "912810TZ1",
            side: celnet_analytics::StreetSide::Sell,
            quantity: 1_000_000.0,
            limit_price: 99.9,
            ord_type: messages::ord_type::LIMIT,
            time_in_force: messages::time_in_force::IMMEDIATE_OR_CANCEL,
        });
        assert_eq!(
            answer,
            RouteAnswer::Unroutable {
                reason: NO_ENDPOINT_REASON.to_owned()
            }
        );
        assert!(router.routable_members().is_empty());
    }

    /// A configured endpoint pointing at nothing yields `session_unavailable` — a
    /// stated routing failure, distinct from a counterparty refusing us.
    #[test]
    fn an_endpoint_that_answers_nothing_is_a_stated_session_failure() {
        let router = FixStreetRouter::with_timeout(Duration::from_millis(300));
        // Port 1 on loopback refuses instantly on every supported platform.
        router.set_endpoints(vec![OrderEndpoint {
            member_id: "dead-sim".to_owned(),
            addr: "127.0.0.1:1".to_owned(),
            sender_comp_id: "CELNET".to_owned(),
            target_comp_id: "dead-sim".to_owned(),
        }]);
        assert_eq!(router.routable_members(), vec!["dead-sim".to_owned()]);
        let answer = router.route(&StreetOrderIntent {
            lp_id: "dead-sim",
            instrument: "912810TZ1",
            side: celnet_analytics::StreetSide::Sell,
            quantity: 1_000.0,
            limit_price: 99.9,
            ord_type: messages::ord_type::LIMIT,
            time_in_force: messages::time_in_force::IMMEDIATE_OR_CANCEL,
        });
        assert_eq!(
            answer,
            RouteAnswer::Unroutable {
                reason: NO_SESSION_REASON.to_owned()
            }
        );
    }

    /// Refreshing the registry keeps an unchanged member's actor (and therefore its
    /// live session) and drops one that disappeared.
    #[test]
    fn refreshing_endpoints_preserves_unchanged_members() {
        let router = FixStreetRouter::new();
        let a = OrderEndpoint {
            member_id: "a-sim".to_owned(),
            addr: "127.0.0.1:5701".to_owned(),
            sender_comp_id: "CELNET".to_owned(),
            target_comp_id: "a-sim".to_owned(),
        };
        let b = OrderEndpoint {
            member_id: "b-sim".to_owned(),
            addr: "127.0.0.1:5702".to_owned(),
            sender_comp_id: "CELNET".to_owned(),
            target_comp_id: "b-sim".to_owned(),
        };
        router.set_endpoints(vec![a.clone(), b]);
        assert_eq!(router.routable_members(), vec!["a-sim", "b-sim"]);
        router.set_endpoints(vec![a]);
        assert_eq!(router.routable_members(), vec!["a-sim"]);
    }

    // --- report classification -------------------------------------------------

    fn report(ord_status: u8, qty: f64, px: f64, text: Option<&str>) -> Vec<u8> {
        let mut enc = celnet_fix::framing::FrameEncoder::new();
        let hdr = messages::Header {
            sender: b"jpm-sim",
            target: b"CELNET",
            seq_num: 3,
            sending_time: b"20260814-09:00:00.000",
        };
        let params = messages::ExecReportParams {
            order_id: b"O-1",
            exec_id: b"E-1",
            cl_ord_id: b"CN-jpm-sim-1",
            exec_type: ord_status,
            ord_status,
            symbol: b"912810TZ1",
            side: celnet_fix::dialect_fx::SIDE_SELL,
            last_qty: qty,
            last_px: px,
            multileg_type: None,
            text: text.map(str::as_bytes),
        };
        messages::build_execution_report(&hdr, &params, &mut enc)
    }

    fn classify(raw: &[u8]) -> RouteAnswer {
        let frame = FrameCursor::parse(raw).expect("report parses");
        classify_report(&ExecReportView::new(frame), 4_321)
    }

    /// The four terminal states a real venue reaches map to four DISTINCT answers, and
    /// each carries the venue's own reason code.
    #[test]
    fn the_terminal_states_stay_distinct() {
        let filled = classify(&report(EXEC_FILLED, 1_000_000.0, 99.9, None));
        match filled {
            RouteAnswer::Traded(f) => {
                assert_eq!(f.filled, 1_000_000.0);
                assert!((f.price - 99.9).abs() < 1e-9);
                assert_eq!(f.reason, None, "a clean fill needs no qualifier");
                assert_eq!(f.latency_nanos, 4_321);
            }
            other => panic!("expected a fill, got {other:?}"),
        }

        let partial = classify(&report(
            ORD_STATUS_PARTIALLY_FILLED,
            250_000.0,
            99.9,
            Some("IOC_DEPTH_EXHAUSTED: filled at the touch, 750000 cancelled"),
        ));
        match partial {
            RouteAnswer::Traded(f) => {
                assert_eq!(f.filled, 250_000.0);
                assert_eq!(f.reason.as_deref(), Some("IOC_DEPTH_EXHAUSTED"));
            }
            other => panic!("expected a partial fill, got {other:?}"),
        }

        assert_eq!(
            classify(&report(
                ORD_STATUS_CANCELED,
                0.0,
                0.0,
                Some("FOK_UNFILLABLE: the full quantity is not available")
            )),
            RouteAnswer::Cancelled {
                reason: "FOK_UNFILLABLE".to_owned(),
                latency_nanos: 4_321
            }
        );

        assert_eq!(
            classify(&report(
                EXEC_REJECTED,
                0.0,
                0.0,
                Some("NOT_A_WHOLE_LOT: this instrument trades in whole lots only")
            )),
            RouteAnswer::Rejected {
                reason: "NOT_A_WHOLE_LOT".to_owned(),
                latency_nanos: 4_321
            }
        );
    }

    /// A venue that will not trade at the price it was itself showing has pulled at
    /// last look — materially different from refusing to deal with us at all.
    #[test]
    fn a_decline_at_the_shown_price_is_a_last_look_pull() {
        assert_eq!(
            classify(&report(
                ORD_STATUS_CANCELED,
                0.0,
                0.0,
                Some("NOT_MARKETABLE: the limit price is away from the quoted market")
            )),
            RouteAnswer::LastLookPulled {
                reason: NOT_MARKETABLE_CODE.to_owned(),
                latency_nanos: 4_321
            }
        );
    }

    /// A "filled" report carrying no quantity is NOT a fill. Booking one would invent
    /// a trade out of a malformed message.
    #[test]
    fn a_fill_report_with_no_quantity_is_never_booked_as_a_fill() {
        assert!(matches!(
            classify(&report(EXEC_FILLED, 0.0, 0.0, None)),
            RouteAnswer::Cancelled { .. }
        ));
    }
}
