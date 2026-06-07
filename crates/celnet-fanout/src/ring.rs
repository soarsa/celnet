//! The lock-free SPMC broadcast ring itself.
//!
//! ## Memory model
//!
//! The ring is a power-of-two array of `Slot<T>`. Each slot carries a
//! **sequence stamp** (`AtomicU64`) and an `UnsafeCell<T>` payload. Publication
//! of slot `i` for the global sequence `s` (`s % capacity == i`) uses a
//! single-writer **seqlock** protocol:
//!
//! 1. producer writes the payload into the slot (`*cell = item`),
//! 2. producer `Release`-stores `seq = s + 1` into the slot stamp (odd → "this
//!    slot now holds the item for sequence `s`"; here we use `s + 1` so the
//!    stamp is always `>= 1` and strictly identifies the published sequence).
//!
//! A consumer wanting global sequence `s` reads slot `i = s % capacity`:
//!
//! 1. `Acquire`-load the stamp; if `stamp != s + 1` the slot does not (yet, or no
//!    longer) hold sequence `s` — either it is not produced yet, or it has been
//!    lapped (overwritten by a *later* sequence). The consumer distinguishes the
//!    two by comparing against the producer's published head.
//! 2. otherwise `Copy` the payload out (`T: Copy`),
//! 3. re-`Acquire`-load the stamp; if it changed, the producer overwrote the
//!    slot mid-read (a torn read) → retry / treat as lapped.
//!
//! Because there is exactly **one** producer, the stamp is monotonic per slot
//! (it only ever advances by `capacity` between successive writes to the same
//! slot), so the two-load seqlock check cleanly detects any concurrent overwrite.
//!
//! ## Single producer, many consumers
//!
//! `Producer` is **not** `Clone` (enforces single-producer). `Consumer` is freely
//! cloned/created — each gets an independent cursor and skip counter and shares
//! the `Arc<Inner<T>>` immutably. Consumers never write the ring's control state
//! except their own (thread-local) cursor, so there is no inter-consumer
//! contention.

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_utils::CachePadded;

/// Why a [`Consumer::try_recv`] returned no item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvError {
    /// The producer has not published anything past this consumer's cursor yet.
    /// The cursor is unchanged; poll again later.
    Empty,
}

/// One ring slot: a versioned seqlock stamp plus the payload cell.
///
/// ## Stamp encoding (true seqlock, in-progress marker included)
///
/// `stamp` encodes the global sequence currently (or being) written to this slot,
/// shifted left one bit with the low bit a **writing-in-progress** flag:
///
/// * `stamp == (seq << 1)`      → the slot **stably** holds global sequence `seq`.
/// * `stamp == (seq << 1) | 1`  → the producer is **mid-write** of `seq` (payload
///   bytes may be torn; readers must reject this).
/// * the initial `stamp == 0` means "never written" — and because real published
///   sequences start at `0` whose stable stamp is also `0`, we reserve the
///   sentinel by initializing slots to [`UNWRITTEN`] (`u64::MAX`, an odd value, so
///   it reads as a permanently-in-progress non-sequence that no `(seq << 1)` ever
///   equals — a clean "empty" that the cursor/head check skips anyway).
///
/// The crucial property over the previous `seq+1` scheme: the producer raises the
/// in-progress bit **before** touching the payload and clears it **after**, so a
/// reader can never copy a payload that is being overwritten without detecting it
/// — the classic seqlock writer-side flag that the earlier single-stamp design
/// (write payload then bump stamp) was missing.
struct Slot<T> {
    /// Seqlock version stamp (see the encoding above). `Release`d by the producer
    /// around the payload write; `Acquire`d by readers before and after copying.
    stamp: AtomicU64,
    /// Single-writer payload. Read concurrently by many consumers; the seqlock
    /// stamp protocol makes the `Copy`-out race-free for `T: Copy`.
    value: UnsafeCell<T>,
}

/// Sentinel stamp for a never-written slot: an odd value (so it reads as
/// "in-progress" / never a stable `(seq << 1)`) that no real sequence produces.
const UNWRITTEN: u64 = u64::MAX;

