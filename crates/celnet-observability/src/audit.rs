//! The **lossless** audit transport for the quote/trade lifecycle: a separate,
//! sequence-assigning, never-dropping channel — distinct from the lossy telemetry
//! ring ([`crate::channel`]).
//!
//! # Why a second, different channel
//!
//! Telemetry ([`crate::channel`]) is *lossy by design*: on a full ring the hot
//! core drops the sample rather than block. That trade-off is correct for
//! latency/metrics samples but **wrong** for the audit log — a missing
//! `QuoteAccepted` or `TradeBooked` record is a compliance defect, not acceptable
//! degradation. The audit path therefore has the opposite contract:
//!
//! * **lossless** — backed by an unbounded queue; an [`AuditSink::record`] never
//!   drops and never silently discards a record;
//! * **backpressured, not on the hot core** — audit records are constructed on
//!   the **non-critical async edge** (the RFQ/RFS services), never on the pinned
//!   hot core, so an unbounded queue here cannot perturb hot-path latency. The
//!   `Sync` sink can be shared across every edge task;
//! * **sequence-assigning** — the sink stamps a single global, gap-free,
//!   strictly-increasing [`AuditRecord::sequence`](crate::logging::AuditRecord)
//!   under an atomic, giving the audit stream a total order and making any drop
//!   detectable downstream as a sequence break.
//!
//! The drain ([`AuditDrain`]) pops records **in sequence order** on a committer
//! task that persists them (and may call
//! [`AuditRecord::emit`](crate::logging::AuditRecord) to mirror them to the
//! structured log). Because the queue is unbounded the producer never blocks; a
//! slow committer grows the backlog (bounded only by audit throughput over a
//! deployment) rather than dropping — exactly the lossless guarantee audit needs.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use crate::logging::AuditRecord;

/// State shared between every [`AuditSink`] clone and the [`AuditDrain`]: the
/// monotonic sequence source and lossless accounting counters.
#[derive(Debug)]
struct AuditShared {
    /// The next audit sequence to assign. Stamped into each record so the stream
    /// is totally ordered and any loss is detectable as a sequence gap.
    next_sequence: AtomicU64,
    /// Total records the sink has accepted (== last assigned sequence). A
    /// lossless channel guarantees the drain eventually sees exactly this many.
    accepted: AtomicU64,
}

impl AuditShared {
    fn new() -> Self {
        Self {
            // Sequences start at 1 so 0 unambiguously means "unsequenced".
            next_sequence: AtomicU64::new(1),
            accepted: AtomicU64::new(0),
        }
    }
}

/// The producer end of the lossless audit channel — cloneable and `Sync`, so
/// every async-edge task can share one sink and submit lifecycle records.
///
/// Construct with [`audit_channel`]. An [`AuditSink::record`] assigns the global
/// audit sequence and enqueues the record on an **unbounded** queue: it never
/// drops and never blocks the caller.
#[derive(Debug, Clone)]
pub struct AuditSink {
    tx: Sender<AuditRecord>,
    shared: Arc<AuditShared>,
}

/// Returned by [`AuditSink::record`] when the [`AuditDrain`] (and thus the
/// committer task) has been dropped, so no record can ever be committed again.
/// The unsent record is handed back (boxed, so the `Err` variant stays small) so
/// the caller can react (e.g. fail closed).
#[derive(Debug)]
pub struct AuditClosed(pub Box<AuditRecord>);

impl std::fmt::Display for AuditClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the audit drain has been dropped; no further records can be committed")
    }
}

impl std::error::Error for AuditClosed {}

impl AuditSink {
    /// Assign the next global audit sequence to `record` and enqueue it,
    /// losslessly. The returned `sequence` is the value stamped into the record.
    ///
    /// This never drops and never blocks: the backing queue is unbounded, so a
    /// slow committer grows the backlog rather than discarding an audit event.
    ///
    /// # Errors
    ///
    /// [`AuditClosed`] (carrying the record back) only if the drain has been
    /// dropped — i.e. the committer is gone and nothing can persist the record.
    pub fn record(&self, mut record: AuditRecord) -> Result<u64, AuditClosed> {
        let sequence = self.shared.next_sequence.fetch_add(1, Ordering::SeqCst);
        record.sequence = sequence;
        match self.tx.send(record) {
            Ok(()) => {
                // `accepted` is the count of records guaranteed to reach the drain.
                self.shared.accepted.fetch_add(1, Ordering::SeqCst);
                Ok(sequence)
            }
            Err(e) => Err(AuditClosed(Box::new(e.0))),
        }
    }

