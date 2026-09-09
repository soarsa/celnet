//! Integration tests for AsyncJournal and CxlPmemJournal
//!
//! Verifies:
//! - Multi-threaded concurrent appends with AsynchronousGroupCommit
//! - Strict sequence monotonicity and zero lost updates
//! - Crash-recovery replay parity between AsyncJournal and synchronous Journal
//! - CxlPmemJournal byte-addressable persistence and CRC32 validation

use std::fs;
use std::thread;

use celnet_journal::{AsyncJournal, CxlPmemJournal, DurabilityPolicy, Journal, RecordKind};

#[test]
fn test_async_journal_multithreaded_group_commit() {
    let temp_dir = std::env::temp_dir().join("celnet_test_async_journal");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let journal_path = temp_dir.join("async_log.cln");

    let policy = DurabilityPolicy::AsynchronousGroupCommit {
        batch_size: 32,
        flush_interval_micros: 50,
    };

    let async_journal = AsyncJournal::open(&journal_path, policy).unwrap();

    let num_threads = 8;
    let records_per_thread = 50;
    let mut handles = Vec::new();

    for t in 0..num_threads {
        let aj = async_journal.clone();
        handles.push(thread::spawn(move || {
            let mut seqs = Vec::new();
            for i in 0..records_per_thread {
                let payload = format!("thread-{}-record-{}", t, i).into_bytes();
                let receipt = aj.append(payload).unwrap();
                let seq = receipt.wait().unwrap();
                seqs.push(seq);
            }
            seqs
        }));
    }

    let mut all_seqs = Vec::new();
    for h in handles {
        let seqs = h.join().unwrap();
        all_seqs.extend(seqs);
    }

    assert_eq!(all_seqs.len(), num_threads * records_per_thread);

    // Ensure all sequence numbers are unique, strictly monotonic from 0..N
    all_seqs.sort_unstable();
    for (idx, &seq) in all_seqs.iter().enumerate() {
        assert_eq!(seq, idx as u64);
    }

    // Flush and shutdown
    async_journal.flush().unwrap();
    async_journal.shutdown().unwrap();

    // Replay with synchronous Journal and assert all records are recovered cleanly
    let reopened = Journal::open(&journal_path).unwrap();
    assert_eq!(reopened.last_sequence(), Some((all_seqs.len() - 1) as u64));

    let mut replayed_count = 0;
    reopened
        .replay(|record| {
            assert_eq!(record.kind, RecordKind::Data);
            replayed_count += 1;
        })
        .unwrap();
    assert_eq!(replayed_count, num_threads * records_per_thread);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cxl_pmem_journal_persistence() {
    let mut pmem = CxlPmemJournal::new();
    assert_eq!(pmem.last_sequence(), None);
    assert_eq!(pmem.len_bytes(), 0);

    let records = vec![
        b"CXL-TX-1001-EURUSD-BUY".to_vec(),
        b"CXL-TX-1002-USDJPY-SELL".to_vec(),
        b"CXL-TX-1003-GBPUSD-QUOTE".to_vec(),
    ];

    let mut expected_seq = 0;
    for payload in &records {
        let seq = pmem.append(payload).unwrap();
        assert_eq!(seq, expected_seq);
        expected_seq += 1;
    }

    assert_eq!(pmem.last_sequence(), Some(2));
    assert!(pmem.len_bytes() > 0);

    // Save bytes to disk and verify Journal::open can read them bit-identically
    let temp_dir = std::env::temp_dir().join("celnet_test_cxl_pmem");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let journal_path = temp_dir.join("cxl_log.cln");

    fs::write(&journal_path, pmem.as_bytes()).unwrap();

    let reopened = Journal::open(&journal_path).unwrap();
    assert_eq!(reopened.last_sequence(), Some(2));

    let mut replayed = Vec::new();
    reopened
        .replay(|r| {
            replayed.push((r.sequence, r.payload));
        })
        .unwrap();

    assert_eq!(replayed.len(), 3);
    assert_eq!(replayed[0].0, 0);
    assert_eq!(&replayed[0].1[..], &records[0][..]);
    assert_eq!(replayed[1].0, 1);
    assert_eq!(&replayed[1].1[..], &records[1][..]);
    assert_eq!(replayed[2].0, 2);
    assert_eq!(&replayed[2].1[..], &records[2][..]);

    let _ = fs::remove_dir_all(&temp_dir);
}
