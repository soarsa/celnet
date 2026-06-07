//! Per-pair price-tick fan-out over the lock-free [`celnet_fanout`] SPMC ring.
//!
//! # Why this exists (the scale improvement)
//!
//! The RFS streaming edge fans a *moving market* out to many counterparty
//! sessions. The high-volume work is the **per-pair market evolution** — the
//! deterministic spot path each subscribed line is repriced against. The previous
//! design generated that path **per subscription**: every [`crate::services::stream`]
//! `Subscription` owned a private counter-based spot tick seeded by its
//! subscription id, so *N* subscribers to the same currency pair ran *N*
//! independent spot evolutions. That is *O(subscribers)* tick generators for a
//! quantity that is logically *O(pairs)*.
//!
//! This module replaces that with the **1-producer-per-pair → N-consumers**
//! pattern (`docs/SCALE-OUT.md` §5, the LMAX-Disruptor multi-consumer shape made
//! real by `celnet-fanout`): for each live currency pair a **single producer**
//! advances the deterministic per-pair spot path once per cadence tick and
//! **publishes** a small [`PriceTick`] POD into a `celnet-fanout`
//! [`BroadcastRing`](celnet_fanout::BroadcastRing). Every session subscribed to
//! that pair holds an independent [`Consumer`](celnet_fanout::Consumer) and
//! **drains** the ring with non-blocking `try_recv` inside its existing
//! `tokio::select!` loop, then formats *its own* `Update` (its per-subscription
//! sequence, click-to-trade tokens, snapshot/delta semantics) from the shared
//! tick. The per-pair compute is done **once** and broadcast.
//!
//! # What is and is NOT on the ring
//!
//! The ring carries **only** the high-volume per-pair price tick — the driving
//! [`MarketContext`](celnet_proto::MarketContext) state (`spot`/`vol`/`r_dom`/
//! `r_for`) plus a monotonic `tick_seq` stamp. Per-instrument pricing (the
//! two-way bid/offer and the 14-Greek set) is **necessarily** computed
//! per-subscriber, because subscribers on the same pair hold *different*
//! instruments (strike, option type, exotic). Control / lifecycle messages
//! (`Snapshot` on subscribe, `Modify`/`Resync`, `Executed`/`StreamReject` on
//! click-to-trade, `Heartbeat`, `StreamEnd`, market-series points) are **not**
//! high-volume broadcast and stay on the per-session `mpsc` path in
//! [`crate::services::stream`]. The ring is the price-update fan-out *only*.
//!
//! # Determinism (preserved, and made *more* correct)
//!
//! The per-pair spot path is the same counter-based `splitmix64` discipline the
//! old per-subscription tick used ([`crate::tick::TickSource`]), but **seeded by a
//! stable hash of the currency pair** instead of the subscription id. The
//! observable consequence — and it is intentional, not a regression — is that all
//! subscribers to a pair now see the **same** spot move (as in production: EURUSD
//! moves once for everyone), rather than *N* private synthetic paths. Every
//! published `PriceTick` is bit-reproducible from `(pair, tick_seq)`; the parity
//! harness reconstructs it independently as the oracle.
//!
//! # Hot path stays lock/alloc-free
//!
//! The producer runs on a **dedicated OS thread off the tokio runtime**. Its
//! publish path is the ring's zero-allocation, lock-free `publish` (the registry
//! `Mutex` is touched only by the control-plane *subscribe* path, never by a
//! publish). Consumers `try_recv` lock-free. The pinned engine hot core is never
//! involved: the producer reads a `MarketContext` baseline once at pair-creation
//! and evolves it deterministically — it does not call the pricing core per tick.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use celnet_fanout::{BroadcastRing, Consumer, Producer};
use celnet_proto::{CcyPair, MarketContext};

use crate::tick::TickSource;

/// The bounded depth of each per-pair broadcast ring. A consumer lagging more than
/// this many ticks behind the producer conflates forward to the oldest still-live
/// tick with exact skip accounting (the `celnet-fanout` overflow policy) — the
/// FX-streaming-correct "a stale quote is worse than a skipped one" choice. 256
/// mirrors the per-session channel depth so the two stages have matched headroom.
const RING_CAPACITY: usize = 256;

