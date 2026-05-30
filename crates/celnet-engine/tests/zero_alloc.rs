//! Zero-allocation proof for the hot pricing loop (`docs/ARCHITECTURE.md` §3.3).
//!
//! The hot path must **acquire no memory**: per §3.3 all pools are pre-sized at
//! startup and the steady-state pricing loop never `alloc`/`realloc`s, syscalls,
//! locks, or logs. We prove this directly with a counting global allocator that
//! wraps the system allocator and tallies acquiring ops (`alloc`/`realloc`)
//! separately from `dealloc`. We pre-build all state, snapshot the counters,
//! price a large batch of requests through the SPSC ring + pricing core, and
//! assert the acquiring count did not move.
//!
//! Two proofs:
//!  * [`hot_pricing_loop_allocates_zero`] — the steady-state single-thread loop.
//!  * [`hot_pricing_under_concurrent_publish_allocates_zero`] — the production
//!    scenario where a publisher hammers `StateHandle::publish` while the core
//!    prices. The acquiring count stays zero; superseded states are *freed*
//!    (a `dealloc`), never allocated. That reclamation-happens-on-the-reader proof
//!    is made **deterministic** in a separate single-threaded armed micro-window
//!    (not left to publisher-thread timing), so the test is not flaky.
//!
//! # Determinism of the counters (no cross-test interference)
//!
//! The allocator counters are process-global, so two tests in this binary that both
//! `arm` would corrupt each other's deltas. These tests are therefore placed in a
//! serialized nextest test-group (`.config/nextest.toml`, group `engine-serial`) so
//! they never co-schedule, and counting is gated to the **armed thread only** via a
//! thread-local so a parallel non-armed thread's allocations are never tallied.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use celnet_engine::core::{PriceRequest, PricingCore};
use celnet_engine::rt::{request_ring, response_ring};
use celnet_surface::{MarketContext, MarketQuotes, build_smile};
use celnet_types::{CcyPair, OptionType, Tenor};

/// A global allocator that delegates to the system allocator while counting the
/// number of allocation-shaped operations performed **by the armed thread only**.
///
/// The thread-local `ARMED` gate lets the concurrent-publish test count
/// allocations on the pricing thread alone while a publisher thread (which is
/// permitted to allocate) runs in parallel: the publisher's thread never arms
/// the gate, so its allocations do not inflate the count.
struct CountingAlloc;

/// Counts **memory-acquiring** operations only (`alloc` + `realloc`). These are
/// the operations the zero-allocation guarantee forbids on the hot path.
static ALLOCS: AtomicU64 = AtomicU64::new(0);

/// Counts `dealloc` separately. Freeing a *superseded* immutable published
/// `MarketState` (its `Arc` refcount reaching zero after a tick) is correct,
/// expected reclamation — it acquires no memory — so it is tracked but **not**
/// part of the zero-allocation assertion. The original single-threaded proof
/// (no concurrent publish) sees zero of either; the concurrent-publish proof
/// sees zero allocs but a bounded number of these reclamation deallocs.
static DEALLOCS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// When `true`, allocator operations on *this* thread are tallied.
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` with allocation counting armed on the current thread, restoring the
/// previous state afterwards.
fn armed<R>(f: impl FnOnce() -> R) -> R {
    let prev = ARMED.with(|a| a.replace(true));
    let r = f();
    ARMED.with(|a| a.set(prev));
    r
}

fn count_alloc() {
    // `try_with` so an allocation during thread-local teardown cannot panic.
    let _ = ARMED.try_with(|a| {
        if a.get() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
    });
}

fn count_dealloc() {
    let _ = ARMED.try_with(|a| {
        if a.get() {
            DEALLOCS.fetch_add(1, Ordering::Relaxed);
        }
    });
}

// SAFETY: every method forwards verbatim to `System`, the platform allocator,
// with the identical `Layout`/`ptr` arguments; the only added behavior is a
// thread-gated counter increment, which cannot affect allocation soundness.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_alloc();
        // SAFETY: forwarding an unchanged layout to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count_dealloc();
        // SAFETY: `ptr`/`layout` came from a prior `System.alloc` with this
        // layout (same global allocator), satisfying `dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // realloc may *grow* memory; treat it as an acquiring op.
        count_alloc();
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
    armed(|| {
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
    });

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

