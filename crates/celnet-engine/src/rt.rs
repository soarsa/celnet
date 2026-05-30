//! Latency runtime primitives (`docs/ARCHITECTURE.md` §3.2).
//!
//! The two-tier engine (async edge ⇄ pinned hot core) is joined by these
//! wait-free / lock-free building blocks, never by mutexes or unbounded
//! channels:
//!
//! * **Core pinning** — [`pin_current_thread_to_core`] (over `core_affinity`)
//!   so the hot core never migrates and never shares a core with the edge.
//! * **Wait-free SPSC rings** — [`RequestRing`] / [`ResponseRing`] (over `rtrb`)
//!   carry `Copy`/POD request and response records edge→core and core→edge.
//! * **Lock-free read-mostly publication** — [`StateHandle`] (over `arc-swap`)
//!   publishes the whole live [`MarketState`] atomically; readers load it with
//!   no lock and no contention, and a writer hot-swaps it in one atomic store.
//! * **Single-writer seqlock** — [`Seqlock`] publishes a small `Copy`
//!   [`PriceSnapshot`] to many readers with no reader-side locking (readers
//!   retry on a torn read).
//! * **False-sharing guard** — [`PaddedCounter`] wraps a hot atomic in a
//!   `crossbeam-utils` `CachePadded` so independent counters never share a cache
//!   line.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use celnet_surface::VannaVolgaSmile;
use crossbeam_utils::CachePadded;

use celnet_conventions::ConventionRecord;

/// The capacity (records) of each hot-path SPSC ring.
///
/// Power-of-two so the consumer/producer index arithmetic the ring performs
/// stays a mask rather than a modulo. Sized to absorb a burst of in-flight
/// requests without ever blocking the edge; back-pressure (a full ring) is
/// surfaced to the caller rather than allocating.
pub const RING_CAPACITY: usize = 1 << 12;

/// The published live market/convention state a pricing core reads on every
/// request.
///
/// This is the read-mostly state hot-swapped behind [`StateHandle`]: the
/// calibrated smile (a [`VannaVolgaSmile`], which implements
/// [`celnet_core::Smile`]), the market state needed to form the Garman-Kohlhagen
/// inputs (spot, the two continuously-compounded rates), and the resolved FX
/// [`ConventionRecord`] for the slice. It is an immutable snapshot: a new market
/// tick or a recalibration produces a *new* `MarketState` that is published
/// atomically, never mutated in place.
#[derive(Debug, Clone)]
pub struct MarketState {
    /// Spot FX rate (quote per 1 unit of base).
    pub spot: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub r_for: f64,
    /// Vol-time to expiry (years) of the published smile slice.
    pub t: f64,
    /// The resolved FX conventions for the slice.
    pub conventions: ConventionRecord,
    /// The calibrated smile evaluated by the pricing core to obtain the Black
    /// vol at the requested strike.
    pub smile: VannaVolgaSmile,
}

impl MarketState {
    /// Outright forward `F = S·e^{(r_dom − r_for)·t}`.
    ///
    /// Uses `celnet_core::math::exp` so the value is bit-identical across
    /// platforms (no libm/`std` intrinsic divergence on the reproducible path).
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.spot * celnet_core::math::exp((self.r_dom - self.r_for) * self.t)
    }
}

/// A lock-free, read-mostly publication handle for the live [`MarketState`].
///
/// Wraps `arc_swap::ArcSwap`: many pricing-core readers `load()` the current
/// state with no lock and no contention, while a single edge-side writer
/// `store()`s a brand-new state (a market tick or a recalibration / hot model
/// swap, §5.1) in one atomic, wait-free operation. Readers in flight keep the
/// `Arc` they loaded alive; the swap never blocks them.
#[derive(Debug)]
pub struct StateHandle {
    inner: ArcSwap<MarketState>,
}

impl StateHandle {
    /// Publish an initial state.
    #[must_use]
    pub fn new(initial: MarketState) -> Self {
        Self {
            inner: ArcSwap::from_pointee(initial),
        }
    }

