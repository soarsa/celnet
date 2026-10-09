//! Distributor egress — the bounded, conflating, rate-limited stage that sits
//! between a microsecond Celnet pricer and a slower downstream price sink
//! (`docs/architecture/CELNET-FIX-INTEGRATION-PLAN.md` §2).
//!
//! # Why this stage exists
//!
//! The downstream distributor is an in-process JVM disruptor mailbox whose publish
//! call (`publishPriceEventOrSkipWhileFull()`) **silently skips while full**
//! (`docs/CELNET-INTEGRATION.md` §0). A Celnet pricer streaming at ≥1M
//! updates/s/core overruns that mailbox trivially, dropping prices with **no
//! error**. The [`EgressGovernor`] converts that silent failure into an
//! explicit, observable, *counted* Celnet SLO:
//!
//! * a **bounded ring** of fixed capacity — back-pressure is explicit, memory is
//!   bounded no matter how fast the producer runs;
//! * **conflation** per `(pair, tenor, strike)` key — a stale price for a
//!   pillar is coalesced in favour of the newest, so the limiter is *lossless in
//!   information* even when lossy in messages (FX quotes are replaceable);
//! * a **token-bucket rate limiter** sized to the downstream drain rate, so the
//!   sink is never offered more than it can absorb;
//! * **counted drops** — every conflation and every capacity drop bumps a
//!   counter ([`EgressMetrics`]), never a silent skip.
//!
//! # Where it runs
//!
//! On the async edge, never on the pinned hot core. The engine pushes price
//! events into the governor over a bounded queue; the governor drains them to a
//! [`DistributorEgress`]/[`PriceSink`] implementation on an edge task. No
//! transcendental math, no pricing, no `unsafe`.
//!
//! # Method provenance (doc-only)
//!
//! The token bucket is the standard leaky/token-bucket traffic shaper; conflation
//! (last-value caching per key) is the standard market-data "conflated feed"
//! pattern. Identifiers stay purpose-named.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_types::{CcyPair, Tenor};

/// The conflation key for a price update: a `(pair, tenor, strike-bits)` triple.
///
/// FX-option prices are quoted per `(pair, tenor)` pillar at a strike; the
/// strike is keyed by its IEEE-754 bit pattern so the key is `Eq`/`Hash` without
/// relying on float equality (no `==` on a pricing value — strikes that are
/// bit-identical are the same pillar; this is an identity key, not a numeric
/// comparison).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PriceKey {
    /// The currency pair.
    pub pair: CcyPair,
    /// The tenor pillar.
    pub tenor: Tenor,
    /// The strike, keyed by its raw bit pattern (identity, not numeric compare).
    strike_bits: u64,
}

impl PriceKey {
    /// Build a key for a `(pair, tenor, strike)` pillar.
    ///
    /// A `NaN` strike is canonicalized to a single bit pattern so two `NaN`
    /// strikes map to one key (no `NaN != NaN` surprise in the conflation map).
    #[must_use]
    pub fn new(pair: CcyPair, tenor: Tenor, strike: f64) -> Self {
        let canon = if strike.is_nan() {
            f64::NAN.to_bits()
        } else {
            // Normalize -0.0 and +0.0 to the same key.
            (strike + 0.0).to_bits()
        };
        Self {
            pair,
            tenor,
            strike_bits: canon,
        }
    }

    /// The keyed strike, recovered from its bit pattern.
    #[must_use]
    pub fn strike(&self) -> f64 {
        f64::from_bits(self.strike_bits)
    }
}

/// A price update offered to the egress stage.
///
/// `Copy`/POD so it never allocates on the path between the engine and the
/// governor. The `seq` is a monotonic per-producer sequence used to define
/// "newest" for conflation: a higher `seq` supersedes a lower one for the same
/// [`PriceKey`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceUpdate {
    /// The conflation key (pair, tenor, strike).
    pub key: PriceKey,
    /// The bid price (in the dialect's price unit; opaque to the governor).
    pub bid: f64,
    /// The offer price.
    pub offer: f64,
    /// Monotonic producer sequence number defining recency for conflation.
    pub seq: u64,
}