/// Regression for the audit major: the zero-alloc property must hold on the
/// *production* scenario — pricing while a publisher concurrently hammers
/// `StateHandle::publish` (a market tick / recalibration landing mid-pricing).
/// This exercises `arc-swap`'s read side (`load_full`) under concurrent `store`,
/// the exact path the original single-threaded proof never covered, plus the
/// per-word seqlock `store` on the hot path.
///
/// The publisher publishes into the **core's own** state handle
/// (`core.shared_state()`, an `Arc`-backed clone of the same storage the hot
/// loop reads through its cached reader) — so the contention is genuine. We
/// arm the allocation counter only on the pricing thread; the publisher thread
/// is permitted to allocate (it `store`s pre-built `Arc<MarketState>` clones,
/// which is exactly the live-tick churn we want to run *against* the hot loop),
/// and its allocations are excluded by the thread-local gate.
#[test]
fn hot_pricing_under_concurrent_publish_allocates_zero() {
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    // Pre-build a pool of states the publisher cycles through (startup).
    let states: Vec<celnet_engine::rt::MarketState> = (0..16)
        .map(|i| {
            let spot = 1.05 + 0.01 * f64::from(i);
            let ctx = MarketContext::new(spot, 0.02, 0.01, 1.0, conv);
            let q = MarketQuotes::three_point(0.105, 0.015, 0.0035);
            let smile = build_smile(&ctx, &q).expect("calibration converges");
            celnet_engine::rt::MarketState {
                spot,
                r_dom: 0.02,
                r_for: 0.01,
                t: 1.0,
                conventions: conv,
                smile,
            }
        })
        .collect();

    let mut core = PricingCore::new(states[0].clone());
    let (mut req_tx, mut req_rx) = request_ring();
    let (mut resp_tx, mut resp_rx) = response_ring();
    let stop = AtomicBool::new(false);

    const N: usize = 4096;
    let strikes: Vec<f64> = (0..N)
        .map(|i| 0.90 + 0.40 * (i as f64) / (N as f64))
        .collect();

    // Warm the path once (outside the measured window).
    let _ = core.price(PriceRequest::new(0, OptionType::Call, 1.10));

    // The publisher gets its own `Arc` to the core's handle (the production edge
    // wiring), independent of the `&mut core` drain borrow.
    let pub_handle = core.shared_state();
    // A second handle retained on this thread for the deterministic reclamation proof
    // after the publisher is joined.
    let prove_handle = core.shared_state();

    let mut total = 0usize;
    std::thread::scope(|s| {
        let stop_ref = &stop;
        let pub_states = &states;
        s.spawn(move || {
            let mut i = 0usize;
            while !stop_ref.load(Ordering::Relaxed) {
                pub_handle.publish(pub_states[i % pub_states.len()].clone());
                i += 1;
            }
        });

        let before_alloc = ALLOCS.load(Ordering::Relaxed);
        let before_dealloc = DEALLOCS.load(Ordering::Relaxed);
        // Measured hot window on the pricing thread only.
        armed(|| {
            let mut id = 0u64;
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
        });
        let after_alloc = ALLOCS.load(Ordering::Relaxed);

        stop.store(true, Ordering::Relaxed);

        assert_eq!(total, N);
        // The hard guarantee (UNWEAKENED): the hot pricing path acquired NO memory
        // (`alloc`/`realloc`) even while a publisher hammered `publish`.
        assert_eq!(
            after_alloc,
            before_alloc,
            "concurrent-publish hot path acquired memory {} times (must be zero)",
            after_alloc.wrapping_sub(before_alloc)
        );
        // `before_dealloc` was captured for the deterministic reclamation proof
        // below; it is intentionally not asserted against the racy concurrent
        // window (whether a supersession's free landed on the armed thread during
        // the window depends on publisher timing — the former flaky sub-assert).
        let _ = before_dealloc;
    });

    // ---- deterministic reclamation proof (no publisher-timing race) ----------
    // Prove the read path *frees* a superseded state ON THE READING THREAD — the
    // property the concurrent window relies on — without depending on a racy
    // publisher. We publish a NEW state from this (main) thread while DISARMED, then
    // arm and `price` once: the cached reader revalidates, swaps to the new state,
    // and drops the previously-cached `Arc` on this armed thread, which the counter
    // records as exactly the reclamation `dealloc` we expect. Acquiring ops still
    // stay zero across this armed step (the guarantee holds here too).
    let fresh = states[1].clone(); // clone DISARMED (startup-class allocation).
    prove_handle.publish(fresh); // publish DISARMED (the publisher's job allocates).
    let before_dealloc = DEALLOCS.load(Ordering::Relaxed);
    let before_alloc = ALLOCS.load(Ordering::Relaxed);
    armed(|| {
        // One priced read: revalidates the cache against the just-published pointer,
        // adopts it, and drops the prior cached `Arc` here → a reclamation dealloc.
        let _ = core.price(PriceRequest::new(N as u64, OptionType::Call, 1.10));
    });
    let after_dealloc = DEALLOCS.load(Ordering::Relaxed);
    let after_alloc = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(
        after_alloc, before_alloc,
        "the reclaiming read still acquired no memory (zero-alloc holds)"
    );
    // The superseded `Arc<MarketState>` (and the smile data it owns) is freed on the
    // reading thread: at least one reclamation dealloc, deterministically — never the
    // former race on whether the publisher's churn happened to free during the window.
    let reclaimed = after_dealloc.wrapping_sub(before_dealloc);
    assert!(
        reclaimed >= 1,
        "a superseded state must be reclaimed on the reading thread \
         (deterministic; no publisher-timing race); saw {reclaimed}"
    );
}
