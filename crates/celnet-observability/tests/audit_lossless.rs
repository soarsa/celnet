//! Integration guard for the **lossless** audit path: under sustained concurrent
//! load from many producers while a committer drains slowly, the audit sink drops
//! **nothing** — every assigned sequence is committed exactly once, gap-free and
//! duplicate-free. This is the property the docs claim and the property a
//! compliance audit trail requires (in contrast to the deliberately-lossy
//! telemetry ring). Bounded with a hard wall-clock deadline so a regression
//! (e.g. an accidental bounded/drop-on-full channel) fails fast, never hangs.

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use celnet_observability::audit_channel;
use celnet_observability::logging::{AuditRecord, AuditStage};

const DEADLINE: Duration = Duration::from_secs(10);
const PRODUCERS: u64 = 6;
const PER_PRODUCER: u64 = 5_000;

fn record(id: u64) -> AuditRecord {
    AuditRecord::new(AuditStage::TradeBooked, id, "idem", "tenant", "EURUSD")
}

#[test]
fn audit_sink_drops_nothing_under_concurrent_load() {
    let start = Instant::now();
    let total = PRODUCERS * PER_PRODUCER;

    let (sink, mut drain) = audit_channel();

    // A committer thread that drains slowly (the queue must backpressure-by-growth
    // losslessly, never drop). Runs until every sink is dropped and the backlog is
    // exhausted, then reports the committed sequence set back.
    let (report_tx, report_rx) = mpsc::channel::<BTreeSet<u64>>();
    let committer = thread::spawn(move || {
        let mut seen = BTreeSet::new();
        while let Some(rec) = drain.commit_blocking() {
            assert!(seen.insert(rec.sequence), "a sequence is committed twice");
        }
        let committed = drain.committed();
        let accepted = drain.accepted();
        assert_eq!(
            committed, accepted,
            "lossless: committed {committed} == accepted {accepted}"
        );
        report_tx.send(seen).expect("report channel open");
    });

    // Fan out producers hammering the shared, cloneable sink.
    let producers: Vec<_> = (0..PRODUCERS)
        .map(|p| {
            let sink = sink.clone();
            thread::spawn(move || {
                for i in 0..PER_PRODUCER {
                    sink.record(record(p * PER_PRODUCER + i))
                        .expect("the lossless sink never closes mid-load");
                }
            })
        })
        .collect();

    for producer in producers {
        producer.join().expect("producer joins");
    }
    // Closing every sink lets the committer terminate once the backlog drains.
    drop(sink);

    let seen = report_rx
        .recv_timeout(DEADLINE)
        .expect("committer reports before the deadline (no hang, no drop)");
    committer.join().expect("committer joins");

    // Exactly {1, 2, …, total}: nothing lost, nothing duplicated, no gap.
    assert_eq!(
        seen.len() as u64,
        total,
        "every record committed exactly once"
    );
    assert_eq!(*seen.iter().next().expect("non-empty"), 1);
    assert_eq!(*seen.iter().next_back().expect("non-empty"), total);

    assert!(
        start.elapsed() < DEADLINE,
        "the lossless audit drain must complete well within the deadline"
    );
}
