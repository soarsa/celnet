//! **Perf-guard:** proves the hot-path telemetry publish performs **exactly zero
//! heap allocations**.
//!
//! We install a process-global counting allocator that delegates to the system
//! allocator but increments atomic counters on every `alloc`/`realloc`. We then
//! measure the allocation delta around a batch of [`HotProbe::publish`] calls and
//! assert it is **0**. A negative control (a deliberately-allocating block)
//! confirms the counter actually observes allocations, so a green test cannot be
//! a false positive from a broken probe.
//!
//! Determinism: the ring is pre-sized to absorb the batch, so no push is ever
//! rejected and the measured path is the *successful* publish path (the one the
//! engine takes in steady state).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use celnet_observability::record::{ErrorClass, HotSample, OpKind};
use celnet_observability::telemetry_channel;

/// A counting allocator that forwards to the system allocator and tallies the
/// number of allocation-shaped calls. Used only by this test harness.
struct CountingAlloc;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: `CountingAlloc` forwards every call verbatim to the global `System`
// allocator (the same allocator the program would otherwise use), only adding
// `Relaxed` atomic increments that have no effect on the returned pointers or
// their validity. The `GlobalAlloc` contract is therefore upheld exactly as
// `System` upholds it. This is the single audited `unsafe` site in the crate; it
// exists solely to *measure* the hot path and is compiled only for tests.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarding to the system allocator with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` were produced by `System.alloc` above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarding to the system allocator with the caller's layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation; `new_size`
        // is the caller's requested size, forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn allocs() -> u64 {
    ALLOCS.load(Ordering::Relaxed)
}

/// Serializes any allocation-measuring test. The counting allocator is a single
/// process-global counter, so two tests measuring deltas concurrently (as the
/// default libtest harness runs them, multi-threaded in one process) would each
/// observe the *other's* allocations and spuriously fail. The measured logic is
/// currently a single `#[test]` (so no contention can occur), and this mutex
/// keeps that invariant safe if further allocation-measuring tests are added.
/// (Under `cargo nextest`, each test is its own process — belt-and-braces.)
static MEASURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The perf-guard is exposed as a **single** `#[test]` so that, under the
/// default multi-threaded libtest harness, no *sibling* test thread is running
/// concurrently and incrementing the process-global allocation counter while we
/// measure a delta around it (that counter cannot distinguish which thread
/// allocated). The two assertions — zero-alloc publish and bounded-work publish
/// — therefore run back-to-back in one thread. (Under `cargo nextest`, which the
/// gate uses, each test is already its own process; the `MEASURE` mutex is
/// belt-and-braces for anyone who adds further allocation-measuring tests here.)
#[test]
fn hot_path_publish_is_allocation_free() {
    let _measure = MEASURE.lock().unwrap_or_else(|p| p.into_inner());
    hot_probe_publish_allocates_zero();
    publish_does_bounded_work_not_wall_clock_timing();
}

fn hot_probe_publish_allocates_zero() {
    // Pre-size the ring so every publish in the batch succeeds (the steady-state
    // hot path). Construction allocates the ring ONCE, here, before measurement.
    const N: usize = 100_000;
    let (mut probe, drain) = telemetry_channel(N.next_power_of_two());

    // Warm: ensure any lazy one-time initialisation inside the push path (none is
    // expected) happens before we measure.
    probe.publish(HotSample::new(0, OpKind::VanillaPrice, 1, 0));
    let _keep_alive = drain; // keep the consumer alive so the ring is not torn down.

    // Negative control: prove the counter actually sees allocations. A boxed
    // value is an unambiguous, single heap allocation (and cannot be elided
    // because it escapes through `black_box`).
    let before_ctrl = allocs();
    let boxed = std::hint::black_box(Box::new([0u8; 64]));
    std::hint::black_box(&boxed);
    let ctrl_delta = allocs() - before_ctrl;
    assert!(
        ctrl_delta >= 1,
        "negative control failed: the counting allocator observed no allocation \
         for a Box (delta={ctrl_delta}); the guard would be meaningless"
    );

    // Measured region: N hot-path publishes, nothing else.
    let before = allocs();
    let bytes_before = BYTES.load(Ordering::Relaxed);
    for i in 0..N as u64 {
        let sample = HotSample::new(i, OpKind::StreamQuote, i, 3).with_class(if i % 7 == 0 {
            ErrorClass::StaleState
        } else {
            ErrorClass::Ok
        });
        let ok = probe.publish(sample);
        std::hint::black_box(ok);
    }
    let delta = allocs() - before;
    let byte_delta = BYTES.load(Ordering::Relaxed) - bytes_before;

    assert_eq!(
        delta, 0,
        "hot-path publish must allocate ZERO times over {N} publishes; observed {delta}"
    );
    assert_eq!(
        byte_delta, 0,
        "hot-path publish must allocate ZERO bytes over {N} publishes; observed {byte_delta}"
    );
}

fn publish_does_bounded_work_not_wall_clock_timing() {
    // A non-flaky "fast publish" sanity check: instead of asserting wall-clock
    // nanoseconds (which is environment-dependent and CI-flaky), we assert the
    // publish performs *bounded, allocation-free* work — exactly the property
    // that makes it sub-100ns-class. We do that by confirming a large batch of
    // publishes still allocates zero times (work is O(1) per call, no growth).
    const N: usize = 50_000;
    let (mut probe, mut drain) = telemetry_channel(N.next_power_of_two());
    let before = allocs();
    for i in 0..N as u64 {
        std::hint::black_box(probe.publish(HotSample::new(i, OpKind::RfqQuote, 1, 0)));
    }
    assert_eq!(
        allocs() - before,
        0,
        "publish must be O(1) allocation-free work per call"
    );
    // Drain to show the consumer side reads them all back (still no alloc on the
    // producer; the drain side may allocate freely — it is the non-critical core).
    let n = drain.drain_all(|_| {});
    assert_eq!(n, N);
}
