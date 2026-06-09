//! The async⇄core bridge: a single, ring-owning conduit between the massively
//! concurrent async edge and the single-threaded, core-pinned
//! [`celnet_engine::PricingCore`] (`docs/ARCHITECTURE.md` §3).
//!
//! # Why a bridge is needed
//!
//! The engine's hot path is a **single-producer / single-consumer** contract:
//! one thread owns the [`celnet_engine::RequestRing`] producer, one owns the
//! [`celnet_engine::ResponseRing`] consumer, and the pricing core busy-polls in
//! between. The gRPC and WebSocket surfaces, by contrast, serve an unbounded
//! number of concurrent tasks. [`CoreLink`] reconciles the two:
//!
//! * a **dedicated OS thread** runs the [`celnet_engine::PricingCore`] busy-poll
//!   loop (optionally pinned to an isolated core), draining the request ring,
//!   pricing each request, and pushing responses to the response ring — it is the
//!   sole owner of the core and is never blocked by the async runtime;
//! * a **submitter** task owns the request-ring producer and the correlation
//!   map: every async caller hands it a [`celnet_engine::PriceRequest`] plus a
//!   `oneshot` reply slot, which it registers under the request's id and pushes
//!   onto the ring;
//! * a **response router** dedicated OS thread owns the response-ring consumer:
//!   it pops each `Copy` [`celnet_engine::PriceResponse`] and forwards it into an
//!   async-wakeable channel (see the next section for why this is an OS thread and
//!   not a tokio task);
//! * a **dispatcher** task receives each routed response and resolves the matching
//!   reply slot by id.
//!
//! The hot path therefore stays exactly SPSC; the async fan-in/fan-out lives
//! entirely on the edge side of the rings. The core is never blocked: a full
//! response ring is back-pressure the router absorbs, never a stall.
//!
//! # Why response routing runs on an OS thread, not a tokio task
//!
//! `rtrb` exposes only non-blocking `pop`; there is no async, parking variant.
//! An earlier design polled the response ring from a tokio task that re-armed
//! itself with `yield_now().await` whenever the ring was momentarily empty. On
//! tokio's **current-thread** scheduler that is fatal: a task that is *always*
//! ready keeps the run queue non-empty, so the runtime never parks, and a runtime
//! that never parks never advances the time driver or services the I/O reactor.
//! Every `tokio::time::{sleep, timeout, interval}` and every socket accept then
//! stalls — the whole edge wedges even though the core is healthy.
//!
//! The fix keeps the two-tier design intact but moves the *only* unavoidable
//! busy-wait — draining a non-blocking SPSC ring — onto a **dedicated OS thread**
//! (the response router), entirely off the async runtime. The router pops each
//! `Copy` [`PriceResponse`] and forwards it into a `tokio::sync::mpsc` whose
//! `send` from a non-async thread wakes the receiving task correctly. A single
//! async dispatcher then `recv().await`s (parking cleanly when idle) and resolves
//! each waiter's `oneshot` by id. No tokio worker ever spins, so the runtime
//! parks when idle and its timer / reactor always make progress.
//!
//! # Control plane vs hot path
//!
//! Vanilla pricing flows over the rings (the hot path). Market-state publishes
//! and the read-only surface / exotic queries flow over a separate **control
//! channel** to the core thread, which owns the live
//! [`celnet_engine::MarketState`] via the core's [`celnet_engine::StateHandle`].
//! Keeping these off the hot ring means a recalibration or a surface read never
//! competes with a price for ring slots.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

use celnet_engine::rt::{request_ring, response_ring};
use celnet_engine::{MarketState, PriceRequest, PriceResponse, PricingCore};
use celnet_exotics::{BarrierStyle, SingleBarrier, single_barrier_price};
use celnet_types::{OptionType, VanillaInputs};
use tokio::sync::{mpsc, oneshot};

/// The number of requests the core prices per busy-poll iteration before it
/// services the control channel and yields the CPU once if idle. Bounded work
/// per poll keeps the loop responsive to control-plane commands and to shutdown.
const DRAIN_BUDGET: usize = 256;

/// A read-only query for the calibrated surface vol at a `(strike, tenor)`.
///
/// Evaluated on the core thread against the live [`MarketState`] so it sees
/// exactly the smile the hot path prices against.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceQuery {
    /// Strike to evaluate the smile at (quote per 1 unit of base).
    pub strike: f64,
    /// Tenor as a year fraction (currently the published slice's `t`).
    pub tenor_years: f64,
}