    /// Load the current state with a lock-free, wait-free read.
    ///
    /// Returns an `Arc` guard; the underlying state stays alive for as long as
    /// the guard is held even if a concurrent [`StateHandle::publish`] swaps in a
    /// new one.
    #[must_use]
    pub fn load(&self) -> Arc<MarketState> {
        self.inner.load_full()
    }

    /// Atomically publish a new state, replacing the previous one.
    ///
    /// Wait-free for both the writer and any concurrent readers; readers see
    /// either the old or the new state in full, never a mix.
    pub fn publish(&self, state: MarketState) {
        self.inner.store(Arc::new(state));
    }
}

/// A small, `Copy` price snapshot published by the pricing core to many readers.
///
/// This is the top-of-book result a downstream consumer (telemetry, the edge's
/// last-known-good cache, a streaming feed) reads without ever taking a lock,
/// via the single-writer [`Seqlock`]. Kept `Copy` and small so the seqlock's
/// torn-read protocol can copy it cheaply between version checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceSnapshot {
    /// The request id this snapshot answers.
    pub request_id: u64,
    /// Present value (domestic premium per 1 unit of base notional).
    pub price: f64,
    /// Spot delta (premium-unadjusted).
    pub delta_spot: f64,
    /// Vega (per `1.0` absolute vol).
    pub vega: f64,
    /// The Black vol the smile assigned to the priced strike.
    pub vol: f64,
}

impl Default for PriceSnapshot {
    fn default() -> Self {
        Self {
            request_id: 0,
            price: 0.0,
            delta_spot: 0.0,
            vega: 0.0,
            vol: 0.0,
        }
    }
}

mod seqlock {
    //! Single-writer / multi-reader seqlock for `Copy` snapshots (§3.2).
    //!
    //! A seqlock lets one writer publish a small `Copy` value to many readers
    //! with **no reader-side locking**: the writer bumps an odd sequence number
    //! before the write and an even one after; a reader snapshots the value
    //! between two reads of the sequence and retries if the sequence changed or
    //! was odd (a *torn* read). This is the right primitive for a high-fan-out
    //! top-of-book point where readers must never block the writer.
    //!
    //! [`Seqlock::store`] takes `&self` so the lock can be shared (e.g. in an
    //! `Arc`) between one writer thread and many reader threads — the textbook
    //! deployment. Soundness does **not** rely on `&mut`: it rests on the atomic
    //! sequence protocol plus the **single-writer contract** (at most one thread
    //! calls `store` at a time). Two concurrent writers are a contract violation
    //! (and would, as in any seqlock, corrupt the protocol).

    use std::cell::UnsafeCell;
    use std::sync::atomic::{AtomicU64, Ordering, fence};

    /// A single-writer, many-reader seqlock over a `Copy` payload `T`.
    #[derive(Debug)]
    pub struct Seqlock<T: Copy> {
        seq: AtomicU64,
        value: UnsafeCell<T>,
    }

    // SAFETY: the seqlock protocol makes concurrent access sound. By contract at
    // most one thread calls `store` at a time; readers only ever read the cell
    // between two acquire-loads of an even sequence and discard the result on a
    // torn read, so no reader ever observes a half-written value as live. The
    // `UnsafeCell<T>` is the only shared-mutable state and `T: Copy` (no
    // destructors / interior pointers), so publishing across threads is sound for
    // any `T: Copy + Send`.
    #[allow(unsafe_code)]
    unsafe impl<T: Copy + Send> Sync for Seqlock<T> {}

    impl<T: Copy + Default> Default for Seqlock<T> {
        fn default() -> Self {
            Self::new(T::default())
        }
    }

    impl<T: Copy> Seqlock<T> {
        /// Construct a seqlock holding `initial`.
        #[must_use]
        pub fn new(initial: T) -> Self {
            Self {
                seq: AtomicU64::new(0),
                value: UnsafeCell::new(initial),
            }
        }