// SAFETY: `Slot<T>` is shared across threads inside `Inner`. The only mutable
// access to `value` is by the single producer, ordered with consumers by the
// `seq` seqlock (Release on publish / Acquire on read, plus a second Acquire to
// detect a concurrent overwrite). `T: Copy + Send` guarantees the payload bytes
// are trivially movable across threads and carry no thread-affine ownership.
#[allow(unsafe_code)]
unsafe impl<T: Send> Sync for Slot<T> {}

/// Shared ring storage; held immutably by the producer and every consumer.
struct Inner<T> {
    /// Pre-allocated, never resized after construction.
    slots: Box<[Slot<T>]>,
    /// `capacity - 1`, capacity being a power of two; for `index = seq & mask`.
    mask: u64,
    /// The highest published sequence **plus one** = number of items produced so
    /// far. `0` means nothing published. `Release`d by the producer after a slot
    /// is fully written; `Acquire`d by consumers to learn what is available and
    /// to compute the lapped/oldest-live frontier. Cache-padded to avoid
    /// false-sharing with the slot array hot lines.
    head: CachePadded<AtomicU64>,
}

impl<T: Copy + Default + Send> Inner<T> {
    fn new(capacity: usize) -> Arc<Self> {
        assert!(
            capacity.is_power_of_two(),
            "capacity must be a power of two"
        );
        assert!(capacity >= 2, "capacity must be at least 2");
        let slots: Box<[Slot<T>]> = (0..capacity)
            .map(|_| Slot {
                stamp: AtomicU64::new(UNWRITTEN),
                value: UnsafeCell::new(T::default()),
            })
            .collect();
        Arc::new(Self {
            slots,
            mask: (capacity as u64) - 1,
            head: CachePadded::new(AtomicU64::new(0)),
        })
    }

    #[inline]
    fn capacity(&self) -> u64 {
        self.mask + 1
    }
}

/// The single producer end. Publishes a monotonic sequence of items. Not
/// `Clone`: there is exactly one producer.
pub struct Producer<T> {
    inner: Arc<Inner<T>>,
    /// Next sequence number to assign (mirrors `inner.head` but owned by the
    /// single producer so the hot path needs no read-modify-write on `head`).
    next_seq: u64,
}

// The producer holds the `Arc` and writes payloads/stamps; `Send` so the
// producer can be moved onto its own (core-pinned) thread.
// Not `Sync` and not `Clone` — single producer by construction.

impl<T: Copy + Default + Send> Producer<T> {
    /// Publish `item` as the next sequence number.
    ///
    /// **Zero-allocation, lock-free, wait-free for the producer.** Overwrites the
    /// slot `next_seq & mask` (conflating any consumer that had not yet read the
    /// previous occupant of that slot — see the crate-level overflow policy),
    /// then advances the published head. Always succeeds (the bounded ring never
    /// back-pressures the producer — the FX-streaming-correct choice).
    #[inline]
    pub fn publish(&mut self, item: T) {
        let seq = self.next_seq;
        let idx = (seq & self.inner.mask) as usize;
        let slot = &self.inner.slots[idx];

        // True seqlock writer protocol (single producer ⇒ no writer-writer race):
        //   1. raise the in-progress flag for `seq` BEFORE touching the payload,
        //      with Release so any reader that later sees the stable stamp also
        //      sees a fully-written payload, and any reader copying concurrently
        //      observes an odd stamp and rejects the read;
        //   2. write the payload;
        //   3. publish the stable stamp `(seq << 1)` with Release.
        // The two stamp stores straddle the payload write, so a reader's
        // pre/post-copy stamp comparison detects ANY overlap with this write
        // (either it sees the odd in-progress stamp, or its before/after stamps
        // differ) — closing the torn-read window the single-store scheme left open.
        let writing = (seq << 1) | 1;
        let stable = seq << 1;
        slot.stamp.store(writing, Ordering::Release);
        // SAFETY: single producer ⇒ exclusive mutable access to this slot's cell.
        // Concurrent readers are fenced out by the odd `writing` stamp just stored
        // (Release) and the seqlock re-check; `idx` is in-bounds (`& mask`).
        #[allow(unsafe_code)]
        // SAFETY: exclusive single-producer write, in-bounds index.
        unsafe {
            *slot.value.get() = item;
        }
        slot.stamp.store(stable, Ordering::Release);

        // Advance the global head LAST (Release), so a consumer that observes
        // `head > seq` is guaranteed (Acquire on head ⇒ happens-after the slot's
        // Release stamp store) to also be able to observe the slot's stable stamp
        // and payload for `seq`.
        self.next_seq = seq + 1;
        self.inner.head.store(self.next_seq, Ordering::Release);
    }

