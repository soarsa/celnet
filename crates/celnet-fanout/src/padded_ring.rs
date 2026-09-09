//! Cache-line-padded and decoupled SPMC broadcast rings.
//!
//! Grounded in PPoPP concurrent queue literature (Morrison & Afek LCRQ).
//!
//! Eliminates false sharing and MESI cache-line invalidation storms across
//! multi-core consumer threads:
//! 1. Every slot is strictly aligned to a 64-byte boundary via `#[repr(align(64))]`.
//! 2. No two adjacent slots ever reside on the same CPU cache line.

use crate::mem::{Arc, AtomicU64, Ordering, PayloadCell, fence, spin_loop};
use crate::ring::RecvError;

use crossbeam_utils::CachePadded;

const UNWRITTEN: u64 = u64::MAX;

/// One ring slot, strictly aligned and padded to 64 bytes to eliminate false sharing.
#[repr(align(64))]
struct PaddedSlot<T> {
    stamp: AtomicU64,
    value: PayloadCell<T>,
}

struct Inner<T> {
    slots: Box<[PaddedSlot<T>]>,
    mask: u64,
    head: CachePadded<AtomicU64>,
}

impl<T: Copy + Default + Send> Inner<T> {
    fn new(capacity: usize) -> Arc<Self> {
        assert!(capacity.is_power_of_two(), "capacity must be a power of two");
        assert!(capacity >= 2, "capacity must be at least 2");

        let slots: Box<[PaddedSlot<T>]> = (0..capacity)
            .map(|_| PaddedSlot {
                stamp: AtomicU64::new(UNWRITTEN),
                value: PayloadCell::new(T::default()),
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

/// A 64-byte cache-line-padded single-producer / multi-consumer broadcast ring.
pub struct CachePaddedBroadcastRing<T> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Copy + Default + Send> CachePaddedBroadcastRing<T> {
    /// Create a new cache-padded broadcast ring with the specified capacity.
    pub fn new(capacity: usize) -> (CachePaddedProducer<T>, CachePaddedConsumer<T>) {
        let inner = Inner::new(capacity);
        let producer = CachePaddedProducer {
            inner: Arc::clone(&inner),
            next_seq: 0,
        };
        let consumer = CachePaddedConsumer {
            inner,
            cursor: 0,
            received: 0,
            skipped: 0,
        };
        (producer, consumer)
    }
}

/// The single producer end of a cache-padded broadcast ring.
pub struct CachePaddedProducer<T> {
    inner: Arc<Inner<T>>,
    next_seq: u64,
}

impl<T: Copy + Default + Send> CachePaddedProducer<T> {
    /// Publish `item` as the next sequence number without false sharing.
    #[inline]
    pub fn publish(&mut self, item: T) {
        let seq = self.next_seq;
        let idx = (seq & self.inner.mask) as usize;
        let slot = &self.inner.slots[idx];

        let writing = (seq << 1) | 1;
        let stable = seq << 1;
        slot.stamp.store(writing, Ordering::Release);

        #[allow(unsafe_code)]
        // SAFETY: exclusive single-producer write to this slot.
        unsafe {
            slot.value.write(item);
        }
        slot.stamp.store(stable, Ordering::Release);

        self.next_seq = seq + 1;
        self.inner.head.store(self.next_seq, Ordering::Release);
    }

    /// Total items published so far.
    #[inline]
    pub fn published(&self) -> u64 {
        self.next_seq
    }

    /// Ring capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity() as usize
    }

    /// Create another independent consumer subscribing from head.
    pub fn subscribe_from_head(&self) -> CachePaddedConsumer<T> {
        let head = self.inner.head.load(Ordering::Acquire);
        CachePaddedConsumer {
            inner: Arc::clone(&self.inner),
            cursor: head,
            received: 0,
            skipped: 0,
        }
    }

    /// Create another independent consumer subscribing from start.
    pub fn subscribe_from_start(&self) -> CachePaddedConsumer<T> {
        CachePaddedConsumer {
            inner: Arc::clone(&self.inner),
            cursor: 0,
            received: 0,
            skipped: 0,
        }
    }
}

/// One independent consumer for the cache-padded broadcast ring.
pub struct CachePaddedConsumer<T> {
    inner: Arc<Inner<T>>,
    cursor: u64,
    received: u64,
    skipped: u64,
}

#[allow(unsafe_code)]
unsafe impl<T: Send> Send for CachePaddedConsumer<T> {}

impl<T: Copy + Default + Send> Clone for CachePaddedConsumer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            cursor: self.cursor,
            received: self.received,
            skipped: self.skipped,
        }
    }
}

impl<T: Copy + Default + Send> CachePaddedConsumer<T> {
    /// Fork a new independent consumer for this ring, starting from the current head.
    pub fn fork_from_head(&self) -> Self {
        let head = self.inner.head.load(Ordering::Acquire);
        Self {
            inner: Arc::clone(&self.inner),
            cursor: head,
            received: 0,
            skipped: 0,
        }
    }

    /// Try to receive the next item in this consumer's sequence.
    #[inline]
    pub fn try_recv(&mut self) -> Result<T, RecvError> {
        let head = self.inner.head.load(Ordering::Acquire);
        if self.cursor >= head {
            return Err(RecvError::Empty);
        }
        let capacity = self.inner.capacity();

        let oldest_live = head.saturating_sub(capacity);
        if self.cursor < oldest_live {
            let gap = oldest_live - self.cursor;
            self.skipped += gap;
            self.cursor = oldest_live;
        }

        loop {
            let seq = self.cursor;
            let idx = (seq & self.inner.mask) as usize;
            let slot = &self.inner.slots[idx];
            let want = seq << 1;

            let stamp_before = slot.stamp.load(Ordering::Acquire);
            if stamp_before != want {
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
                spin_loop();
                continue;
            }

            #[allow(unsafe_code)]
            // SAFETY: seqlock stamp protocol checks before and after copy.
            let item = unsafe { slot.value.read() };
            fence(Ordering::Acquire);

            let stamp_after = slot.stamp.load(Ordering::Acquire);
            if stamp_after != want {
                spin_loop();
                continue;
            }

            self.cursor = seq + 1;
            self.received += 1;
            return Ok(item);
        }
    }

    /// Number of successfully delivered items.
    #[inline]
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Number of conflated (skipped) items.
    #[inline]
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Next sequence number this consumer will attempt to read.
    #[inline]
    pub fn cursor(&self) -> u64 {
        self.cursor
    }
}