        /// Publish a new value.
        ///
        /// Takes `&self` so the lock can be shared with readers behind an `Arc`.
        /// **Single-writer contract:** the caller must ensure at most one thread
        /// invokes `store` at a time (the engine satisfies this by giving the
        /// pricing core sole ownership of the writer endpoint).
        #[allow(unsafe_code)]
        pub fn store(&self, value: T) {
            // Enter the write critical section: make the sequence odd.
            let seq = self.seq.load(Ordering::Relaxed);
            self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
            // Ensure the odd sequence is visible before the payload write.
            fence(Ordering::Release);
            // SAFETY: single-writer by contract; readers never treat a value read
            // during an odd sequence (or a sequence that changed across the read)
            // as live, so this exclusive write never races an observed read.
            unsafe {
                *self.value.get() = value;
            }
            // Publish the payload, then leave the critical section (even seq).
            self.seq.store(seq.wrapping_add(2), Ordering::Release);
        }

        /// Read a consistent snapshot, retrying on a torn read.
        ///
        /// Wait-free for the writer; the reader spins only while the writer is
        /// mid-write (a bounded, single-writer window).
        #[must_use]
        #[allow(unsafe_code)]
        pub fn read(&self) -> T {
            loop {
                let before = self.seq.load(Ordering::Acquire);
                if before & 1 != 0 {
                    // Writer is mid-write; retry.
                    std::hint::spin_loop();
                    continue;
                }
                // SAFETY: `T: Copy`, so this is a plain bitwise copy with no
                // aliasing of a live `&mut`. If the copy races a write we detect
                // it via the sequence check below and discard the result.
                let value = unsafe { *self.value.get() };
                fence(Ordering::Acquire);
                let after = self.seq.load(Ordering::Relaxed);
                if before == after {
                    return value;
                }
                std::hint::spin_loop();
            }
        }

        /// The current sequence number (even ⇔ no write in progress). Exposed
        /// for tests and diagnostics.
        #[must_use]
        pub fn sequence(&self) -> u64 {
            self.seq.load(Ordering::Acquire)
        }
    }
}

pub use seqlock::Seqlock;

/// A hot atomic counter padded to its own cache line to kill false sharing.
///
/// Per §3.2 every shared hot atomic is wrapped in `crossbeam-utils`'
/// `CachePadded` so two independent counters (e.g. *requests priced* and
/// *requests dropped*) updated by different cores never contend on the same
/// cache line — the single highest-leverage SPSC optimization.
#[derive(Debug, Default)]
#[repr(transparent)]
pub struct PaddedCounter(CachePadded<AtomicU64>);

impl PaddedCounter {
    /// A zeroed counter.
    #[must_use]
    pub fn new() -> Self {
        Self(CachePadded::new(AtomicU64::new(0)))
    }

    /// Increment by one and return the previous value (relaxed — counters carry
    /// no happens-before obligation).
    pub fn incr(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }

    /// Add `n`, returning the previous value.
    pub fn add(&self, n: u64) -> u64 {
        self.0.fetch_add(n, Ordering::Relaxed)
    }

    /// The current value.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// The edge→core request ring: a wait-free SPSC producer/consumer pair carrying
/// `Copy` price requests. The edge holds the producer; the pinned pricing core
/// holds the consumer.
pub type RequestRing = (
    rtrb::Producer<crate::core::PriceRequest>,
    rtrb::Consumer<crate::core::PriceRequest>,
);

/// The core→edge response ring: a wait-free SPSC pair carrying `Copy` price
/// responses back to the edge. The core holds the producer; the edge holds the
/// consumer.
pub type ResponseRing = (
    rtrb::Producer<crate::core::PriceResponse>,
    rtrb::Consumer<crate::core::PriceResponse>,
);

/// Allocate a request ring sized to [`RING_CAPACITY`].
#[must_use]
pub fn request_ring() -> RequestRing {
    rtrb::RingBuffer::new(RING_CAPACITY)
}

/// Allocate a response ring sized to [`RING_CAPACITY`].
#[must_use]
pub fn response_ring() -> ResponseRing {
    rtrb::RingBuffer::new(RING_CAPACITY)
}

/// The live book the engine prices, hot-swappable behind [`StateHandle`]'s
/// sibling pattern and serialized by [`crate::handoff`] for blue-green upgrade.
///
/// Each [`BookEntry`] is a booked position (an option line) the engine reprices
/// on every market tick. The book is `Clone` so a snapshot can be taken for
/// serialization without holding any lock.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookState {
    /// The booked option lines, in insertion order.
    pub entries: Vec<BookEntry>,
}