/// The per-pair multiplicative spot-bump volatility per tick (matches the previous
/// per-subscription [`crate::services::stream`] bump so the streamed line's
/// per-tick motion is the same scale).
const STREAM_BUMP: f64 = 0.0005;

/// The wall-clock interval between deterministic producer ticks. Chosen a little
/// shorter than the consumer-side drain interval so a healthy consumer always has
/// at least one fresh tick to deliver each time it polls (and a *slow* consumer
/// exercises the ring's conflation path).
const PRODUCER_INTERVAL: Duration = Duration::from_millis(2);

/// One per-pair price tick broadcast on the ring: the driving market state plus a
/// monotonic per-pair stamp. **`Copy + Default + Send`** — the `celnet-fanout`
/// ring payload bound — so it can live in the lock-free SPMC ring.
///
/// This is deliberately a small POD: the per-instrument two-way price and Greeks
/// are *not* here (they are per-subscriber, derived from this tick by the stream
/// driver). `tick_seq` lets a consumer and the parity oracle agree on exactly
/// which deterministic spot the tick carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceTick {
    /// The driving market context the subscribed line is repriced against.
    pub market: MarketContext,
    /// The monotonic per-pair tick sequence (`0` is the producer's first tick).
    /// Independent of any subscription's own `Update` sequence.
    pub tick_seq: u64,
}

impl Default for PriceTick {
    fn default() -> Self {
        Self {
            market: MarketContext {
                spot: 0.0,
                vol: 0.0,
                r_dom: 0.0,
                r_for: 0.0,
            },
            tick_seq: 0,
        }
    }
}

/// A stable 64-bit seed for a currency pair's deterministic spot path. Built from
/// the pair's `base`/`quote` codes so it is reproducible across processes and
/// independent of subscription ids. Uses the same public-domain `splitmix64`
/// finalizer the tick discipline uses, folded over the ASCII bytes (FNV-style
/// accumulation then a `splitmix64` finalize for good avalanche).
#[must_use]
pub fn pair_seed(pair: &CcyPair) -> u64 {
    // FNV-1a accumulation over the canonical "BASE/QUOTE" bytes, then a
    // splitmix64 finalize. Deterministic, allocation-free over a small buffer.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    };
    mix(pair.base.as_bytes());
    mix(b"/");
    mix(pair.quote.as_bytes());
    TickSource::splitmix64(h)
}

/// The deterministically-bumped spot for a pair at a given tick sequence, *without*
/// any mutable state — pure in `(seed, tick_seq, base_spot)`. Reuses the exact
/// `splitmix64` + `unit_signed` discipline of [`crate::tick::TickSource`] (and the
/// previous per-subscription tick) so the path is bit-identical to that mixer and
/// the parity oracle can recompute it independently.
///
/// Centering and bumping use *separate* multiply/add (never a fused `mul_add`):
/// FMA fuses the two ops at a single rounding not guaranteed bit-identical across
/// targets/opt-levels, which would break the cross-target bit-stable reproduction
/// the streaming determinism discipline promises.
#[must_use]
pub fn spot_at(seed: u64, tick_seq: u64, base_spot: f64) -> f64 {
    let mixed = TickSource::splitmix64(seed ^ tick_seq.wrapping_mul(0x2545_F491_4F6C_DD1D));
    let u = TickSource::unit_signed(mixed);
    base_spot * (u * STREAM_BUMP + 1.0)
}

/// The single producer plus its driving state for one pair, owned exclusively by
/// the producer OS thread.
struct PairProducer {
    producer: Producer<PriceTick>,
    base: MarketContext,
    seed: u64,
    tick_seq: u64,
}

impl PairProducer {
    /// Advance and publish the next deterministic tick for this pair.
    #[inline]
    fn drive(&mut self) {
        let spot = spot_at(self.seed, self.tick_seq, self.base.spot);
        let market = MarketContext { spot, ..self.base };
        self.producer.publish(PriceTick {
            market,
            tick_seq: self.tick_seq,
        });
        self.tick_seq = self.tick_seq.wrapping_add(1);
    }
}