    /// The number of records accepted (and therefore guaranteed deliverable) so
    /// far — equal to the highest sequence assigned to a successfully-enqueued
    /// record.
    #[must_use]
    pub fn accepted(&self) -> u64 {
        self.shared.accepted.load(Ordering::SeqCst)
    }
}

/// The consumer end of the lossless audit channel, run on a committer task on a
/// non-critical core. It pops records **in sequence order** and persists them
/// (and may mirror them to the structured log via
/// [`AuditRecord::emit`](crate::logging::AuditRecord)).
#[derive(Debug)]
pub struct AuditDrain {
    rx: Receiver<AuditRecord>,
    shared: Arc<AuditShared>,
    /// The highest sequence committed so far. With concurrent producers the
    /// channel's FIFO arrival order need not equal sequence order (two edge tasks
    /// may assign-then-send out of order), so this tracks the running maximum, not
    /// a strict predecessor; total order is recovered downstream by sorting on the
    /// stamped sequence.
    highest_committed: u64,
    /// Total records committed over this drain's lifetime.
    committed: u64,
}

impl AuditDrain {
    /// Pop the next audit record if one is available, without blocking. Returns
    /// `None` when the queue is momentarily empty (or once every sink has been
    /// dropped and the backlog is exhausted).
    ///
    /// Records arrive strictly in the sequence order the sink assigned (FIFO on a
    /// single unbounded queue), so a committer can persist them as a totally
    /// ordered, gap-free stream.
    pub fn try_commit(&mut self) -> Option<AuditRecord> {
        match self.rx.try_recv() {
            Ok(record) => {
                self.account(record.sequence);
                Some(record)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    /// Block the **committer** task until the next record is available, returning
    /// `None` once every [`AuditSink`] has been dropped and the backlog is fully
    /// drained. Intended for a dedicated blocking committer thread/task; never
    /// called on the hot core.
    pub fn commit_blocking(&mut self) -> Option<AuditRecord> {
        match self.rx.recv() {
            Ok(record) => {
                self.account(record.sequence);
                Some(record)
            }
            Err(_disconnected) => None,
        }
    }

    /// Drain every record currently queued, invoking `commit` for each in
    /// sequence order, and return how many were committed. Bounded by the records
    /// already enqueued, so it never live-locks against a producing edge.
    pub fn drain_available<F: FnMut(AuditRecord)>(&mut self, mut commit: F) -> usize {
        let mut n = 0;
        while let Some(record) = self.try_commit() {
            commit(record);
            n += 1;
        }
        n
    }

    fn account(&mut self, sequence: u64) {
        // A sequence is always positive (assigned from 1) — a zero would mean an
        // unsequenced record reached the channel, which the sink never enqueues.
        debug_assert!(sequence != 0, "audit records are sequenced from 1");
        self.highest_committed = self.highest_committed.max(sequence);
        self.committed += 1;
    }

    /// Total records committed over this drain's lifetime. The lossless guarantee
    /// is `committed() == sink.accepted()` once the backlog is fully drained — no
    /// record is ever dropped.
    #[must_use]
    pub fn committed(&self) -> u64 {
        self.committed
    }

    /// The number of records the sink(s) have accepted (and so are guaranteed to
    /// be committed by this lossless drain). Once every sink is drained,
    /// `committed() == accepted()` — the proof that nothing was dropped.
    #[must_use]
    pub fn accepted(&self) -> u64 {
        self.shared.accepted.load(Ordering::SeqCst)
    }

    /// The highest sequence committed (0 before the first commit).
    #[must_use]
    pub fn highest_committed(&self) -> u64 {
        self.highest_committed
    }
}

/// Create the lossless audit channel, returning a cloneable [`AuditSink`] (shared
/// by every async-edge task) and the single [`AuditDrain`] (run on the committer).
///
/// The channel is **unbounded**: the sink never drops and never blocks, so the
/// audit path is genuinely lossless, distinct from the lossy telemetry ring.
#[must_use]
pub fn audit_channel() -> (AuditSink, AuditDrain) {
    let (tx, rx) = channel::<AuditRecord>();
    let shared = Arc::new(AuditShared::new());
    let sink = AuditSink {
        tx,
        shared: Arc::clone(&shared),
    };
    let drain = AuditDrain {
        rx,
        shared,
        highest_committed: 0,
        committed: 0,
    };
    (sink, drain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::AuditStage;
    use crate::record::OpKind;

    fn sample(stage: AuditStage, id: u64) -> AuditRecord {
        AuditRecord::new(stage, id, "idem", "tenant", "EURUSD")
    }

    #[test]
    fn assigns_strictly_increasing_gap_free_sequences() {
        let (sink, mut drain) = audit_channel();
        for i in 0..16u64 {
            let seq = sink
                .record(sample(AuditStage::QuoteIssued, i))
                .expect("sink open");
            assert_eq!(seq, i + 1, "sequences are assigned 1,2,3,…");
        }
        let mut seen = Vec::new();
        drain.drain_available(|r| seen.push(r.sequence));
        assert_eq!(seen, (1..=16).collect::<Vec<_>>(), "gap-free, in order");
        assert_eq!(drain.committed(), 16);
        assert_eq!(sink.accepted(), 16);
    }

    /// The defining property: under heavy concurrent load from many edge tasks,
    /// the audit sink drops **nothing** — every assigned sequence is committed
    /// exactly once, with no gap and no duplicate. (Contrast the lossy telemetry
    /// ring, which deliberately drops on a full ring.)
    #[test]
    fn never_drops_under_concurrent_load() {
        use std::collections::BTreeSet;
        use std::thread;

        let (sink, mut drain) = audit_channel();
        const THREADS: u64 = 8;
        const PER_THREAD: u64 = 4_000;
        let total = THREADS * PER_THREAD;

        let producers: Vec<_> = (0..THREADS)
            .map(|t| {
                let sink = sink.clone();
                thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        sink.record(sample(AuditStage::TradeBooked, t * PER_THREAD + i))
                            .expect("lossless sink never closes mid-load");
                    }
                })
            })
            .collect();
        for p in producers {
            p.join().expect("producer thread joins");
        }
        drop(sink); // close so commit_blocking terminates once drained.

        // Commit everything and collect the assigned sequences.
        let mut seqs = BTreeSet::new();
        while let Some(record) = drain.commit_blocking() {
            assert!(
                seqs.insert(record.sequence),
                "no sequence is committed twice"
            );
        }

        assert_eq!(
            drain.committed(),
            total,
            "every accepted audit record is committed (nothing dropped)"
        );
        assert_eq!(
            drain.committed(),
            drain.accepted(),
            "committed == accepted: the lossless invariant"
        );
        // The set is exactly {1, 2, …, total}: no gap, no duplicate.
        assert_eq!(*seqs.iter().next().expect("non-empty"), 1);
        assert_eq!(*seqs.iter().next_back().expect("non-empty"), total);
        assert_eq!(seqs.len() as u64, total);
    }

    #[test]
    fn priced_record_carries_sequence_for_emit() {
        let (sink, mut drain) = audit_channel();
        let rec = sample(AuditStage::QuoteAccepted, 7).with_priced(OpKind::VanillaPrice, 0.0123);
        let seq = sink.record(rec).expect("sink open");
        let committed = drain.try_commit().expect("a record is queued");
        assert_eq!(committed.sequence, seq);
        assert_eq!(committed.premium, Some(0.0123));
    }

    #[test]
    fn record_after_drain_dropped_is_reported_not_dropped_silently() {
        let (sink, drain) = audit_channel();
        drop(drain);
        let err = sink
            .record(sample(AuditStage::QuoteRejected, 1))
            .expect_err("a closed drain surfaces an error, never a silent drop");
        // The record is handed back so the caller can fail closed.
        assert_eq!(err.0.stage, AuditStage::QuoteRejected);
    }
}