/// A single booked option line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookEntry {
    /// Stable identifier of the booked line.
    pub id: u64,
    /// Call or put.
    pub option_type: celnet_types::OptionType,
    /// Strike (quote per 1 unit of base).
    pub strike: f64,
    /// Base-currency notional of the position (signed: long positive).
    pub notional: f64,
}

impl BookState {
    /// An empty book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a booked line.
    pub fn push(&mut self, entry: BookEntry) {
        self.entries.push(entry);
    }

    /// The number of booked lines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the book is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Pin the current thread to the physical core with index `core_index`.
///
/// Wraps `core_affinity`: the engine pins the hot pricing core (and the edge) to
/// dedicated cores so the busy-poll loop never migrates and never shares a core
/// with the async edge (§3.2). Returns `true` on success. If the platform
/// exposes no affinity control, or `core_index` is out of range, returns `false`
/// and the caller runs unpinned (correct, just not isolated) — pinning is a
/// performance affordance, never a correctness requirement, so this never
/// panics.
#[must_use]
pub fn pin_current_thread_to_core(core_index: usize) -> bool {
    match core_affinity::get_core_ids() {
        Some(ids) => match ids.get(core_index) {
            Some(&id) => core_affinity::set_for_current(id),
            None => false,
        },
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_counter_is_cache_line_isolated() {
        // CachePadded forces each counter onto its own cache line: the struct is
        // at least a cache line wide (64 or 128 bytes depending on target).
        assert!(std::mem::size_of::<PaddedCounter>() >= 64);
        let c = PaddedCounter::new();
        assert_eq!(c.incr(), 0);
        assert_eq!(c.add(4), 1);
        assert_eq!(c.get(), 5);
    }

    #[test]
    fn seqlock_single_thread_roundtrip() {
        let s = Seqlock::new(PriceSnapshot::default());
        let snap = PriceSnapshot {
            request_id: 7,
            price: 1.234,
            delta_spot: 0.5,
            vega: 0.2,
            vol: 0.1,
        };
        s.store(snap);
        assert_eq!(s.read(), snap);
        // Sequence is even after a completed write.
        assert_eq!(s.sequence() & 1, 0);
    }

    #[test]
    fn pin_is_total() {
        // Pinning must never panic; out-of-range index simply fails gracefully.
        let _ = pin_current_thread_to_core(0);
        assert!(!pin_current_thread_to_core(usize::MAX));
    }

    #[test]
    fn state_handle_publishes_atomically() {
        let pair = celnet_types::CcyPair::parse("EURUSD").unwrap();
        let conv = celnet_conventions::resolve(pair, celnet_types::Tenor::Years(1)).record;
        let s0 = crate::testing::market_state(1.10, 0.105, 0.015, 0.0035, conv);
        let handle = StateHandle::new(s0);
        let before = handle.load();
        assert!((before.spot - 1.10).abs() < 1e-12);

        let s1 = crate::testing::market_state(1.20, 0.11, 0.02, 0.004, conv);
        handle.publish(s1);
        let after = handle.load();
        assert!((after.spot - 1.20).abs() < 1e-12);
        // The previously-loaded guard still observes the old state in full.
        assert!((before.spot - 1.10).abs() < 1e-12);
    }
}