/// A control-plane request to the producer thread to ensure a pair has a live ring
/// and return a fresh consumer for it.
struct SubscribeRequest {
    pair: CcyPair,
    base: MarketContext,
    reply: std::sync::mpsc::Sender<Consumer<PriceTick>>,
}

/// The per-pair price-tick fan-out hub.
///
/// One hub per edge. It owns a dedicated producer thread that drives every live
/// pair's ring on [`PRODUCER_INTERVAL`]. Sessions call [`PriceFanout::subscribe`]
/// to obtain a [`Consumer<PriceTick>`] for a pair (lazily creating that pair's
/// ring on first subscribe); they then drain it from their async loop.
#[derive(Debug)]
pub(crate) struct PriceFanout {
    /// Control channel to the producer thread (subscribe / ensure-pair).
    subscribe_tx: std::sync::mpsc::Sender<SubscribeRequest>,
    /// Set false to ask the producer thread to wind down; joined on drop.
    running: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl PriceFanout {
    /// Spawn the per-pair price-tick fan-out, with its dedicated producer thread.
    #[must_use]
    pub(crate) fn start() -> Arc<Self> {
        let (subscribe_tx, subscribe_rx) = std::sync::mpsc::channel::<SubscribeRequest>();
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = Arc::clone(&running);
        let handle = std::thread::Builder::new()
            .name("celnet-pricefanout".to_owned())
            .spawn(move || producer_loop(subscribe_rx, &running_thread))
            .expect("spawn price-fanout producer thread");
        Arc::new(Self {
            subscribe_tx,
            running,
            handle: Mutex::new(Some(handle)),
        })
    }

    /// Subscribe to a pair's price-tick ring, returning an independent consumer
    /// positioned at the producer's current head (it observes only ticks from now
    /// on — the same "from now on" semantics a freshly-seeded per-subscription
    /// tick had). The pair's ring is created lazily on first subscribe, seeded
    /// from `base` (the subscription's baseline market). A returned `None` means
    /// the producer thread has stopped (edge shutting down) — the caller falls
    /// back to no streamed updates, never a fake.
    #[must_use]
    pub(crate) fn subscribe(
        &self,
        pair: &CcyPair,
        base: MarketContext,
    ) -> Option<Consumer<PriceTick>> {
        let (reply, reply_rx) = std::sync::mpsc::channel::<Consumer<PriceTick>>();
        self.subscribe_tx
            .send(SubscribeRequest {
                pair: pair.clone(),
                base,
                reply,
            })
            .ok()?;
        reply_rx.recv().ok()
    }
}

impl Drop for PriceFanout {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        // Dropping the last `subscribe_tx` also unblocks the producer loop's recv.
        if let Some(handle) = self.handle.lock().expect("fanout handle lock").take() {
            let _ = handle.join();
        }
    }
}

