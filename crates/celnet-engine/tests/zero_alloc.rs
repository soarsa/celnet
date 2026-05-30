//! Zero-allocation proof for the hot pricing loop (`docs/ARCHITECTURE.md` §3.3).
//!
//! The hot path must perform **no heap allocation**: per §3.3 all pools are
//! pre-sized at startup and the steady-state pricing loop never allocates,
//! syscalls, locks, or logs. We prove this directly with a counting global
//! allocator that wraps the system allocator and tallies every `alloc` /
//! `realloc` / `dealloc`. We pre-build all state, snapshot the allocation count,
//! price a large batch of requests through the SPSC ring + pricing core, and
//! assert the count did not move.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_engine::core::{PriceRequest, PricingCore};
use celnet_engine::rt::{request_ring, response_ring};
use celnet_surface::{MarketContext, MarketQuotes, build_smile};
use celnet_types::{CcyPair, OptionType, Tenor};

/// A global allocator that delegates to the system allocator while counting the
/// number of allocation-shaped operations. Counting is `Relaxed` (we only read
/// it on the single test thread, between fences provided by the calls).
struct CountingAlloc;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method forwards verbatim to `System`, the platform allocator,
// with the identical `Layout`/`ptr` arguments; the only added behavior is an
// atomic counter increment, which cannot affect allocation soundness.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarding an unchanged layout to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `ptr`/`layout` came from a prior `System.alloc` with this
        // layout (same global allocator), satisfying `dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarding a valid `ptr`/`layout`/`new_size` to the system
        // allocator unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn build_core() -> PricingCore {
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    let ctx = MarketContext::new(1.10, 0.02, 0.01, 1.0, conv);
    let q = MarketQuotes::three_point(0.105, 0.015, 0.0035);
    let smile = build_smile(&ctx, &q).expect("calibration converges");
    let state = celnet_engine::rt::MarketState {
        spot: 1.10,
        r_dom: 0.02,
        r_for: 0.01,
        t: 1.0,
        conventions: conv,
        smile,
    };
    PricingCore::new(state)
}

#[test]
fn hot_pricing_loop_allocates_zero() {
    // Build EVERYTHING up front (these allocate; that's fine, it's startup).
    let mut core = build_core();
    let (mut req_tx, mut req_rx) = request_ring();
    let (mut resp_tx, mut resp_rx) = response_ring();

    // Pre-generate a strike schedule (startup), so the hot loop reads from a
    // fixed array and never builds anything.
    const N: usize = 4096;
    let strikes: Vec<f64> = (0..N)
        .map(|i| 0.90 + 0.40 * (i as f64) / (N as f64))
        .collect();

    // Warm the path once so any lazy one-time initialization (none expected) is
    // outside the measured window.
    let _ = core.price(PriceRequest::new(0, OptionType::Call, 1.10));
    let _ = resp_rx.pop();

    // --- measured hot window: submit -> drain/price -> consume, repeatedly ---
    let before = ALLOCS.load(Ordering::Relaxed);

    let mut total = 0usize;
    let mut id = 0u64;
    // Process in ring-sized chunks so neither ring ever overflows; each chunk is
    // pure wait-free push/pop + closed-form pricing — zero allocation.
    for chunk in strikes.chunks(2048) {
        for &k in chunk {
            req_tx
                .push(PriceRequest::new(id, OptionType::Call, k))
                .expect("request ring sized for the chunk");
            id += 1;
        }
        let priced = core.drain(&mut req_rx, &mut resp_tx, chunk.len());
        assert_eq!(priced, chunk.len());
        while resp_rx.pop().is_ok() {
            total += 1;
        }
    }

    let after = ALLOCS.load(Ordering::Relaxed);
    // --- end measured window ---

    assert_eq!(total, N, "every request must be priced and consumed");
    assert_eq!(
        after,
        before,
        "hot pricing loop allocated {} times (must be zero)",
        after - before
    );
    // Sanity: top-of-book reflects the last priced request.
    assert_eq!(core.top_of_book().request_id, (N - 1) as u64);
}