    /// Total items published so far (= the next sequence number).
    #[inline]
    pub fn published(&self) -> u64 {
        self.next_seq
    }

    /// Ring capacity (number of slots).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity() as usize
    }

    /// Create another independent consumer for this ring, starting from the
    /// current head (it will see only items published from now on).
    pub fn subscribe_from_head(&self) -> Consumer<T> {
        let head = self.inner.head.load(Ordering::Acquire);
        Consumer {
            inner: Arc::clone(&self.inner),
            cursor: head,
            received: 0,
            skipped: 0,
        }
    }

    /// Create another independent consumer starting from sequence `0` (it will
    /// see the entire published history that still fits in the ring; older items
    /// already overwritten are accounted as initial skips on first read).
    pub fn subscribe_from_start(&self) -> Consumer<T> {
        Consumer {
            inner: Arc::clone(&self.inner),
            cursor: 0,
            received: 0,
            skipped: 0,
        }
    }
}

/// One independent consumer. Holds its own read cursor and skip counter; reading
/// never contends with the producer or with other consumers (other than the
/// unavoidable cache traffic on the shared slot lines).
pub struct Consumer<T> {
    inner: Arc<Inner<T>>,
    /// Next global sequence this consumer wants to read.
    cursor: u64,
    /// Items successfully delivered to this consumer.
    received: u64,
    /// Items conflated away (overwritten before this consumer read them).
    skipped: u64,
}

// SAFETY (Send): a `Consumer` only ever reads the shared ring (via the seqlock)
// and mutates its own `cursor`/counters, so it can be moved to another thread.
// It is intentionally NOT `Sync` (each consumer is single-threaded; clone for a
// second thread). `T: Send` required to move payloads across threads.
#[allow(unsafe_code)]
unsafe impl<T: Send> Send for Consumer<T> {}