/// The result of a [`SurfaceQuery`]: the calibrated Black vol and the outright
/// forward implied by the live market state.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceVol {
    /// Calibrated Black vol at the queried point (absolute, 0.10 = 10 vol).
    pub vol: f64,
    /// Outright forward at the slice tenor.
    pub forward: f64,
}

/// A read-only projection of the live [`MarketState`] for the async edge: the
/// diffusion state plus the ATM Black vol (the smile vol at the forward).
///
/// The RFQ / scenario services price client-supplied instruments against the
/// maker's *own* current market; this snapshot is how the edge reads that market
/// off the core thread without touching the hot pricing ring.
#[derive(Debug, Clone, Copy)]
pub struct MarketSnapshot {
    /// Spot FX rate (quote per 1 unit of base).
    pub spot: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub r_for: f64,
    /// Vol-time to expiry of the published slice (years).
    pub t: f64,
    /// The outright forward at the slice tenor.
    pub forward: f64,
    /// The ATM Black vol (the smile vol at the forward).
    pub atm_vol: f64,
}

/// The barrier topology of a single-barrier exotic: the direction the barrier
/// sits relative to spot and whether it knocks the option in or out.
///
/// A vendor/research-neutral, purpose-named enum that maps both to the proto
/// `BarrierKind` on the wire and to the [`celnet_exotics::BarrierKind`] the
/// closed form consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierTopology {
    /// Knock-out, barrier below spot.
    DownAndOut,
    /// Knock-out, barrier above spot.
    UpAndOut,
    /// Knock-in, barrier below spot.
    DownAndIn,
    /// Knock-in, barrier above spot.
    UpAndIn,
}

impl BarrierTopology {
    /// `true` if the barrier sits above spot.
    const fn up(self) -> bool {
        matches!(self, BarrierTopology::UpAndOut | BarrierTopology::UpAndIn)
    }

    /// The knock style (in/out).
    const fn style(self) -> BarrierStyle {
        match self {
            BarrierTopology::DownAndOut | BarrierTopology::UpAndOut => BarrierStyle::KnockOut,
            BarrierTopology::DownAndIn | BarrierTopology::UpAndIn => BarrierStyle::KnockIn,
        }
    }
}

/// A market observable a time-series feed streams, evaluated against the live
/// [`MarketState`] on the core thread. Purpose-named and vendor-neutral; maps both
/// to the wire `MarketObservable` and to the derivation off the live smile/state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observable {
    /// At-the-money-forward Black vol for the live slice (absolute, 0.10 = 10v).
    AtmVol,
    /// Spot FX rate (quote per 1 base).
    Spot,
    /// Risk reversal (call vol − put vol) at the given signed delta wing — vol.
    RiskReversal {
        /// The delta wing magnitude (e.g. 0.25 or 0.10) the RR is measured at.
        delta: f64,
    },
    /// Butterfly (½(call+put) − ATM) at the given signed delta wing — vol.
    Butterfly {
        /// The delta wing magnitude (e.g. 0.25 or 0.10) the BF is measured at.
        delta: f64,
    },
    /// Outright forward for the live slice (quote per 1 base).
    Forward,
}

/// A read-only market-observable query against the live [`MarketState`].
///
/// Evaluated on the core thread so the value is derived from exactly the state the
/// hot path prices against — never a fabricated or independently-recomputed
/// number. The result is the observable's value in its natural unit, or `None` when
/// it cannot be derived from the live state (e.g. a wing strike inversion fails for
/// a degenerate delta).
#[derive(Debug, Clone, Copy)]
pub struct ObservableQuery {
    /// The observable to evaluate.
    pub observable: Observable,
}

/// A read-only single-barrier exotic pricing query.
///
/// The vanilla inputs and barrier topology come from the request; the closed-form
/// continuous-monitoring barrier price is computed on the core thread via
/// [`celnet_exotics::single_barrier_price`].
#[derive(Debug, Clone, Copy)]
pub struct ExoticQuery {
    /// Call or put of the underlying vanilla.
    pub option_type: OptionType,
    /// Garman-Kohlhagen inputs for the underlying (carries the strike).
    pub inputs: VanillaInputs,
    /// Barrier topology (knock-in/out, up/down).
    pub topology: BarrierTopology,
    /// Barrier level (quote per 1 unit of base).
    pub barrier: f64,
    /// Rebate paid on the terminating event (domestic).
    pub rebate: f64,
}

