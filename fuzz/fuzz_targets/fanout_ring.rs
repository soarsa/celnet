//! Fuzz target: operational property-fuzz of the SPMC broadcast ring
//! (`celnet_fanout::BroadcastRing`).
//!
//! Unlike the untrusted-byte decoders, this ring has no wire format — its attack
//! surface is the **interleaving** of `publish` / `try_recv` against a given
//! capacity. The fuzzer turns its arbitrary bytes into a capacity (a power of two
//! in `[2, 256]`) and a sequence of operations, then drives the real ring
//! single-threaded and asserts the conflation contract end to end. The payload we
//! publish *is* its own global sequence number (`payload == produced`), so every
//! delivered value carries the ground-truth sequence it must equal — letting us
//! check ordering, the live window, and exact skip accounting deterministically.
//!
//! Invariants asserted (the same the gates assert, now over fuzzer-chosen
//! schedules — this is the property surface the mutation gate hardens):
//!   * **No panic / no torn value:** every `try_recv` returns `Ok`/`Empty`.
//!   * **In-order, never-duplicated:** delivered sequences strictly increase.
//!   * **Live window:** a delivered sequence `v` satisfies
//!     `oldest_live <= v < head`, i.e. it was resident in the ring (never stale,
//!     never ahead of the producer).
//!   * **Conservation (per op):** `cursor == received + skipped` and
//!     `cursor <= produced`.
//!   * **Convergence:** after a full drain the consumer has caught up
//!     (`cursor == produced`) and `received + skipped == produced`.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run fanout_ring -- -max_total_time=120

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use celnet_fanout::{BroadcastRing, RecvError};

#[derive(Arbitrary, Debug)]
enum Op {
    /// Publish the next sequence number as the payload.
    Publish,
    /// Receive a single item (or observe `Empty`).
    Recv,
    /// Drain every currently-available item.
    DrainAll,
}

#[derive(Arbitrary, Debug)]
struct Scenario {
    /// Selects capacity `2^(1 + cap_sel % 8)` ∈ {2, 4, …, 256}.
    cap_sel: u8,
    ops: Vec<Op>,
}

/// Assert the per-delivery invariants for a value `v` (which equals the global
/// sequence we published as its own payload). Returns the new `last_delivered`.
#[inline]
fn check_delivery(
    v: u64,
    produced: u64,
    capacity: u64,
    last_delivered: Option<u64>,
) -> Option<u64> {
    let head = produced;
    let oldest_live = head.saturating_sub(capacity);
    assert!(
        v < head,
        "delivered seq {v} >= head {head} (ahead of producer)"
    );
    assert!(
        v >= oldest_live,
        "delivered seq {v} < oldest_live {oldest_live} (stale/torn read)"
    );
    if let Some(prev) = last_delivered {
        assert!(
            v > prev,
            "out-of-order or duplicate delivery: {v} after {prev}"
        );
    }
    Some(v)
}

fuzz_target!(|s: Scenario| {
    let cap = 1usize << (1 + (s.cap_sel % 8) as u32); // 2..=256, power of two
    let mut ring = BroadcastRing::<u64>::new(cap);
    let capacity = ring.capacity() as u64;
    let mut c = ring.consumer();

    let mut produced: u64 = 0;
    let mut last_delivered: Option<u64> = None;

    for op in s.ops {
        match op {
            Op::Publish => {
                ring.producer().publish(produced);
                produced += 1;
            }
            Op::Recv => match c.try_recv() {
                Ok(v) => last_delivered = check_delivery(v, produced, capacity, last_delivered),
                Err(RecvError::Empty) => {
                    // Empty ⇒ the consumer has consumed everything published.
                    assert!(
                        c.cursor() >= produced,
                        "Empty but cursor {} < produced {produced}",
                        c.cursor()
                    );
                }
            },
            Op::DrainAll => {
                while let Ok(v) = c.try_recv() {
                    last_delivered = check_delivery(v, produced, capacity, last_delivered);
                }
                assert_eq!(
                    c.cursor(),
                    produced,
                    "after DrainAll the consumer is not caught up"
                );
            }
        }

        // Conservation holds after every operation.
        assert_eq!(
            c.cursor(),
            c.received() + c.skipped(),
            "cursor {} != received {} + skipped {}",
            c.cursor(),
            c.received(),
            c.skipped()
        );
        assert!(
            c.cursor() <= produced,
            "cursor {} overran producer {produced}",
            c.cursor()
        );
    }

    // Final convergence: a full drain leaves the consumer exactly at the head with
    // every produced item either received or precisely accounted as skipped.
    while c.try_recv().is_ok() {}
    assert_eq!(c.cursor(), produced, "final cursor != produced");
    assert_eq!(
        c.received() + c.skipped(),
        produced,
        "conservation violated: received {} + skipped {} != produced {produced}",
        c.received(),
        c.skipped()
    );
});