impl<T: Copy + Default + Send> Consumer<T> {
    /// Try to receive the next item in this consumer's sequence.
    ///
    /// Returns:
    /// * `Ok(item)` — the next in-order item (cursor advanced by one, `received`
    ///   incremented).
    /// * `Err(RecvError::Empty)` — nothing new past the cursor yet.
    ///
    /// **Conflation:** if the producer has lapped this consumer (overwritten the
    /// slot at the cursor with a *later* sequence), the cursor is fast-forwarded
    /// to the **oldest still-live** sequence, the skipped gap is added to
    /// `skipped`, and that oldest-live item is returned. Thus a slow consumer
    /// always converges on the latest data with exact skip accounting:
    /// `received + skipped == produced-observed`.
    ///
    /// Zero-allocation, lock-free.
    #[inline]
    pub fn try_recv(&mut self) -> Result<T, RecvError> {
        let head = self.inner.head.load(Ordering::Acquire);
        if self.cursor >= head {
            return Err(RecvError::Empty);
        }
        let capacity = self.inner.capacity();

        // Oldest sequence still resident in the ring: head holds sequences in the
        // half-open window [head - capacity, head). If our cursor is older than
        // that window's start, those items were overwritten (conflated).
        let oldest_live = head.saturating_sub(capacity);
        if self.cursor < oldest_live {
            let gap = oldest_live - self.cursor;
            self.skipped += gap;
            self.cursor = oldest_live;
        }

        // Read the slot for `cursor` with the seqlock protocol. Loop because a
        // concurrent overwrite (the producer lapping us *during* this read) is
        // possible for the very newest in-window slots; on a torn read we
        // recompute the live window and retry.
        loop {
            let seq = self.cursor;
            let idx = (seq & self.inner.mask) as usize;
            let slot = &self.inner.slots[idx];
            // The stable stamp that means "this slot holds exactly global seq".
            let want = seq << 1;

            let stamp_before = slot.stamp.load(Ordering::Acquire);
            if stamp_before != want {
                // The slot does not stably hold `seq`. Cases:
                //  * in-progress write (low bit set) of `seq` or a later seq;
                //  * already lapped (stamp encodes a sequence != seq);
                //  * still `UNWRITTEN`.
                // Re-derive the live frontier and either lap forward, report
                // empty, or spin-retry a transient mid-write of our own slot.
                let head2 = self.inner.head.load(Ordering::Acquire);
                let oldest_live2 = head2.saturating_sub(capacity);
                if self.cursor < oldest_live2 {
                    self.skipped += oldest_live2 - self.cursor;
                    self.cursor = oldest_live2;
                    continue;
                }
                if self.cursor >= head2 {
                    return Err(RecvError::Empty);
                }
                // In-window but not yet stable: the producer is mid-write of this
                // exact slot. Spin-retry the same cursor.
                std::hint::spin_loop();
                continue;
            }

            // SAFETY: `T: Copy`; we read the payload bytes out of the cell. The
            // pre/post seqlock stamp check guarantees the producer did not write
            // this slot during the copy (any overlap flips the stamp to the odd
            // in-progress marker or to a later sequence), so the returned value is
            // a coherent, never-torn snapshot of sequence `seq`.
            #[allow(unsafe_code)]
            // SAFETY: in-bounds index; `Copy` read guarded by the seqlock stamps.
            let value = unsafe { *slot.value.get() };

            // Seqlock reader barrier (canonical form): an Acquire fence between the
            // plain payload copy and the post-stamp re-check. Without it, on a
            // weakly-ordered architecture (e.g. aarch64) the payload read above is
            // a plain load that the CPU may reorder PAST the `stamp_after` load —
            // letting a concurrent producer overwrite of this slot land inside the
            // copy window yet still be validated by a stale-but-equal `stamp_after`,
            // i.e. an undetected torn read. The fence pins the payload read before
            // the re-check so any overlapping write is always observed as a stamp
            // change and the read is retried. (Surfaced once under 16x full-suite
            // CPU oversubscription by the conflation stress test; the two-Acquire-
            // load form alone left this reorder window open.)
            std::sync::atomic::fence(Ordering::Acquire);
            let stamp_after = slot.stamp.load(Ordering::Acquire);
            if stamp_after != want {
                // Producer began (or finished) overwriting this slot while we
                // copied → torn read. Discard and retry (likely laps us forward).
                std::hint::spin_loop();
                continue;
            }

            // Clean, stable read of sequence `seq`.
            self.cursor = seq + 1;
            self.received += 1;
            return Ok(value);
        }
    }

    /// Items successfully delivered to this consumer.
    #[inline]
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Items conflated away (overwritten before this consumer could read them).
    /// Together with [`received`](Self::received): `received + skipped` equals the
    /// number of produced items this consumer has reached in the sequence.
    #[inline]
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// This consumer's next-to-read sequence number.
    #[inline]
    pub fn cursor(&self) -> u64 {
        self.cursor
    }
}

impl<T: Copy + Default + Send> Clone for Consumer<T> {
    /// A clone is an **independent** consumer at the same cursor (it will read the
    /// same future sequence the original would). Skip/received counters reset —
    /// they are per-consumer-instance metrics.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            cursor: self.cursor,
            received: 0,
            skipped: 0,
        }
    }
}

/// A constructed SPMC broadcast ring: the producer plus a factory for consumers.
///
/// Construct with [`BroadcastRing::new`], then take the single [`Producer`] and
/// spawn as many [`Consumer`]s as you need.
pub struct BroadcastRing<T> {
    producer: Producer<T>,
}