/// A failure routing a request to or a response from the pricing core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreLinkError {
    /// The pricing core / bridge has shut down and can no longer serve requests.
    CoreUnavailable,
}

impl core::fmt::Display for CoreLinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CoreLinkError::CoreUnavailable => write!(f, "pricing core is unavailable"),
        }
    }
}

impl std::error::Error for CoreLinkError {}

/// A control-plane command sent to the core thread (off the hot ring).
enum Control {
    /// Republish the live market state (a tick or a recalibration / hot swap).
    ///
    /// The optional `oneshot` is fired once the new state has been applied, so a
    /// caller can order a *subsequent* hot-path price against the just-published
    /// state (the core applies pending publishes before draining the price ring,
    /// and the ack guarantees the publish was enqueued-and-applied first).
    Publish(MarketState, Option<oneshot::Sender<()>>),
    /// Evaluate the surface vol at a point, replying on the `oneshot`.
    Surface(SurfaceQuery, oneshot::Sender<SurfaceVol>),
    /// Read a projection of the live market state, replying on the `oneshot`.
    Snapshot(oneshot::Sender<MarketSnapshot>),
    /// Price a single-barrier exotic, replying on the `oneshot`.
    Exotic(ExoticQuery, oneshot::Sender<f64>),
    /// Evaluate a market observable against the live state, replying on the
    /// `oneshot` (`None` when the observable cannot be derived).
    Observe(ObservableQuery, oneshot::Sender<Option<f64>>),
    /// Stop the busy-poll loop and join the thread.
    Stop,
}

/// A submission to the ring submitter: a hot-path price request plus its reply
/// slot. The submitter registers the slot under the request id before pushing.
struct Submission {
    request: PriceRequest,
    reply: oneshot::Sender<PriceResponse>,
}

/// The async-side handle to the running pricing core.
///
/// Cheap to clone behind an [`Arc`]; every method is non-blocking and async. The
/// hot vanilla path goes through [`CoreLink::price`]; market ticks and the
/// read-only surface / exotic paths go through the control channel.
#[derive(Debug)]
pub struct CoreLink {
    /// Monotonic generator for hot-path request correlation ids.
    next_id: AtomicU64,
    /// Channel into the submitter task (hot path).
    submit_tx: mpsc::UnboundedSender<Submission>,
    /// Channel into the core thread (control plane).
    control_tx: mpsc::UnboundedSender<Control>,
    /// Shutdown flag observed by the dedicated OS threads (core + router) so they
    /// exit their non-blocking poll loops promptly and the process can terminate.
    shutdown: Arc<AtomicBool>,
    /// The dedicated core + response-router thread join handles (taken on
    /// shutdown). Both are joined exactly once so the test process always exits.
    threads: std::sync::Mutex<Option<(JoinHandle<()>, JoinHandle<()>)>>,
}

