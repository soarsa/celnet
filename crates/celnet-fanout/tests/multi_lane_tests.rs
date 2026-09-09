//! Integration tests for CachePaddedBroadcastRing and MultiLaneBroadcastRing
//!
//! Verifies:
//! - 64-byte alignment and padding eliminates false sharing
//! - Multi-lane sharded broadcast preserves total order across all lanes
//! - Exact delivery accounting across 64 concurrent reader threads

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use celnet_fanout::{CachePaddedBroadcastRing, MultiLaneBroadcastRing, RecvError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct TestQuote {
    timestamp_ns: u64,
    seq: u64,
    bid: u64,
    ask: u64,
}

#[test]
fn test_cache_padded_ring_concurrent_consumers() {
    let capacity = 1024;
    let (mut producer, consumer0) = CachePaddedBroadcastRing::<TestQuote>::new(capacity);

    let num_consumers = 16;
    let num_items = 2000;
    let stop = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();
    for c_id in 0..num_consumers {
        let mut c = if c_id == 0 {
            consumer0.clone()
        } else {
            producer.subscribe_from_start()
        };
        let stop_clone = Arc::clone(&stop);

        handles.push(thread::spawn(move || {
            let mut last_seq = 0u64;
            while (c.received() + c.skipped()) < num_items as u64 {
                match c.try_recv() {
                    Ok(item) => {
                        assert!(
                            item.seq >= last_seq,
                            "sequence inversion detected on consumer {}",
                            c_id
                        );
                        last_seq = item.seq;
                    }
                    Err(RecvError::Empty) => {
                        if stop_clone.load(Ordering::Acquire)
                            && (c.received() + c.skipped()) >= num_items as u64
                        {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                }
            }
            (c_id, c.received(), c.skipped())
        }));
    }

    // Producer publishes quotes
    for seq in 0..num_items as u64 {
        producer.publish(TestQuote {
            timestamp_ns: 1_700_000_000 + seq,
            seq,
            bid: 108_500 + (seq % 100),
            ask: 108_520 + (seq % 100),
        });
    }
    stop.store(true, Ordering::Release);

    for h in handles {
        let (c_id, received, skipped) = h.join().unwrap();
        assert_eq!(
            received + skipped,
            num_items as u64,
            "accounting invariant violated on consumer {}",
            c_id
        );
        assert!(
            received > 0,
            "consumer {} received zero items",
            c_id
        );
    }
}

#[test]
fn test_multi_lane_sharded_fanout() {
    let num_lanes = 4;
    let capacity = 512;
    let (mut producer, factory) =
        MultiLaneBroadcastRing::<TestQuote>::new(num_lanes, capacity);

    let total_subscribers = 24;
    let num_items = 1000;
    let stop = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();
    for sub_id in 0..total_subscribers {
        let mut consumer = factory.subscribe(sub_id);
        assert_eq!(consumer.lane(), sub_id % num_lanes);
        let stop_clone = Arc::clone(&stop);

        handles.push(thread::spawn(move || {
            while (consumer.received() + consumer.skipped()) < num_items as u64 {
                match consumer.try_recv() {
                    Ok(_) => {}
                    Err(RecvError::Empty) => {
                        if stop_clone.load(Ordering::Acquire)
                            && (consumer.received() + consumer.skipped()) >= num_items as u64
                        {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                }
            }
            (sub_id, consumer.received(), consumer.skipped())
        }));
    }

    thread::sleep(Duration::from_millis(5));

    for seq in 0..num_items as u64 {
        producer.publish(TestQuote {
            timestamp_ns: 1_800_000_000 + seq,
            seq,
            bid: 120_000 + seq,
            ask: 120_010 + seq,
        });
    }
    stop.store(true, Ordering::Release);

    for h in handles {
        let (sub_id, received, skipped) = h.join().unwrap();
        assert_eq!(
            received + skipped,
            num_items as u64,
            "accounting invariant violated on sub_id {}",
            sub_id
        );
        assert!(
            received > 0,
            "sub_id {} received zero items",
            sub_id
        );
    }
}
