//! Zero-allocation proof for the publish hot path.
//!
//! The broadcast ring's backing storage is allocated **once** at construction;
//! `publish` and `try_recv` only do atomic loads/stores and a `Copy` of the
//! payload into/out of a pre-existing slot. We prove `publish` (and a draining
//! `try_recv`) acquire **no** memory with a counting global allocator that wraps
//! the system allocator and tallies acquiring ops (`alloc`/`realloc`), gated to
//! the armed thread only (mirrors `celnet-engine/tests/zero_alloc.rs`).
//!
//! Counting is gated to the armed thread via a thread-local so any parallel
//! non-armed thread's allocations are never tallied; this binary's single
//! allocation test needs no nextest serialization.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_fanout::{BroadcastRing, Consumer, Producer};

struct CountingAlloc;

/// Memory-acquiring operations (`alloc` + `realloc`) on the armed thread.
static ALLOCS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn armed<R>(f: impl FnOnce() -> R) -> R {
    let prev = ARMED.with(|a| a.replace(true));
    let r = f();
    ARMED.with(|a| a.set(prev));
    r
}

fn count_alloc() {
    let _ = ARMED.try_with(|a| {
        if a.get() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
    });
}

// SAFETY: every method forwards verbatim to `System` with identical
// `Layout`/`ptr` arguments; the only added behavior is a thread-gated counter
// increment, which cannot affect allocation soundness.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_alloc();
        // SAFETY: forwarding an unchanged layout to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` came from a prior `System.alloc` (same global
        // allocator) with this layout, satisfying `dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_alloc();
        // SAFETY: forwarding a valid `ptr`/`layout`/`new_size` unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// A non-trivial `Copy` payload (price tick) so the proof covers a realistic
/// fan-out item, not just a `u64`.
#[derive(Clone, Copy, Default)]
struct PriceTick {
    seq: u64,
    bid: f64,
    ask: f64,
    pair: u32,
}

#[test]
fn publish_and_drain_allocate_zero() {
    // Build everything up front (allocates — startup, fine).
    let ring = BroadcastRing::<PriceTick>::new(1024);
    let mut producer: Producer<PriceTick> = ring.into_producer();
    let mut consumer: Consumer<PriceTick> = producer.subscribe_from_start();

    // Warm once (no lazy init expected).
    producer.publish(PriceTick {
        seq: 0,
        bid: 1.10,
        ask: 1.1001,
        pair: 1,
    });
    let _ = consumer.try_recv();

    const N: u64 = 1_000_000;
    let before = ALLOCS.load(Ordering::Relaxed);
    let mut drained = 0u64;
    // Accumulate over every field so the payload is genuinely consumed (proves
    // the `Copy`-out delivered real data, and keeps the fields live).
    let mut checksum = 0.0f64;
    let mut seq_sum = 0u64;
    armed(|| {
        for i in 0..N {
            producer.publish(PriceTick {
                seq: i,
                bid: 1.10 + (i as f64) * 1e-9,
                ask: 1.1001 + (i as f64) * 1e-9,
                pair: (i % 32) as u32,
            });
            // Interleave a drain so try_recv is on the armed hot path too.
            while let Ok(tick) = consumer.try_recv() {
                checksum += tick.bid + tick.ask + f64::from(tick.pair);
                seq_sum = seq_sum.wrapping_add(tick.seq);
                drained += 1;
            }
        }
    });
    let after = ALLOCS.load(Ordering::Relaxed);

    assert!(drained > 0, "consumer drained nothing — bad test");
    assert!(
        checksum.is_finite() && seq_sum > 0,
        "payload fields must be delivered intact"
    );
    assert_eq!(
        after,
        before,
        "publish/try_recv hot path allocated {} times (must be zero)",
        after - before
    );
}
