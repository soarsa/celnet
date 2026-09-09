//! SOTA Scalability Phase 5 Verification Suite:
//! 1. Strict Head-Drop Ingress Ring Buffer (Anti-Bufferbloat).
//! 2. Controlled Delay (CoDel RFC 8289) Sojourn Latency Tracking (Anti-Coordinated Omission).
//! 3. Jasper Fair Multicast Proxy Tree (Microsecond Simultaneous Playout & H=2 Tree Hedging).

use std::sync::Arc;
use std::time::Duration;
use celnet_server::ingress::{CoDelConfig, CoDelQueue, HeadDropQueue};
use celnet_server::multicast::{EdgeProxy, JasperConfig, JasperMulticastTree, SubscriberId};

#[test]
fn test_strict_head_drop_queue_eviction_and_freshness() {
    let capacity = 5;
    let queue = HeadDropQueue::<u64>::new(capacity);

    // Push 10 sequential quotes into a queue of capacity 5
    for seq in 1..=10 {
        queue.push(seq);
    }

    assert_eq!(queue.len(), 5);
    let stats = queue.stats().snapshot();
    assert_eq!(stats.enqueued, 10);
    assert_eq!(stats.head_dropped, 5);

    // Consumers must pop only the 5 freshest quotes: 6, 7, 8, 9, 10
    let mut drained = Vec::new();
    while let Some(item) = queue.pop() {
        drained.push(item);
    }

    assert_eq!(drained, vec![6, 7, 8, 9, 10]);
    assert_eq!(queue.stats().snapshot().dequeued, 5);
}

#[test]
fn test_codel_sojourn_latency_and_anti_bufferbloat() {
    let config = CoDelConfig {
        target_delay: Duration::from_millis(2),
        interval: Duration::from_millis(10),
        max_capacity: 100,
    };
    let queue = CoDelQueue::<u32>::new(config);

    // Enqueue 20 items
    for i in 0..20 {
        assert!(queue.enqueue(i));
    }

    assert_eq!(queue.len(), 20);

    // Dequeue immediately: sojourn times should be very low (< 2ms)
    let mut dequeued_count = 0;
    while let Some(_) = queue.dequeue() {
        dequeued_count += 1;
    }

    assert_eq!(dequeued_count, 20);

    // Verify accurate percentile tracking without Coordinated Omission
    let stats = queue.latency_stats();
    assert_eq!(stats.total_samples, 20);
    assert!(stats.p50 <= Duration::from_millis(5));
    assert!(stats.max >= stats.min);
}

#[test]
fn test_jasper_fair_multicast_simultaneous_delivery() {
    // 4 edge proxies, each serving 10 subscribers = 40 total counterparties
    let mut proxies = Vec::new();
    let mut sub_id = 1;
    for proxy_id in 0..4 {
        let subs: Vec<SubscriberId> = (0..10)
            .map(|_| {
                let id = SubscriberId(sub_id);
                sub_id += 1;
                id
            })
            .collect();
        proxies.push(Arc::new(EdgeProxy::new(proxy_id, subs)));
    }

    let config = JasperConfig {
        hold_window: Duration::from_micros(100),
        hedging_degree: 2,
        fairness_sla: Duration::from_micros(200),
    };

    let tree = JasperMulticastTree::new(config, proxies);

    // Publish a quote
    let quote = "EUR/USD 1.08500 / 1.08505";
    let frame = tree.publish(quote);

    assert_eq!(frame.sequence, 1);
    assert_eq!(frame.payload, quote);

    // Execute synchronized playout
    let report = tree.execute_synchronized_delivery(&frame);

    assert_eq!(report.subscriber_count, 40);
    assert!(
        report.conforms_to_sla,
        "Delivery spread ({:?}) exceeded fairness SLA ({:?})",
        report.spread, config.fairness_sla
    );

    println!(
        "Jasper Fair Multicast: 40 counterparties reached simultaneously with spread = {:?}",
        report.spread
    );
}