impl CoreLink {
    /// Spawn the pricing core on a dedicated thread and wire up the async bridge.
    ///
    /// `initial` is the market state the core starts from. `pin_core` optionally
    /// pins the busy-poll thread to a physical core index for latency isolation
    /// (pinning is best-effort and never a correctness requirement).
    ///
    /// Spawns four things: the OS-thread busy-poll core (request ring → price →
    /// response ring), a dedicated OS-thread response router (response ring → an
    /// async-wakeable channel), the async submitter task (owns the request-ring
    /// producer + correlation map), and the async dispatcher task (resolves each
    /// waiter's `oneshot` by id). Returns an [`Arc`] handle.
    ///
    /// Neither async task busy-spins: both park on a tokio channel when idle, so
    /// the runtime parks and its timer / I/O reactor always make progress (see the
    /// module docs for why this matters on the current-thread scheduler). The only
    /// non-blocking poll of an `rtrb` ring lives on the dedicated router OS thread,
    /// off the runtime entirely.
    #[must_use]
    pub fn start(initial: MarketState, pin_core: Option<usize>) -> Arc<Self> {
        // The hot-path SPSC rings (edge ⇄ core).
        let (mut req_tx, mut req_rx) = request_ring();
        let (mut resp_tx, mut resp_rx) = response_ring();

        // A single shutdown flag the two dedicated OS threads observe so their
        // non-blocking poll loops exit promptly on `stop()` / `Drop`.
        let shutdown = Arc::new(AtomicBool::new(false));

        // Control plane (async edge → core thread).
        let (control_tx, mut control_rx) = mpsc::unbounded_channel::<Control>();
        // A std channel mirrors the control commands onto the busy-poll thread,
        // which is *not* async and cannot await a tokio receiver. The async side
        // forwards each command across this boundary.
        let (core_ctrl_tx, core_ctrl_rx) = std::sync::mpsc::channel::<Control>();

        // Bridge task: move control commands from the async receiver to the std
        // channel the core thread polls. Runs until the control sender is gone.
        tokio::spawn(async move {
            while let Some(cmd) = control_rx.recv().await {
                let stop = matches!(cmd, Control::Stop);
                if core_ctrl_tx.send(cmd).is_err() {
                    break;
                }
                if stop {
                    break;
                }
            }
        });

        // The dedicated busy-poll pricing core thread.
        let core_shutdown = Arc::clone(&shutdown);
        let core_thread = std::thread::Builder::new()
            .name("celnet-pricing-core".to_owned())
            .spawn(move || {
                if let Some(idx) = pin_core {
                    let _ = celnet_engine::pin_current_thread_to_core(idx);
                }
                let mut core = PricingCore::new(initial);
                run_core(
                    &mut core,
                    &mut req_rx,
                    &mut resp_tx,
                    &core_ctrl_rx,
                    &core_shutdown,
                );
                // Belt-and-braces: once the core loop has returned (Stop / flag),
                // publish the shutdown flag so the router winds down even if it
                // was the control path, not the flag, that ended the loop.
                core_shutdown.store(true, Ordering::Release);
            })
            .expect("spawning the pricing-core thread must succeed");

        // The pending correlation map is shared between the submitter (insert) and
        // the dispatcher (remove). A std Mutex is fine: each critical section is a
        // single map op and no `.await` is ever held across the lock.
        let pending: Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<PriceResponse>>>> =
            Arc::new(std::sync::Mutex::new(HashMap::new()));

        // The dedicated response-router OS thread owns the response-ring consumer.
        // It pops each `Copy` response and forwards it into a tokio mpsc whose
        // `send` from this non-async thread correctly wakes the async dispatcher.
        // Running this off the runtime is what keeps the current-thread scheduler
        // free to park (and thus drive its timer / reactor) — an async task polling
        // the ring would never let the runtime go idle.
        let (routed_tx, mut routed_rx) = mpsc::unbounded_channel::<PriceResponse>();
        let router_shutdown = Arc::clone(&shutdown);
        let router_thread = std::thread::Builder::new()
            .name("celnet-resp-router".to_owned())
            .spawn(move || {
                loop {
                    match resp_rx.pop() {
                        Ok(resp) => {
                            // Receiver dropped ⇒ the bridge is gone; stop.
                            if routed_tx.send(resp).is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            // Ring momentarily empty. Honour shutdown promptly;
                            // otherwise park briefly so this thread never hot-spins
                            // a core while waiting for the next priced response.
                            if router_shutdown.load(Ordering::Acquire) {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_micros(50));
                        }
                    }
                }
            })
            .expect("spawning the response-router thread must succeed");

        // The submitter task owns the request-ring producer and the correlation
        // map. It parks on `submit_rx.recv()` when idle (no spin).
        let (submit_tx, mut submit_rx) = mpsc::unbounded_channel::<Submission>();
        let submit_pending = Arc::clone(&pending);
        let submit_shutdown = Arc::clone(&shutdown);
        tokio::spawn(async move {
            while let Some(sub) = submit_rx.recv().await {
                let id = sub.request.request_id;
                {
                    let mut map = submit_pending.lock().expect("pending map not poisoned");
                    map.insert(id, sub.reply);
                }
                // Push onto the hot ring. A full ring is back-pressure, never an
                // error: wait briefly (the core drains continuously) and retry. The
                // wait is a real timer-backed `sleep`, not a `yield_now`, so it
                // parks the runtime instead of monopolising the current-thread
                // scheduler — and it bails out if the core is shutting down so a
                // never-draining ring cannot wedge the submitter.
                let mut req = sub.request;
                loop {
                    match req_tx.push(req) {
                        Ok(()) => break,
                        Err(rtrb::PushError::Full(returned)) => {
                            if submit_shutdown.load(Ordering::Acquire) {
                                // Core is going away; resolve nothing (the waiter
                                // sees `CoreUnavailable` when its oneshot drops).
                                let mut map =
                                    submit_pending.lock().expect("pending map not poisoned");
                                map.remove(&id);
                                break;
                            }
                            req = returned;
                            tokio::time::sleep(std::time::Duration::from_micros(50)).await;
                        }
                    }
                }
            }
        });

        // The dispatcher resolves each routed response to its waiter by id. It
        // parks on `routed_rx.recv()` when idle — no busy-poll, no `yield_now`.
        let dispatch_pending = Arc::clone(&pending);
        tokio::spawn(async move {
            while let Some(resp) = routed_rx.recv().await {
                let slot = {
                    let mut map = dispatch_pending.lock().expect("pending map not poisoned");
                    map.remove(&resp.request_id)
                };
                if let Some(tx) = slot {
                    // The waiter may have dropped (client cancelled); ignore.
                    let _ = tx.send(resp);
                }
            }
            // The router thread closed the channel: the bridge is shutting down.
            // Drop every still-pending waiter so each `price().await` resolves to
            // `CoreUnavailable` rather than hanging forever.
            dispatch_pending
                .lock()
                .expect("pending map not poisoned")
                .clear();
        });

        Arc::new(Self {
            next_id: AtomicU64::new(1),
            submit_tx,
            control_tx,
            shutdown,
            threads: std::sync::Mutex::new(Some((core_thread, router_thread))),
        })
    }

