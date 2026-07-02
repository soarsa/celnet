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
//!   publishes the whole live [`MarketState`] atomically; the hot core reads it
//!   through a cached [`StateReader`] ([`StateHandle::reader`]) with no lock and
//!   no allocation even under concurrent publishes, and a writer hot-swaps it in
//!   one atomic store.
//! * **Single-writer seqlock** — [`Seqlock`] publishes a small `Copy`
//!   [`PriceSnapshot`] to many readers with no reader-side locking (readers
//!   retry on a torn read). The payload is copied **per word with atomic
//!   accesses**, so the reader/writer overlap is never a data race (see the
//!   `seqlock` module docs for the soundness argument).
//! * **False-sharing guard** — [`PaddedCounter`] wraps a hot atomic in a
//!   `crossbeam-utils` `CachePadded` so independent counters never share a cache
//!   line.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use celnet_surface::MarketHedgeSmile;
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
/// calibrated smile (a [`MarketHedgeSmile`], which implements
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
    pub smile: MarketHedgeSmile,
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
#[derive(Debug, Clone)]
pub struct StateHandle {
    inner: Arc<ArcSwap<MarketState>>,
}

impl StateHandle {
    /// Publish an initial state.
    #[must_use]
    pub fn new(initial: MarketState) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(initial)),
        }
    }

    /// Load the current state with a lock-free, wait-free read, returning an
    /// owned `Arc`.
    ///
    /// The underlying state stays alive for as long as the returned `Arc` is
    /// held even if a concurrent [`StateHandle::publish`] swaps in a new one.
    ///
    /// Note: this uses `arc_swap::ArcSwap::load_full`. On the **hot pricing
    /// path** prefer a [`StateReader`] ([`StateHandle::reader`]), whose cached
    /// revalidation is unconditionally allocation-free — even while a writer
    /// hammers `publish` — because it never takes `arc-swap`'s per-load
    /// debt-reclamation path.
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

    /// Build a [`StateReader`] — the allocation-free hot-path read handle.
    ///
    /// The reader caches the loaded `Arc` and, on each [`StateReader::load`],
    /// cheaply revalidates it against the live pointer: an unchanged pointer is a
    /// pure atomic compare (no clone, no allocation); a changed pointer triggers
    /// a single `Arc` strong-count clone (a refcount bump, **not** a heap
    /// allocation). Crucially this read path never uses `arc-swap`'s cheap-guard
    /// debt slots, so it cannot fall back to the slower debt-reclamation path
    /// that can allocate under concurrent `publish` — making it unconditionally
    /// allocation-free on the steady-state hot loop.
    #[must_use]
    pub fn reader(&self) -> StateReader {
        StateReader {
            cache: arc_swap::Cache::new(Arc::clone(&self.inner)),
        }
    }
}

/// An allocation-free, cached reader of the live [`MarketState`] for the hot
/// pricing loop.
///
/// Created by [`StateHandle::reader`]. Each [`StateReader::load`] returns the
/// current state, revalidating a cached `Arc` against the live pointer. On the
/// steady-state hot loop it performs **no heap allocation** even while a writer
/// concurrently publishes new states (proven in `tests/zero_alloc.rs`).
#[derive(Debug)]
pub struct StateReader {
    cache: arc_swap::Cache<Arc<ArcSwap<MarketState>>, Arc<MarketState>>,
}