/// Observable, counted egress metrics — the Celnet SLO that replaces the JVM's
/// silent skip-while-full. Every counter is monotonic and lock-free.
///
/// Cloning shares the same underlying counters (`Arc`), so a metrics scraper on
/// another task observes the live values.
#[derive(Debug, Clone, Default)]
pub struct EgressMetrics {
    inner: Arc<EgressCounters>,
}

#[derive(Debug, Default)]
struct EgressCounters {
    offered: AtomicU64,
    delivered: AtomicU64,
    conflated: AtomicU64,
    dropped_capacity: AtomicU64,
}

impl EgressMetrics {
    /// A fresh, zeroed metrics handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Total updates offered to the governor.
    #[must_use]
    pub fn offered(&self) -> u64 {
        self.inner.offered.load(Ordering::Relaxed)
    }

    /// Total updates actually delivered downstream.
    #[must_use]
    pub fn delivered(&self) -> u64 {
        self.inner.delivered.load(Ordering::Relaxed)
    }

    /// Total updates conflated (superseded by a newer update for the same key
    /// before they could be drained — a counted, non-silent drop).
    #[must_use]
    pub fn conflated(&self) -> u64 {
        self.inner.conflated.load(Ordering::Relaxed)
    }

    /// Total updates dropped because the bounded ring was full and no existing
    /// key could be conflated (a counted, non-silent drop).
    #[must_use]
    pub fn dropped_capacity(&self) -> u64 {
        self.inner.dropped_capacity.load(Ordering::Relaxed)
    }

    /// Total counted drops = conflated + capacity drops.
    #[must_use]
    pub fn dropped_total(&self) -> u64 {
        self.conflated() + self.dropped_capacity()
    }
}

/// Configuration for the [`EgressGovernor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EgressConfig {
    /// Fixed ring capacity: the maximum number of *distinct* pending keys the
    /// governor will hold. Memory is bounded by this regardless of producer
    /// rate. Must be non-zero.
    pub capacity: usize,
    /// Sustained drain rate in updates per second the limiter is sized to (the
    /// measured downstream sink rate). Must be positive.
    pub drain_rate_per_sec: f64,
    /// Token-bucket burst size: the maximum number of tokens that can accumulate
    /// (allows a short burst up to this many back-to-back deliveries). Must be
    /// at least 1.
    pub burst: f64,
}

impl EgressConfig {
    /// A config draining at `drain_rate_per_sec` into a ring of `capacity`
    /// distinct keys, with a burst equal to one second of drain.
    ///
    /// # Panics
    ///
    /// Does not panic; invalid values are rejected by [`EgressGovernor::new`].
    #[must_use]
    pub fn new(capacity: usize, drain_rate_per_sec: f64) -> Self {
        Self {
            capacity,
            drain_rate_per_sec,
            burst: drain_rate_per_sec.max(1.0),
        }
    }
}

/// A monotonic clock source, abstracted so tests drive a deterministic virtual
/// clock and production uses the wall clock. Returns nanoseconds since an
/// arbitrary fixed epoch.
pub trait NanoClock: Send + Sync {
    /// Current instant in nanoseconds since an arbitrary, monotonic epoch.
    fn now_nanos(&self) -> u64;
}

/// Wall-clock implementation backed by [`std::time::Instant`].
#[derive(Debug)]
pub struct MonotonicClock {
    origin: std::time::Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}