    /// Allocate the next hot-path correlation id.
    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Price a vanilla option on the hot path and await its full Greek set.
    ///
    /// Submits the `(option_type, strike)` over the request ring (the core forms
    /// the Garman-Kohlhagen inputs from the live market state and the smile vol),
    /// then awaits the correlated [`PriceResponse`].
    ///
    /// # Errors
    ///
    /// [`CoreLinkError::CoreUnavailable`] if the core / bridge has shut down.
    pub async fn price(
        &self,
        option_type: OptionType,
        strike: f64,
    ) -> Result<PriceResponse, CoreLinkError> {
        let id = self.next_request_id();
        let (reply, rx) = oneshot::channel();
        let sub = Submission {
            request: PriceRequest::new(id, option_type, strike),
            reply,
        };
        self.submit_tx
            .send(sub)
            .map_err(|_| CoreLinkError::CoreUnavailable)?;
        rx.await.map_err(|_| CoreLinkError::CoreUnavailable)
    }

    /// Publish a new market state to the core (a tick or a recalibration).
    ///
    /// Lock-free on the core side (the engine hot-swaps the state behind
    /// `arc-swap`); this just enqueues the new state on the control channel.
    ///
    /// # Errors
    ///
    /// [`CoreLinkError::CoreUnavailable`] if the core has shut down.
    pub fn publish(&self, state: MarketState) -> Result<(), CoreLinkError> {
        self.control_tx
            .send(Control::Publish(state, None))
            .map_err(|_| CoreLinkError::CoreUnavailable)
    }

