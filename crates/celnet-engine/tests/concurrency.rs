//! Concurrency tests for the lock-free runtime primitives (§3.2):
//! the seqlock returns a *consistent* snapshot under concurrent writes, and the
//! `arc-swap` state publication is observed by a reader with no locking.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use celnet_engine::rt::{PriceSnapshot, Seqlock, StateHandle};
use celnet_engine::testing::{consistent_pair, make_state};

/// Under a hammering single writer, every snapshot a reader observes is one the
/// writer actually published — never a torn mix of two. The invariant is encoded
/// structurally: the writer always stores a snapshot satisfying a fixed relation
/// between its fields (`delta_spot == request_id`, `vega == request_id*2`); a
/// torn read would break that relation and fail the assertion.
#[test]
fn seqlock_reads_are_always_consistent() {
    // `store` takes `&self`, so one `Arc<Seqlock>` is shared by the (single)
    // writer thread and many reader threads — the textbook seqlock deployment.
    let lock = Arc::new(Seqlock::new(PriceSnapshot::default()));
    let stop = Arc::new(AtomicBool::new(false));

    let writer_lock = Arc::clone(&lock);
    let writer_stop = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        let mut i: u64 = 1;
        while !writer_stop.load(Ordering::Relaxed) {
            writer_lock.store(consistent_pair(i));
            i = i.wrapping_add(1);
        }
    });

    let mut readers = Vec::new();
    for _ in 0..4 {
        let rlock = Arc::clone(&lock);
        readers.push(thread::spawn(move || {
            for _ in 0..500_000 {
                let s = rlock.read();
                if s.request_id != 0 {
                    assert_eq!(
                        s.delta_spot, s.request_id as f64,
                        "torn read: delta_spot != request_id"
                    );
                    assert_eq!(
                        s.vega,
                        s.request_id as f64 * 2.0,
                        "torn read: vega != request_id*2"
                    );
                }
            }
        }));
    }

    for r in readers {
        r.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
}

/// A reader observes a republished `arc-swap` state with no locking, and every
/// observed state is a coherent whole (atomic all-or-nothing publish).
#[test]
fn arc_swap_state_observed_without_locking() {
    let conv = celnet_conventions::resolve(
        celnet_types::CcyPair::parse("EURUSD").unwrap(),
        celnet_types::Tenor::Years(1),
    )
    .record;
    let handle = Arc::new(StateHandle::new(make_state(1.10, conv)));
    let stop = Arc::new(AtomicBool::new(false));

    let whandle = Arc::clone(&handle);
    let wstop = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        let mut spot = 1.10f64;
        while !wstop.load(Ordering::Relaxed) {
            spot += 0.001;
            if spot > 2.0 {
                spot = 1.10;
            }
            whandle.publish(make_state(spot, conv));
        }
    });

    let rhandle = Arc::clone(&handle);
    for _ in 0..500_000 {
        // Lock-free load; the whole `MarketState` is published atomically.
        let st = rhandle.load();
        assert!(st.spot.is_finite() && st.spot > 0.0);
        let f = st.forward();
        assert!(f.is_finite() && f > 0.0);
    }

    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
}