/// The dedicated producer-thread loop. Owns every per-pair [`PairProducer`] and the
/// shared [`PairRing`] templates, drives all live pairs once per
/// [`PRODUCER_INTERVAL`], and services subscribe requests between cadence ticks.
/// Runs entirely off the tokio runtime so a slow async consumer can never stall
/// publication (the ring conflates instead).
fn producer_loop(subscribe_rx: std::sync::mpsc::Receiver<SubscribeRequest>, running: &AtomicBool) {
    let mut producers: HashMap<(String, String), PairProducer> = HashMap::new();
    let mut last = std::time::Instant::now();

    while running.load(Ordering::Acquire) {
        // Service all pending subscribe requests (non-blocking), creating a ring
        // for any new pair and replying with a fresh head-positioned consumer.
        loop {
            match subscribe_rx.try_recv() {
                Ok(req) => {
                    let key = (req.pair.base.clone(), req.pair.quote.clone());
                    let entry = producers.entry(key).or_insert_with(|| {
                        let ring = BroadcastRing::<PriceTick>::new(RING_CAPACITY);
                        let producer = ring.into_producer();
                        PairProducer {
                            seed: pair_seed(&req.pair),
                            base: req.base,
                            tick_seq: 0,
                            producer,
                        }
                    });
                    // A fresh consumer at the producer's current head.
                    let consumer = entry.producer.subscribe_from_head();
                    let _ = req.reply.send(consumer);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The hub (and all sessions) dropped: no more subscribers can
                    // arrive. Finish driving is pointless — wind down.
                    return;
                }
            }
        }

        // Drive every live pair once per cadence interval.
        let now = std::time::Instant::now();
        if now.duration_since(last) >= PRODUCER_INTERVAL {
            last = now;
            for prod in producers.values_mut() {
                prod.drive();
            }
        }

        // Park briefly so the thread is not a busy-spin; short enough to keep the
        // subscribe path responsive and the cadence near PRODUCER_INTERVAL.
        std::thread::sleep(PRODUCER_INTERVAL / 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(base: &str, quote: &str) -> CcyPair {
        CcyPair {
            base: base.to_owned(),
            quote: quote.to_owned(),
        }
    }

    fn base_market(spot: f64) -> MarketContext {
        MarketContext {
            spot,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.01,
        }
    }

    /// The per-pair seed is stable and pair-specific: the same pair always seeds
    /// identically, and different pairs (almost surely) seed differently.
    #[test]
    fn pair_seed_is_stable_and_pair_specific() {
        assert_eq!(
            pair_seed(&pair("EUR", "USD")),
            pair_seed(&pair("EUR", "USD"))
        );
        assert_ne!(
            pair_seed(&pair("EUR", "USD")),
            pair_seed(&pair("GBP", "USD"))
        );
        // Not a naive concatenation collision: EUR/USD ≠ EU/RUSD.
        assert_ne!(
            pair_seed(&pair("EUR", "USD")),
            pair_seed(&pair("EU", "RUSD"))
        );
    }

    /// The deterministic spot path is pure in `(seed, tick_seq, base_spot)` and
    /// stays strictly positive for any `STREAM_BUMP < 1` (so a streamed price is
    /// never built on a non-positive spot).
    #[test]
    fn spot_path_is_pure_and_positive() {
        let seed = pair_seed(&pair("EUR", "USD"));
        for t in 0..256u64 {
            let a = spot_at(seed, t, 1.10);
            let b = spot_at(seed, t, 1.10);
            assert_eq!(a.to_bits(), b.to_bits(), "pure in (seed, tick, spot)");
            assert!(a > 0.0, "spot stays positive: {a}");
        }
    }

    /// End-to-end through the real producer thread: two consumers subscribed to the
    /// SAME pair observe the SAME deterministic tick stream in order (genuine
    /// broadcast — not a partition), and each reconstructed spot matches the pure
    /// `spot_at` oracle.
    #[test]
    fn two_consumers_same_pair_see_identical_ordered_ticks() {
        let hub = PriceFanout::start();
        let p = pair("EUR", "USD");
        let seed = pair_seed(&p);
        let base = base_market(1.10);
        let mut c1 = hub.subscribe(&p, base).expect("ring");
        let mut c2 = hub.subscribe(&p, base).expect("ring");

        // Collect a handful of ticks from each consumer, deadline-bounded.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got1: Vec<PriceTick> = Vec::new();
        let mut got2: Vec<PriceTick> = Vec::new();
        while (got1.len() < 8 || got2.len() < 8) && std::time::Instant::now() < deadline {
            while let Ok(t) = c1.try_recv() {
                got1.push(t);
            }
            while let Ok(t) = c2.try_recv() {
                got2.push(t);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(got1.len() >= 8 && got2.len() >= 8, "both saw enough ticks");

        // Each consumer's ticks are strictly increasing in tick_seq and match the
        // pure spot oracle for that seq.
        for got in [&got1, &got2] {
            for w in got.windows(2) {
                assert!(w[0].tick_seq < w[1].tick_seq, "in order, no duplicates");
            }
            for t in got.iter() {
                let want = spot_at(seed, t.tick_seq, base.spot);
                assert_eq!(
                    t.market.spot.to_bits(),
                    want.to_bits(),
                    "tick spot matches the pure oracle"
                );
            }
        }
        // Overlapping tick_seqs carry identical spots across the two consumers
        // (broadcast: same published value for everyone).
        let lo1 = got1.first().unwrap().tick_seq;
        let lo2 = got2.first().unwrap().tick_seq;
        let hi1 = got1.last().unwrap().tick_seq;
        let hi2 = got2.last().unwrap().tick_seq;
        let overlap_lo = lo1.max(lo2);
        let overlap_hi = hi1.min(hi2);
        assert!(overlap_hi >= overlap_lo, "the streams overlap");
        for seq in overlap_lo..=overlap_hi {
            let a = got1
                .iter()
                .find(|t| t.tick_seq == seq)
                .map(|t| t.market.spot);
            let b = got2
                .iter()
                .find(|t| t.tick_seq == seq)
                .map(|t| t.market.spot);
            if let (Some(a), Some(b)) = (a, b) {
                assert_eq!(a.to_bits(), b.to_bits(), "broadcast: same value per seq");
            }
        }
        drop(hub);
    }

    /// **Conflation accounting (c)** on the exact consumer the stream driver drains:
    /// a consumer that polls slowly is lapped by the producer; the ring conflates it
    /// forward to the oldest still-live tick with **exact skip accounting**
    /// (`received + skipped == produced-observed`), every delivered tick is a real
    /// published value matching the pure oracle, and the delivered stream stays
    /// strictly increasing (forward-only — never torn, duplicated, or reordered).
    #[test]
    fn slow_consumer_conflates_forward_with_exact_skip_accounting() {
        let hub = PriceFanout::start();
        let p = pair("EUR", "USD");
        let seed = pair_seed(&p);
        let base = base_market(1.10);
        let mut c = hub.subscribe(&p, base).expect("ring");
        // The cursor at subscribe time is the head this consumer started from
        // (received == skipped == 0 here); exact accounting is relative to it.
        let start_head = c.cursor();
        assert_eq!(c.received(), 0);
        assert_eq!(c.skipped(), 0);

        // Poll slowly so the 2ms producer laps the capacity-256 ring. To lap, the
        // consumer must fall > capacity ticks behind: 256 ticks × 2ms ≈ 512ms, so a
        // ~700ms gap between reads guarantees the ring overwrites unread slots and
        // the consumer conflates forward to the oldest still-live tick.
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut delivered: Vec<PriceTick> = Vec::new();
        let mut observed_conflation = false;
        while delivered.len() < 4 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(700));
            if let Ok(t) = c.try_recv() {
                if let Some(prev) = delivered.last() {
                    assert!(
                        t.tick_seq > prev.tick_seq,
                        "forward-only, strictly increasing: {} -> {}",
                        prev.tick_seq,
                        t.tick_seq
                    );
                    if t.tick_seq > prev.tick_seq + 1 {
                        observed_conflation = true;
                    }
                }
                // Every delivered tick is a real published value (matches the oracle).
                assert_eq!(
                    t.market.spot.to_bits(),
                    spot_at(seed, t.tick_seq, base.spot).to_bits(),
                    "a delivered tick is a real produced value, never torn/invented"
                );
                delivered.push(t);
            }
        }
        assert!(
            delivered.len() >= 4,
            "saw enough deliveries before the deadline"
        );
        assert!(
            observed_conflation,
            "a slow consumer must observe forward conflation (skipped ≥1 intermediate)"
        );
        // Exact accounting: every produced tick this consumer has reached is either
        // received or skipped — nothing is lost or double-counted. The producer's
        // head (received + skipped) must equal a real produced count, and the last
        // received tick must be at or after received+skipped−1's frontier.
        let received = c.received();
        let skipped = c.skipped();
        assert!(received > 0 && skipped > 0, "both received and conflated");
        // The frontier the consumer has accounted for == its cursor (next-to-read)
        // minus the head it subscribed from: received + skipped is exactly how many
        // produced ticks it has passed since subscribing — no loss, no double-count.
        assert_eq!(
            received + skipped,
            c.cursor() - start_head,
            "received + skipped == produced-observed (exact, no loss, no double-count)"
        );
        drop(hub);
    }
}