    /// Publish a new market state and await confirmation that the core has
    /// applied it.
    ///
    /// Use this when a *subsequent* hot-path price must be computed against the
    /// just-published state: the core applies pending publishes before draining
    /// the price ring, so awaiting this ack and then calling [`CoreLink::price`]
    /// guarantees the price sees this exact state (no publish-vs-price race). The
    /// RFS tick driver relies on this to keep each streamed line self-consistent.
    ///
    /// # Errors
    ///
    /// [`CoreLinkError::CoreUnavailable`] if the core has shut down.
    pub async fn publish_acked(&self, state: MarketState) -> Result<(), CoreLinkError> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(Control::Publish(state, Some(tx)))
            .map_err(|_| CoreLinkError::CoreUnavailable)?;
        rx.await.map_err(|_| CoreLinkError::CoreUnavailable)
    }

    /// Evaluate the calibrated surface vol at a `(strike, tenor)` against the
    /// live market state.
    ///
    /// # Errors
    ///
    /// [`CoreLinkError::CoreUnavailable`] if the core has shut down.
    pub async fn surface_vol(&self, query: SurfaceQuery) -> Result<SurfaceVol, CoreLinkError> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(Control::Surface(query, tx))
            .map_err(|_| CoreLinkError::CoreUnavailable)?;
        rx.await.map_err(|_| CoreLinkError::CoreUnavailable)
    }

    /// Read a projection of the live market state (spot, rates, vol-time, forward,
    /// ATM vol) for the async edge's RFQ / scenario pricing.
    ///
    /// # Errors
    ///
    /// [`CoreLinkError::CoreUnavailable`] if the core has shut down.
    pub async fn market_snapshot(&self) -> Result<MarketSnapshot, CoreLinkError> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(Control::Snapshot(tx))
            .map_err(|_| CoreLinkError::CoreUnavailable)?;
        rx.await.map_err(|_| CoreLinkError::CoreUnavailable)
    }

    /// Evaluate a market observable (ATM vol / spot / RR / BF / forward) against
    /// the live market state, on the core thread, so the value reflects exactly the
    /// smile/state the hot path prices against.
    ///
    /// Returns `Ok(None)` when the observable cannot be derived from the live state
    /// (e.g. a degenerate delta wing whose strike inversion fails).
    ///
    /// # Errors
    ///
    /// [`CoreLinkError::CoreUnavailable`] if the core has shut down.
    pub async fn observe(&self, query: ObservableQuery) -> Result<Option<f64>, CoreLinkError> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(Control::Observe(query, tx))
            .map_err(|_| CoreLinkError::CoreUnavailable)?;
        rx.await.map_err(|_| CoreLinkError::CoreUnavailable)
    }

    /// Price a single-barrier exotic against the live market state.
    ///
    /// # Errors
    ///
    /// [`CoreLinkError::CoreUnavailable`] if the core has shut down.
    pub async fn price_barrier(&self, query: ExoticQuery) -> Result<f64, CoreLinkError> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(Control::Exotic(query, tx))
            .map_err(|_| CoreLinkError::CoreUnavailable)?;
        rx.await.map_err(|_| CoreLinkError::CoreUnavailable)
    }

    /// Stop the busy-poll core and the response router, joining both threads.
    ///
    /// Idempotent: only the first call signals stop and joins; later calls are
    /// no-ops. Called on edge shutdown after the connection drain. Guarantees the
    /// two dedicated OS threads have exited before returning, so the process can
    /// always terminate (the test harness relies on this).
    pub fn stop(&self) {
        // Publish shutdown to the router first so it stops the moment the ring
        // drains, then ask the core loop to stop. Setting the flag before sending
        // Stop means even if the control channel is already gone the router still
        // unwinds.
        self.shutdown.store(true, Ordering::Release);
        let _ = self.control_tx.send(Control::Stop);
        if let Ok(mut guard) = self.threads.lock()
            && let Some((core, router)) = guard.take()
        {
            let _ = core.join();
            let _ = router.join();
        }
    }
}