impl<T: Copy + Default + Send> BroadcastRing<T> {
    /// Build a ring with `capacity` slots (rounded up to a power of two, min 2).
    /// All storage is allocated here, once; nothing allocates afterward.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two().max(2);
        let inner = Inner::new(capacity);
        Self {
            producer: Producer { inner, next_seq: 0 },
        }
    }

    /// Consume the ring, returning its single producer. Create consumers via the
    /// producer's `subscribe_*` methods (or [`Self::consumer`] before taking it).
    pub fn into_producer(self) -> Producer<T> {
        self.producer
    }

    /// Borrow the producer.
    pub fn producer(&mut self) -> &mut Producer<T> {
        &mut self.producer
    }

    /// Create an independent consumer starting from sequence `0`.
    pub fn consumer(&self) -> Consumer<T> {
        self.producer.subscribe_from_start()
    }

    /// Ring capacity (number of slots).
    pub fn capacity(&self) -> usize {
        self.producer.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_rounds_up_to_power_of_two() {
        let r = BroadcastRing::<u64>::new(100);
        assert_eq!(r.capacity(), 128);
        let r = BroadcastRing::<u64>::new(1024);
        assert_eq!(r.capacity(), 1024);
        let r = BroadcastRing::<u64>::new(1);
        assert_eq!(r.capacity(), 2);
    }

    #[test]
    fn single_consumer_in_window_sees_every_item_in_order() {
        let mut ring = BroadcastRing::<u64>::new(64);
        let mut c = ring.consumer();
        let p = ring.producer();
        // Publish fewer than capacity so nothing is conflated.
        for i in 0..50u64 {
            p.publish(i * 7);
        }
        let mut got = Vec::new();
        while let Ok(v) = c.try_recv() {
            got.push(v);
        }
        let want: Vec<u64> = (0..50u64).map(|i| i * 7).collect();
        assert_eq!(got, want);
        assert_eq!(c.received(), 50);
        assert_eq!(c.skipped(), 0);
    }

    #[test]
    fn empty_when_nothing_new() {
        let mut ring = BroadcastRing::<u64>::new(8);
        let mut c = ring.consumer();
        assert_eq!(c.try_recv(), Err(RecvError::Empty));
        ring.producer().publish(42);
        assert_eq!(c.try_recv(), Ok(42));
        assert_eq!(c.try_recv(), Err(RecvError::Empty));
    }

    #[test]
    fn lapped_consumer_conflates_with_exact_skip_accounting() {
        let cap = 8usize;
        let mut ring = BroadcastRing::<u64>::new(cap);
        let mut c = ring.consumer();
        let p = ring.producer();
        // Publish far more than capacity WITHOUT the consumer reading.
        const PRODUCED: u64 = 1000;
        for i in 0..PRODUCED {
            p.publish(i);
        }
        // Drain everything the consumer can now see.
        let mut got = Vec::new();
        while let Ok(v) = c.try_recv() {
            got.push(v);
        }
        // It must converge on the LATEST item.
        assert_eq!(*got.last().unwrap(), PRODUCED - 1);
        // Exact accounting: received + skipped == produced (this consumer reached
        // the full sequence).
        assert_eq!(c.received() + c.skipped(), PRODUCED);
        assert_eq!(c.received(), got.len() as u64);
        // It saw at most `capacity` live items (the ring window), in order.
        assert!(got.len() <= cap);
        for w in got.windows(2) {
            assert!(w[0] < w[1], "delivered items must be strictly in order");
        }
    }

    #[test]
    fn subscribe_from_head_skips_history() {
        let mut ring = BroadcastRing::<u64>::new(16);
        ring.producer().publish(1);
        ring.producer().publish(2);
        let mut c = ring.producer().subscribe_from_head();
        ring.producer().publish(3);
        assert_eq!(c.try_recv(), Ok(3));
        assert_eq!(c.try_recv(), Err(RecvError::Empty));
        assert_eq!(c.skipped(), 0);
    }
}