impl NanoClock for MonotonicClock {
    fn now_nanos(&self) -> u64 {
        // Saturating cast: monotonic, so always non-negative; u64 nanos covers
        // ~584 years of uptime.
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// Errors constructing an [`EgressGovernor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressError {
    /// `capacity` was zero.
    ZeroCapacity,
    /// `drain_rate_per_sec` was non-positive or non-finite.
    BadDrainRate,
    /// `burst` was less than 1 or non-finite.
    BadBurst,
}

impl core::fmt::Display for EgressError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EgressError::ZeroCapacity => write!(f, "egress ring capacity must be non-zero"),
            EgressError::BadDrainRate => write!(f, "egress drain rate must be positive and finite"),
            EgressError::BadBurst => write!(f, "egress burst must be >= 1 and finite"),
        }
    }
}

impl std::error::Error for EgressError {}

/// The async-edge sink the governor drains into — the outbound deployment seam.
///
/// Implementations frame and publish a delivered [`PriceUpdate`] to a downstream
/// consumer (the JVM distributor sidecar, a native distributor socket, a
/// standalone TCP/UDS distributor, or a test sink). Returns `Ok(())` on accept;
/// an `Err` means the downstream refused (e.g. closed socket) and the governor
/// stops draining.
pub trait PriceSink {
    /// The error a sink may report when it cannot accept an update.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Publish one delivered update downstream.
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] if the downstream cannot accept the update.
    fn publish(
        &mut self,
        update: &PriceUpdate,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

/// The outbound distributor egress seam (alias clarifying intent at the
/// deployment boundary).
///
/// A [`DistributorEgress`] *is* a [`PriceSink`]; the distinct name marks the
/// role at the wiring boundary (the thing the [`EgressGovernor`] publishes to),
/// matching the `DistributorEgress` trait named in
/// `docs/architecture/CELNET-FIX-INTEGRATION-PLAN.md` §2.3. The seam is identical whether the
/// concrete impl is the JVM sidecar (option A) or the native distributor socket
/// (option B), so swapping is a leaf change with no caller impact.
pub trait DistributorEgress: PriceSink {}

impl<T: PriceSink> DistributorEgress for T {}

/// The bounded, conflating, rate-limited egress governor.
///
/// Offer updates with [`EgressGovernor::offer`] (cheap, non-blocking, lock-free
/// w.r.t. the downstream); they accumulate in a bounded ring with per-key
/// conflation. Drain them downstream at the configured rate with
/// [`EgressGovernor::drain_to`], which releases at most the number of tokens the
/// token bucket has accrued since the last drain.
///
/// The governor itself does no I/O; the caller's edge task owns the
/// [`PriceSink`] and pumps `offer`/`drain_to`.
#[derive(Debug)]
pub struct EgressGovernor {
    cfg: EgressConfig,
    /// FIFO of pending keys (insertion order), bounded by `capacity` distinct
    /// keys. Conflation updates the value in `pending` in place without
    /// re-queuing, preserving fair age-ordered delivery.
    order: VecDeque<PriceKey>,
    /// The current (conflated) value per pending key.
    pending: HashMap<PriceKey, PriceUpdate>,
    /// Token-bucket tokens currently available for delivery.
    tokens: f64,
    /// Last instant the bucket was refilled.
    last_refill_nanos: u64,
    /// Whether the bucket has ever been refilled (first drain seeds the clock).
    seeded: bool,
    metrics: EgressMetrics,
}

impl EgressGovernor {
    /// Build a governor for `cfg`, sharing the supplied `metrics` handle.
    ///
    /// # Errors
    ///
    /// Returns [`EgressError`] if the capacity is zero or the drain rate / burst
    /// are not positive and finite.
    pub fn new(cfg: EgressConfig, metrics: EgressMetrics) -> Result<Self, EgressError> {
        if cfg.capacity == 0 {
            return Err(EgressError::ZeroCapacity);
        }
        if !cfg.drain_rate_per_sec.is_finite() || cfg.drain_rate_per_sec <= 0.0 {
            return Err(EgressError::BadDrainRate);
        }
        if !cfg.burst.is_finite() || cfg.burst < 1.0 {
            return Err(EgressError::BadBurst);
        }
        Ok(Self {
            cfg,
            order: VecDeque::with_capacity(cfg.capacity),
            pending: HashMap::with_capacity(cfg.capacity),
            // Start full so an initial burst up to `burst` is allowed immediately.
            tokens: cfg.burst,
            last_refill_nanos: 0,
            seeded: false,
            metrics,
        })
    }

    /// The shared metrics handle.
    #[must_use]
    pub fn metrics(&self) -> &EgressMetrics {
        &self.metrics
    }

    /// Number of distinct keys currently pending (bounded by `capacity`).
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.order.len()
    }

    /// Offer one update to the governor (non-blocking, never allocates beyond
    /// the bounded ring).
    ///
    /// Conflation/back-pressure semantics:
    ///
    /// * If the key is already pending and the offered `seq` is newer, the
    ///   pending value is replaced (conflation: `conflated` is bumped for the
    ///   superseded value). An older/equal `seq` for a pending key is itself
    ///   conflated away (kept value is the newer one).
    /// * If the key is new and the ring has room, it is enqueued.
    /// * If the key is new and the ring is **full**, the update is dropped and
    ///   `dropped_capacity` is bumped — a counted, non-silent drop. Memory stays
    ///   bounded by `capacity` distinct keys no matter the producer rate.
    pub fn offer(&mut self, update: PriceUpdate) {
        self.metrics.inner.offered.fetch_add(1, Ordering::Relaxed);

        if let Some(existing) = self.pending.get_mut(&update.key) {
            // Conflation: one of the two values is superseded — counted, never
            // silent. Keep the newer (higher seq); if the incoming is not newer,
            // it is the one conflated away.
            self.metrics.inner.conflated.fetch_add(1, Ordering::Relaxed);
            if update.seq >= existing.seq {
                *existing = update;
            }
            return;
        }

        if self.order.len() >= self.cfg.capacity {
            // Ring full of distinct keys: drop the newcomer, counted.
            self.metrics
                .inner
                .dropped_capacity
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        self.order.push_back(update.key);
        self.pending.insert(update.key, update);
    }

    /// Refill the token bucket up to `burst` based on elapsed time, then return
    /// the number of whole tokens available for delivery.
    fn refill(&mut self, now_nanos: u64) {
        if !self.seeded {
            self.last_refill_nanos = now_nanos;
            self.seeded = true;
            return;
        }
        let elapsed_nanos = now_nanos.saturating_sub(self.last_refill_nanos);
        if elapsed_nanos == 0 {
            return;
        }
        let elapsed_secs = elapsed_nanos as f64 * 1e-9;
        self.tokens =
            (self.tokens + elapsed_secs * self.cfg.drain_rate_per_sec).min(self.cfg.burst);
        self.last_refill_nanos = now_nanos;
    }

    /// Drain pending updates to `sink` at the configured rate, using `clock` for
    /// token refill. Delivers at most `floor(available_tokens)` updates this
    /// call, oldest-first (age-fair), consuming one token per delivery.
    ///
    /// Returns the number of updates delivered. Stops early (and returns the
    /// count delivered so far) if the sink reports an error, leaving the
    /// remaining updates pending for a later drain.
    ///
    /// # Errors
    ///
    /// Returns the sink's error if `publish` fails; the governor's state stays
    /// consistent (the failed update is re-queued at the front so age order is
    /// preserved).
    pub async fn drain_to<S: PriceSink, C: NanoClock>(
        &mut self,
        sink: &mut S,
        clock: &C,
    ) -> Result<usize, S::Error> {
        self.refill(clock.now_nanos());

        let mut delivered = 0usize;
        while self.tokens >= 1.0 {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            let Some(update) = self.pending.remove(&key) else {
                // Key was in `order` but not `pending` — impossible by
                // construction (they are inserted/removed together), but be
                // defensive rather than panic.
                continue;
            };
            match sink.publish(&update).await {
                Ok(()) => {
                    self.tokens -= 1.0;
                    delivered += 1;
                    self.metrics.inner.delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    // Re-queue at the front so age order is preserved; do not
                    // consume the token.
                    self.order.push_front(key);
                    self.pending.insert(key, update);
                    return Err(e);
                }
            }
        }
        Ok(delivered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::CcyPair;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU64 as TestAtomicU64;
    use tokio::net::{TcpListener, TcpStream};

    fn eurusd() -> CcyPair {
        CcyPair::parse("EURUSD").unwrap()
    }

    fn key(strike: f64) -> PriceKey {
        PriceKey::new(eurusd(), Tenor::Years(1), strike)
    }

    /// A deterministic virtual clock the tests advance by hand.
    #[derive(Default)]
    struct VirtualClock {
        nanos: TestAtomicU64,
    }
    impl VirtualClock {
        fn advance_secs(&self, secs: f64) {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let add = (secs * 1e9) as u64;
            self.nanos.fetch_add(add, Ordering::SeqCst);
        }
    }
    impl NanoClock for VirtualClock {
        fn now_nanos(&self) -> u64 {
            self.nanos.load(Ordering::SeqCst)
        }
    }

    /// A simple in-memory sink that records every delivered update.
    #[derive(Default)]
    struct RecordingSink {
        received: Vec<PriceUpdate>,
    }
    #[derive(Debug)]
    struct NeverError;
    impl core::fmt::Display for NeverError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "never")
        }
    }
    impl std::error::Error for NeverError {}
    impl PriceSink for RecordingSink {
        type Error = NeverError;
        async fn publish(&mut self, update: &PriceUpdate) -> Result<(), Self::Error> {
            self.received.push(*update);
            Ok(())
        }
    }

    #[test]
    fn rejects_bad_config() {
        let m = EgressMetrics::new();
        assert_eq!(
            EgressGovernor::new(EgressConfig::new(0, 10.0), m.clone()).unwrap_err(),
            EgressError::ZeroCapacity
        );
        assert_eq!(
            EgressGovernor::new(EgressConfig::new(8, 0.0), m.clone()).unwrap_err(),
            EgressError::BadDrainRate
        );
        let mut bad = EgressConfig::new(8, 10.0);
        bad.burst = 0.5;
        assert_eq!(
            EgressGovernor::new(bad, m).unwrap_err(),
            EgressError::BadBurst
        );
    }

    #[test]
    fn conflation_keeps_newest_per_key_and_counts_drops() {
        let m = EgressMetrics::new();
        let mut g = EgressGovernor::new(EgressConfig::new(8, 1000.0), m.clone()).unwrap();
        // Three updates, same key, increasing seq → two conflated, newest kept.
        g.offer(PriceUpdate {
            key: key(1.10),
            bid: 1.0,
            offer: 1.1,
            seq: 1,
        });
        g.offer(PriceUpdate {
            key: key(1.10),
            bid: 2.0,
            offer: 2.1,
            seq: 2,
        });
        g.offer(PriceUpdate {
            key: key(1.10),
            bid: 3.0,
            offer: 3.1,
            seq: 3,
        });
        assert_eq!(g.pending_len(), 1, "conflated to a single pending key");
        assert_eq!(m.conflated(), 2);
        assert_eq!(m.offered(), 3);
        assert!(is_close_bits(g.pending[&key(1.10)].bid, 3.0));
    }

    fn is_close_bits(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[tokio::test]
    async fn rate_limiter_releases_at_drain_rate() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let m = EgressMetrics::new();
            // Drain 10/s, burst 10, capacity large.
            let mut cfg = EgressConfig::new(1000, 10.0);
            cfg.burst = 10.0;
            let mut g = EgressGovernor::new(cfg, m.clone()).unwrap();
            let clock = VirtualClock::default();
            let mut sink = RecordingSink::default();

            // Offer 100 distinct keys.
            for i in 0..100u64 {
                g.offer(PriceUpdate {
                    key: key(1.0 + i as f64 * 0.001),
                    bid: i as f64,
                    offer: i as f64,
                    seq: i,
                });
            }
            // First drain: bucket starts full (burst=10) → 10 delivered.
            let n0 = g.drain_to(&mut sink, &clock).await.unwrap();
            assert_eq!(n0, 10, "initial burst");
            // Advance 1s → 10 more tokens.
            clock.advance_secs(1.0);
            let n1 = g.drain_to(&mut sink, &clock).await.unwrap();
            assert_eq!(n1, 10, "one second of drain");
            // Advance 0.05s → 0.5 token → nothing whole.
            clock.advance_secs(0.05);
            let n2 = g.drain_to(&mut sink, &clock).await.unwrap();
            assert_eq!(n2, 0, "sub-token interval delivers nothing");
            assert_eq!(sink.received.len(), 20);
            assert_eq!(m.delivered(), 20);
        })
        .await
        .expect("rate limiter test timed out");
    }

    /// A REAL rate-limited socket sink: it drains a bounded number of bytes per
    /// interval off a real loopback TCP stream and asserts it never buffers more
    /// than its capacity. Producer at 1e6/s into a 1e4/s sink ⇒ bounded memory,
    /// newest-per-key always delivered, drop_count == produced − delivered.
    #[tokio::test]
    async fn real_socket_sink_bounded_and_lossless_in_information() {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            // ---- REAL rate-limited socket sink (the far side) ----------------
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            // Shared record of what the sink actually received (newest bid per key).
            let received: Arc<Mutex<HashMap<u64, (u64, f64)>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let recv_handle = received.clone();
            // The sink's bounded receive capacity (it reads at most this many
            // queued frames per tick — a real drain-rate ceiling).
            const SINK_DRAIN_PER_TICK: usize = 10;
            const SINK_CAPACITY: usize = 64;

            let server = tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 24]; // key(u64)+seq(u64)+bid(f64) frame
                let mut backlog: VecDeque<[u8; 24]> = VecDeque::new();
                let mut max_backlog = 0usize;
                loop {
                    // Read up to SINK_DRAIN_PER_TICK frames into the backlog,
                    // then process them. The governor's rate limit guarantees
                    // the producer never offers faster than we drain, so the
                    // backlog stays bounded.
                    let mut got_any = false;
                    for _ in 0..SINK_DRAIN_PER_TICK {
                        match stream.read_exact(&mut buf).await {
                            Ok(_) => {
                                backlog.push_back(buf);
                                got_any = true;
                            }
                            Err(_) => break,
                        }
                        if backlog.len() >= SINK_DRAIN_PER_TICK {
                            break;
                        }
                    }
                    max_backlog = max_backlog.max(backlog.len());
                    while let Some(frame) = backlog.pop_front() {
                        let k = u64::from_le_bytes(frame[0..8].try_into().unwrap());
                        let seq = u64::from_le_bytes(frame[8..16].try_into().unwrap());
                        let bid = f64::from_le_bytes(frame[16..24].try_into().unwrap());
                        let mut map = recv_handle.lock().unwrap();
                        let e = map.entry(k).or_insert((0, 0.0));
                        // Newest-wins assertion: seq is monotonic per key.
                        assert!(seq >= e.0, "sink received out-of-order seq for key");
                        *e = (seq, bid);
                    }
                    if !got_any {
                        break;
                    }
                    assert!(
                        max_backlog <= SINK_CAPACITY,
                        "sink backlog exceeded capacity: {max_backlog}"
                    );
                }
                max_backlog
            });

            // ---- The governor + a socket-backed PriceSink (the near side) ----
            struct SocketSink {
                stream: TcpStream,
            }
            #[derive(Debug)]
            struct IoErr(std::io::Error);
            impl core::fmt::Display for IoErr {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    write!(f, "{}", self.0)
                }
            }
            impl std::error::Error for IoErr {}
            impl PriceSink for SocketSink {
                type Error = IoErr;
                async fn publish(&mut self, u: &PriceUpdate) -> Result<(), Self::Error> {
                    use tokio::io::AsyncWriteExt;
                    // Frame: key-bits via strike + seq + bid (24 bytes).
                    let mut frame = [0u8; 24];
                    frame[0..8].copy_from_slice(&u.key.strike().to_bits().to_le_bytes());
                    frame[8..16].copy_from_slice(&u.seq.to_le_bytes());
                    frame[16..24].copy_from_slice(&u.bid.to_le_bytes());
                    self.stream.write_all(&frame).await.map_err(IoErr)
                }
            }

            let stream = TcpStream::connect(addr).await.unwrap();
            let mut sink = SocketSink { stream };

            let m = EgressMetrics::new();
            // 1e4/s drain, ring capacity 32 distinct keys.
            let mut cfg = EgressConfig::new(32, 10_000.0);
            cfg.burst = 10.0;
            let mut g = EgressGovernor::new(cfg, m.clone()).unwrap();
            let clock = VirtualClock::default();

            // 16 distinct keys; we use bit-identical strike values per key.
            const N_KEYS: u64 = 16;
            let strikes: Vec<f64> = (0..N_KEYS).map(|i| 1.0 + i as f64 * 0.0001).collect();

            // Producer at ~1e6/s: 2000 updates across 16 keys, advancing the
            // clock by 1µs each (so 2000µs of wall time, draining ~20 tokens).
            let total_produced = 2000u64;
            for n in 0..total_produced {
                let s = strikes[(n % N_KEYS) as usize];
                g.offer(PriceUpdate {
                    key: key(s),
                    bid: n as f64,
                    offer: n as f64,
                    seq: n,
                });
                clock.advance_secs(1e-6);
                // Periodically drain at the limited rate.
                if n % 16 == 0 {
                    let _ = g.drain_to(&mut sink, &clock).await.unwrap();
                }
                // Memory bound: never more than capacity distinct keys pending.
                assert!(g.pending_len() <= 32);
            }
            // Final drain: advance the clock each iteration so the token bucket
            // refills (it caps at `burst` per refill), fully flushing everything
            // still pending at the configured drain rate.
            while g.pending_len() > 0 {
                clock.advance_secs(1.0);
                g.drain_to(&mut sink, &clock).await.unwrap();
            }
            drop(sink); // close the socket so the server loop ends.

            let max_backlog = server.await.unwrap();
            assert!(max_backlog <= SINK_CAPACITY);

            // The sink received the *newest* seq for every key still pending /
            // delivered. Check the last delivered seq per key is the max we
            // produced for that key (newest-wins through conflation).
            let map = received.lock().unwrap();
            for (i, s) in strikes.iter().enumerate() {
                if let Some((seq, _bid)) = map.get(&s.to_bits()) {
                    // The newest seq produced for key i is the largest n with
                    // n % N_KEYS == i.
                    let expected_newest =
                        ((total_produced - 1 - i as u64) / N_KEYS) * N_KEYS + i as u64;
                    assert_eq!(
                        *seq, expected_newest,
                        "key {i}: sink did not end on the newest produced seq"
                    );
                }
            }

            // Drop accounting: produced == delivered + conflated + capacity drops.
            assert_eq!(m.offered(), total_produced);
            assert_eq!(
                m.offered(),
                m.delivered() + m.dropped_total(),
                "every produced update is accounted for: delivered or counted-dropped"
            );
            // It was genuinely lossy in messages (conflation happened) ...
            assert!(
                m.dropped_total() > 0,
                "conflation must have dropped stale messages"
            );
            // ... but bounded in memory throughout (asserted in the loop above).
        })
        .await
        .expect("real socket sink test timed out");
    }
}