impl Drop for CoreLink {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Derive a [`Observable`] from the live [`MarketState`].
///
/// `AtmVol`/`Spot`/`Forward` are read directly off the state and its smile;
/// `RiskReversal`/`Butterfly` invert the live convention to the signed delta wing
/// strikes (call at `+|delta|`, put at `−|delta|`, both struck at the live ATM vol
/// as the inversion template) and read the *live* smile vol there — so the wing
/// observables reflect the exact marked smile, never a recomputed proxy. Returns
/// `None` if a wing strike inversion fails (a degenerate delta), so the feed skips
/// a point rather than fabricating one.
fn observe_live(st: &MarketState, observable: Observable) -> Option<f64> {
    use celnet_core::Smile;
    let forward = st.forward();
    let atm_vol = Smile::implied_vol(&st.smile, forward, forward, st.t).0;
    match observable {
        Observable::AtmVol => Some(atm_vol),
        Observable::Spot => Some(st.spot),
        Observable::Forward => Some(forward),
        Observable::RiskReversal { delta } | Observable::Butterfly { delta } => {
            let mag = delta.abs();
            // Template at the ATM vol for the delta→strike inversion.
            let template = VanillaInputs::new(st.spot, forward, atm_vol, st.t, st.r_dom, st.r_for);
            let conv = st.conventions.delta;
            let call_strike =
                celnet_vanilla::strike_from_delta(conv, OptionType::Call, mag, &template).ok()?;
            let put_strike =
                celnet_vanilla::strike_from_delta(conv, OptionType::Put, -mag, &template).ok()?;
            let call_vol = Smile::implied_vol(&st.smile, call_strike, forward, st.t).0;
            let put_vol = Smile::implied_vol(&st.smile, put_strike, forward, st.t).0;
            Some(match observable {
                Observable::RiskReversal { .. } => call_vol - put_vol,
                Observable::Butterfly { .. } => 0.5 * (call_vol + put_vol) - atm_vol,
                _ => unreachable!(),
            })
        }
    }
}

/// The busy-poll body run on the dedicated core thread.
///
/// Each iteration: drain a bounded batch of hot-path price requests off the
/// request ring (pricing each and pushing the response), then service any
/// pending control-plane commands (state publish, surface / exotic reads, stop).
/// When both are momentarily idle the loop parks briefly to avoid a hot spin on a
/// shared machine; on a dedicated isolated core the park is negligible. The loop
/// never allocates on the hot pricing path (the engine guarantees that) and never
/// blocks on the async edge.
///
/// # Runtime-independent shutdown
///
/// The loop halts on **either** a `Control::Stop` (the graceful path, forwarded by
/// the async control bridge) **or** the shared `shutdown` flag set by
/// [`CoreLink::stop`]. The flag is essential: `stop()` blocks the calling thread
/// in `join()`, and on a single-threaded (current-thread) runtime that same thread
/// drives the async bridge — so a `Control::Stop` that depends on the bridge could
/// never be forwarded, deadlocking the join. Observing the flag directly lets the
/// core thread exit without any cooperation from the (possibly-blocked) runtime.
fn run_core(
    core: &mut PricingCore,
    req_rx: &mut rtrb::Consumer<PriceRequest>,
    resp_tx: &mut rtrb::Producer<PriceResponse>,
    ctrl_rx: &std::sync::mpsc::Receiver<Control>,
    shutdown: &AtomicBool,
) {
    use std::sync::mpsc::TryRecvError;
    loop {
        // Authoritative, runtime-independent halt: a set flag ends the loop at
        // once, even if the async control bridge can never deliver `Control::Stop`.
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        // Control plane FIRST: apply every pending command (state publishes,
        // surface / exotic reads) before draining the price ring. This orders a
        // publish ahead of a subsequently-submitted price so a caller that awaits
        // the publish ack and then prices sees a price computed against exactly
        // the published state (no cross-channel race between publish and price).
        let mut handled_control = false;
        loop {
            match ctrl_rx.try_recv() {
                Ok(Control::Publish(state, ack)) => {
                    core.state().publish(state);
                    if let Some(tx) = ack {
                        let _ = tx.send(());
                    }
                    handled_control = true;
                }
                Ok(Control::Surface(q, reply)) => {
                    let st = core.state().load();
                    let forward = st.forward();
                    let vol = celnet_core::Smile::implied_vol(&st.smile, q.strike, forward, st.t).0;
                    let _ = reply.send(SurfaceVol { vol, forward });
                    handled_control = true;
                }
                Ok(Control::Snapshot(reply)) => {
                    let st = core.state().load();
                    let forward = st.forward();
                    let atm_vol =
                        celnet_core::Smile::implied_vol(&st.smile, forward, forward, st.t).0;
                    let _ = reply.send(MarketSnapshot {
                        spot: st.spot,
                        r_dom: st.r_dom,
                        r_for: st.r_for,
                        t: st.t,
                        forward,
                        atm_vol,
                    });
                    handled_control = true;
                }
                Ok(Control::Exotic(q, reply)) => {
                    let spec = SingleBarrier {
                        kind: celnet_exotics::BarrierKind {
                            up: q.topology.up(),
                            style: q.topology.style(),
                            option: q.option_type,
                        },
                        strike: q.inputs.strike,
                        barrier: q.barrier,
                        rebate: q.rebate,
                    };
                    let price = single_barrier_price(&(&q.inputs).into(), spec);
                    let _ = reply.send(price);
                    handled_control = true;
                }
                Ok(Control::Observe(q, reply)) => {
                    let st = core.state().load();
                    let _ = reply.send(observe_live(&st, q.observable));
                    handled_control = true;
                }
                Ok(Control::Stop) => return,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // Hot path: price a bounded batch from the ring.
        let priced = core.drain(req_rx, resp_tx, DRAIN_BUDGET);

        // If nothing happened this iteration, park briefly so we don't pin the
        // CPU on a shared host. A pinned/isolated core would set a tiny park.
        if priced == 0 && !handled_control {
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
    }
}