impl StateReader {
    /// Load the current [`MarketState`], allocation-free.
    ///
    /// Returns a borrow valid until the next `load`. An unchanged underlying
    /// pointer costs a single atomic load + compare; a changed pointer costs one
    /// `Arc` strong-count clone (no heap allocation).
    pub fn load(&mut self) -> &Arc<MarketState> {
        self.cache.load()
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
    //!
    //! # Why the payload is stored as per-word atomics, not a plain `UnsafeCell<T>`
    //!
    //! A naïve seqlock copies the payload with plain (non-atomic) reads and
    //! writes, relying on the sequence check to *discard* a torn read. That is a
    //! **data race / undefined behavior** under the C++20 / Rust memory model:
    //! the reader's load and the writer's store touch the same bytes without
    //! synchronization, and the model forbids the racing access from *occurring*
    //! at all — it does not merely make the *result* unspecified. A conforming
    //! compiler may then assume the race never happens and miscompile the load
    //! (tear it across the sequence checks, hoist it, or synthesize a trap
    //! value). This is the textbook reason real seqlocks (the `seqlock` crate,
    //! Folly `SeqLock`, the Linux kernel's `READ_ONCE`/`WRITE_ONCE`) never use a
    //! plain field copy.
    //!
    //! We make the copy sound by storing the payload as a fixed array of
    //! [`AtomicUsize`] words and copying it **per word with relaxed atomic
    //! accesses**. Per-word atomic load/store is, by definition, never a data
    //! race even when reader and writer touch the same word concurrently — the
    //! reader simply observes one of the two values for that word. The sequence
    //! protocol (Acquire/Release on `seq`) still provides the *consistency*
    //! guarantee: a reader that observes a stable even sequence around its copy
    //! is guaranteed the per-word values it read all belong to the same publish.
    //! The relaxed per-word ordering is sufficient because the Acquire load of
    //! `seq` *after* the copy, paired with the writer's Release store of `seq`,
    //! establishes the happens-before edge that orders the payload words; the
    //! reader only *returns* a value once that edge is confirmed by `before ==
    //! after`.

    use std::marker::PhantomData;
    use std::mem::{MaybeUninit, align_of, size_of};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// Number of `usize` words needed to cover `T`'s byte image (rounding up so
    /// any trailing padding bytes are also copied through the atomic words).
    fn word_count<T>() -> usize {
        size_of::<T>().div_ceil(size_of::<usize>())
    }

    /// A single-writer, many-reader seqlock over a `Copy` payload `T`.
    ///
    /// The payload is held as a fixed-length, boxed slice of [`AtomicUsize`]
    /// words (`ceil(size_of::<T>() / size_of::<usize>())` of them), allocated
    /// **once at construction**, so reader/writer copies are per-word atomic and
    /// therefore never a data race; see the module docs for the soundness
    /// argument. The slice never reallocates after construction, so the hot
    /// `store`/`read` paths perform no allocation.
    #[derive(Debug)]
    pub struct Seqlock<T: Copy> {
        seq: AtomicU64,
        words: Box<[AtomicUsize]>,
        /// Catches an accidental *second* concurrent writer under
        /// `debug_assertions`: `store` flips it to `true` on entry and back to
        /// `false` on exit, asserting it was `false` on entry. The single-writer
        /// contract makes this a pure diagnostic — it carries no release/acquire
        /// obligation and is compiled out in release builds.
        #[cfg(debug_assertions)]
        writer_active: std::sync::atomic::AtomicBool,
        _marker: PhantomData<T>,
    }

    // SAFETY: the payload lives in a `Box<[AtomicUsize]>`, so every concurrent
    // reader/writer access to the shared bytes is a per-word atomic access and is
    // not a data race for any thread count. The atomic `seq` sequence protocol
    // provides the *consistency* guarantee (a reader returns a value only when it
    // bracketed the copy with two equal even sequence reads). `T: Copy` means no
    // destructors / no interior owning pointers, so reconstituting a `T` from the
    // copied words is sound. Thus sharing across threads is sound for any
    // `T: Copy + Send`.
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
            // The atomic-word copy reinterprets `T`'s bytes through `usize`-aligned
            // storage; assert `T` is not over-aligned relative to `usize` so the
            // `copy_nonoverlapping` in `read_words`/`write_words` is sound.
            assert!(
                align_of::<T>() <= align_of::<usize>(),
                "Seqlock payload alignment must not exceed usize alignment"
            );
            let words = (0..word_count::<T>())
                .map(|_| AtomicUsize::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let lock = Self {
                seq: AtomicU64::new(0),
                words,
                #[cfg(debug_assertions)]
                writer_active: std::sync::atomic::AtomicBool::new(false),
                _marker: PhantomData,
            };
            // Seed the payload words through the same atomic path the readers use,
            // so the initial value is observable before any `store`.
            lock.write_words(initial);
            lock
        }

        /// Decompose `value` into its `usize` words and store each atomically
        /// (relaxed — consistency is carried by the `seq` Release store the
        /// caller performs afterwards). The final partial word (if `T` does not
        /// fill a whole word) is zero-padded so the byte image is fully defined.
        ///
        /// Allocation-free: the per-word staging value is a single `usize` on the
        /// stack.
        #[allow(unsafe_code)]
        fn write_words(&self, value: T) {
            let total = size_of::<T>();
            // Raw byte view of the source `value` (a live local; never aliased).
            let src = (&raw const value).cast::<u8>();
            let word = size_of::<usize>();
            for (i, slot) in self.words.iter().enumerate() {
                // Bytes this word covers: a full word, or the zero-padded tail.
                let off = i * word;
                let n = (total - off).min(word);
                // A zeroed stack word guarantees the tail/padding bytes are 0.
                let mut buf: usize = 0;
                // SAFETY: `off + n <= total = size_of::<T>()`, so the source range
                // is within `value`'s byte image; `buf` is a `usize` on the stack
                // (one word, properly aligned), so writing `n <= word` bytes to
                // its front is in bounds. Both pointers are valid and non-aliasing
                // (distinct locals).
                unsafe {
                    std::ptr::copy_nonoverlapping(src.add(off), (&raw mut buf).cast::<u8>(), n);
                }
                slot.store(buf, Ordering::Relaxed);
            }
        }

        /// Atomically load each payload word and reassemble a `T`.
        ///
        /// Allocation-free: copies each loaded word into a stack `MaybeUninit<T>`.
        #[allow(unsafe_code)]
        fn read_words(&self) -> T {
            let total = size_of::<T>();
            let word = size_of::<usize>();
            let mut out = MaybeUninit::<T>::uninit();
            let dst = out.as_mut_ptr().cast::<u8>();
            for (i, slot) in self.words.iter().enumerate() {
                let buf = slot.load(Ordering::Relaxed);
                let off = i * word;
                let n = (total - off).min(word);
                // SAFETY: `off + n <= total = size_of::<T>()`, so the destination
                // range is within `out`'s storage; `buf` is a stack `usize`
                // (>= `n` bytes). After the loop every byte of `out` has been
                // written exactly once, fully initializing the `T`. `T: Copy`, so
                // the reconstituted byte image is a valid `T` (a bitwise copy of a
                // `T` previously written by `write_words`).
                unsafe {
                    std::ptr::copy_nonoverlapping((&raw const buf).cast::<u8>(), dst.add(off), n);
                }
            }
            // SAFETY: the loop above wrote all `size_of::<T>()` bytes of `out`.
            unsafe { out.assume_init() }
        }

        /// Publish a new value.
        ///
        /// Takes `&self` so the lock can be shared with readers behind an `Arc`.
        /// **Single-writer contract:** the caller must ensure at most one thread
        /// invokes `store` at a time (the engine satisfies this by giving the
        /// pricing core sole ownership of the writer endpoint). A violation is
        /// caught by a `debug_assert!` under `debug_assertions`.
        pub fn store(&self, value: T) {
            #[cfg(debug_assertions)]
            {
                // Single-writer guard: must not already be inside a `store`.
                let was_active = self
                    .writer_active
                    .swap(true, std::sync::atomic::Ordering::Relaxed);
                debug_assert!(
                    !was_active,
                    "Seqlock single-writer contract violated: a second thread \
                     entered store() concurrently"
                );
            }

            // Enter the write critical section: make the sequence odd. The single
            // writer owns `seq`, so the load/store pair is uncontended; Release on
            // the odd store orders it before the payload words for any reader that
            // later Acquire-loads it.
            let seq = self.seq.load(Ordering::Relaxed);
            self.seq.store(seq.wrapping_add(1), Ordering::Release);
            // Write the payload word-by-word (relaxed atomics — never a race).
            self.write_words(value);
            // Publish: leave the critical section with an even seq, Release so the
            // payload words happen-before any reader's confirming Acquire load.
            self.seq.store(seq.wrapping_add(2), Ordering::Release);

            #[cfg(debug_assertions)]
            {
                self.writer_active
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }

        /// Read a consistent snapshot, retrying on a torn read.
        ///
        /// Wait-free for the writer; the reader spins only while the writer is
        /// mid-write (a bounded, single-writer window).
        #[must_use]
        pub fn read(&self) -> T {
            loop {
                let before = self.seq.load(Ordering::Acquire);
                if before & 1 != 0 {
                    // Writer is mid-write; retry.
                    std::hint::spin_loop();
                    continue;
                }
                // Per-word atomic copy: never a data race even if it overlaps a
                // concurrent write; any torn combination is rejected below.
                let value = self.read_words();
                let after = self.seq.load(Ordering::Acquire);
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

        /// Test-only: pretend a writer is already inside the critical section,
        /// so the next `store` deterministically trips the single-writer
        /// `debug_assert!`. Lets the guard be exercised without relying on a
        /// racy thread-overlap.
        #[cfg(all(test, debug_assertions))]
        pub(crate) fn force_writer_active_for_test(&self) {
            self.writer_active
                .store(true, std::sync::atomic::Ordering::Relaxed);
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

    /// The initial value seeded by `new` is readable before any `store`, and the
    /// per-word copy reproduces it bit-for-bit.
    #[test]
    fn seqlock_initial_value_is_readable() {
        let snap = PriceSnapshot {
            request_id: 99,
            price: -2.5,
            delta_spot: 0.123_456_789,
            vega: 1e-12,
            vol: 0.0987,
        };
        let s = Seqlock::new(snap);
        assert_eq!(s.read(), snap);
        assert_eq!(s.sequence(), 0);
    }

    /// Regression for the data-race blocker + the partial-tail-word path: a
    /// payload whose size is **not** a multiple of the word size must round-trip
    /// bit-exactly through the per-word atomic copy (the final partial word is
    /// zero-padded and reassembled correctly).
    #[test]
    fn seqlock_roundtrips_non_word_multiple_payload() {
        // 9 fields of mixed width incl. a bool + u8 so the byte image has a
        // ragged tail and interior padding, exercising `write_words`/`read_words`
        // boundary handling.
        #[derive(Clone, Copy, PartialEq, Debug, Default)]
        struct Ragged {
            a: u8,
            b: u64,
            c: u16,
            d: f64,
            e: bool,
        }
        let s = Seqlock::<Ragged>::default();
        let v = Ragged {
            a: 0xAB,
            b: 0x0123_4567_89AB_CDEF,
            c: 0xBEEF,
            d: core::f64::consts::PI,
            e: true,
        };
        s.store(v);
        assert_eq!(s.read(), v);
    }

    /// Regression for the minor single-writer-contract finding: a second writer
    /// entering the critical section must trip the `debug_assert!` guard (panic)
    /// rather than silently corrupt the protocol. Deterministic: we mark the
    /// writer active (as a concurrent second writer would) then expect `store`
    /// to panic. Only meaningful under `debug_assertions`.
    #[cfg(debug_assertions)]
    #[test]
    fn seqlock_detects_second_concurrent_writer() {
        let s = Seqlock::new(PriceSnapshot::default());
        s.force_writer_active_for_test();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.store(PriceSnapshot {
                request_id: 1,
                ..Default::default()
            });
        }));
        std::panic::set_hook(prev);
        assert!(
            r.is_err(),
            "a second concurrent writer must trip the single-writer debug guard"
        );
    }

    #[test]
    fn pin_is_total() {
        // Pinning must never panic; out-of-range index simply fails gracefully.
        let _ = pin_current_thread_to_core(0);
        assert!(!pin_current_thread_to_core(usize::MAX));
    }

    /// ADR-0016 hot-core embargo (the central-core-unification Phase-A1 deliverable,
    /// critique F5): the pinned streaming [`MarketState`] must NEVER gain a
    /// request/batch-tier curve handle — no `celnet_core::contract::ResolvedMarket`,
    /// no borrowed `&dyn DiscountCurve` / `celnet_core::CurveCarry`, and no owned
    /// term-structure handle (`Arc<celnet_rates::curve::Curve>`). The unified
    /// pricing contract deliberately homes those at the request tier; the flat-`f64`
    /// hot state stays exactly its market scalars + resolved conventions + smile.
    ///
    /// This is enforced structurally by three independent facts.
    ///
    /// The `'static` bound: `MarketState` is published as `Arc<ArcSwap<MarketState>>`
    /// and read lock-free on the hot path, so it is `'static`. Every request-tier
    /// curve handle is a BORROWED value — `ResolvedMarket<'a>`, `CurveCarry<'a>`, or
    /// a bare `&'a dyn DiscountCurve` — carrying a non-`'static` lifetime; adding one
    /// as a field would make `MarketState` lifetime-parameterised and fail this bound
    /// (a compile error — the embargo biting), and would also break the existing
    /// `ArcSwap` publication.
    ///
    /// The size pin: `MarketState` is EXACTLY its four flat `f64` market scalars plus
    /// the resolved `ConventionRecord` plus the calibrated `MarketHedgeSmile` —
    /// proven against an independently-declared shadow with the identical field set.
    /// Smuggling in any extra field (e.g. an owned `Arc<dyn DiscountCurve>`, 8 bytes)
    /// grows the size and trips this pin.
    ///
    /// The absent `celnet-rates` edge: the general bootstrapped `Curve` lives in
    /// `celnet-rates`, which `celnet-engine` does not depend on (gate: `cargo tree -p
    /// celnet-engine` lists no `celnet-rates`), so a term-structure curve handle is
    /// not even nameable in the hot core.
    #[test]
    fn hot_core_embargoes_request_tier_curve_handles() {
        // (1) Borrowed handles are barred by the `'static` bound.
        fn assert_static<T: 'static>() {}
        assert_static::<MarketState>();

        // (2) No extra (owned) field may be smuggled in: the layout matches the
        // exact declared field set. Any curve handle added to `MarketState` without
        // mirroring it here (a glaring, review-visible edit) trips this pin.
        struct HotStateShadow {
            _spot: f64,
            _r_dom: f64,
            _r_for: f64,
            _t: f64,
            _conventions: ConventionRecord,
            _smile: MarketHedgeSmile,
        }
        assert_eq!(
            core::mem::size_of::<MarketState>(),
            core::mem::size_of::<HotStateShadow>(),
            "MarketState must stay flat: no ResolvedMarket / Arc<Curve> / DiscountCurve \
             handle field may enter the hot core (ADR-0016)"
        );
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
