//! Journal tests — all timeout-bounded by the harness (`cargo nextest` slow
//! timeout) and by construction loop-free (fixed record counts). Each test uses
//! a unique temp directory under [`std::env::temp_dir`] and removes it on exit,
//! so no external `tempfile` dependency is needed (guardrail: prefer std fs).

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// A self-cleaning unique temp directory (RAII), std-only.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut path = std::env::temp_dir();
        path.push(format!("celnet-journal-{tag}-{pid}-{nanos}-{n}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir { path }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn append_then_reopen_replays_all_in_order() {
    let dir = TempDir::new("roundtrip");
    let log = dir.file("events.log");

    const N: u64 = 64;
    {
        let mut journal = Journal::open(&log).expect("open fresh");
        assert_eq!(
            journal.last_sequence(),
            None,
            "fresh log has no last sequence"
        );
        for i in 0..N {
            let payload = format!("event-{i}").into_bytes();
            let seq = journal.append(&payload).expect("append");
            assert_eq!(seq, i, "sequence assigned densely from 0");
        }
        assert_eq!(journal.last_sequence(), Some(N - 1));
        assert_eq!(journal.next_sequence(), N);
    }

    // Reopen a fresh handle and replay.
    let journal = Journal::open(&log).expect("reopen");
    assert_eq!(
        journal.last_sequence(),
        Some(N - 1),
        "sequence survives reopen"
    );
    let recs = journal.records().expect("replay");
    assert_eq!(recs.len() as u64, N, "exactly N records recovered");
    for (i, rec) in recs.iter().enumerate() {
        assert_eq!(rec.sequence, i as u64, "in-order, dense sequence");
        assert_eq!(rec.payload, format!("event-{i}").into_bytes());
    }
}

#[test]
fn truncated_tail_is_recovered_cleanly() {
    let dir = TempDir::new("torn-tail");
    let log = dir.file("events.log");

    const N: u64 = 10;
    {
        let mut journal = Journal::open(&log).expect("open");
        for i in 0..N {
            journal
                .append(format!("rec-{i}").as_bytes())
                .expect("append");
        }
    }

    // Simulate a crash mid-append: chop a few bytes off the physical end so the
    // final record's frame is truncated (its CRC trailer is partially gone).
    let full_len = std::fs::metadata(&log).expect("meta").len();
    let f = OpenOptions::new()
        .write(true)
        .open(&log)
        .expect("open for trunc");
    f.set_len(full_len - 3).expect("truncate tail");
    f.sync_all().expect("sync trunc");
    drop(f);

    // Reopen: torn tail must be detected, healed, and surfaced as N-1 good
    // records with NO error to the caller.
    let journal = Journal::open(&log).expect("reopen after torn tail (no error)");
    let recs = journal.records().expect("replay after heal");
    assert_eq!(recs.len() as u64, N - 1, "torn final record dropped");
    assert_eq!(journal.last_sequence(), Some(N - 2));
    assert_eq!(
        journal.next_sequence(),
        N - 1,
        "next append reuses the dropped slot"
    );
    for (i, rec) in recs.iter().enumerate() {
        assert_eq!(rec.sequence, i as u64);
        assert_eq!(rec.payload, format!("rec-{i}").into_bytes());
    }

    // The file must have been physically truncated to the last good record.
    let healed_len = std::fs::metadata(&log).expect("meta").len();
    assert!(
        healed_len < full_len - 3,
        "file truncated to last good record"
    );
    assert_eq!(healed_len, journal.len_bytes());
}

#[test]
fn appending_after_torn_tail_recovery_continues_cleanly() {
    let dir = TempDir::new("torn-then-append");
    let log = dir.file("events.log");

    {
        let mut journal = Journal::open(&log).expect("open");
        for i in 0..5u64 {
            journal.append(format!("a-{i}").as_bytes()).expect("append");
        }
    }
    let full_len = std::fs::metadata(&log).expect("meta").len();
    let f = OpenOptions::new()
        .write(true)
        .open(&log)
        .expect("open for trunc");
    f.set_len(full_len - 2).expect("truncate tail");
    drop(f);

    // Recover, then keep appending — sequence picks up where the good data ended.
    {
        let mut journal = Journal::open(&log).expect("reopen");
        assert_eq!(
            journal.next_sequence(),
            4,
            "dropped record's slot is reused"
        );
        let seq = journal.append(b"replacement").expect("append after heal");
        assert_eq!(seq, 4);
        let seq = journal.append(b"more").expect("append again");
        assert_eq!(seq, 5);
    }

    let journal = Journal::open(&log).expect("final reopen");
    let recs = journal.records().expect("replay");
    assert_eq!(recs.len(), 6, "4 survivors + 2 new");
    assert_eq!(recs[4].payload, b"replacement".to_vec());
    assert_eq!(recs[5].payload, b"more".to_vec());
    for (i, rec) in recs.iter().enumerate() {
        assert_eq!(
            rec.sequence, i as u64,
            "sequence strictly monotonic across reopen"
        );
    }
}

#[test]
fn flipped_byte_in_final_record_is_detected_and_dropped() {
    let dir = TempDir::new("flip-tail");
    let log = dir.file("events.log");

    const N: u64 = 8;
    {
        let mut journal = Journal::open(&log).expect("open");
        for i in 0..N {
            journal
                .append(format!("payload-{i}").as_bytes())
                .expect("append");
        }
    }

    // Flip a byte inside the LAST record's payload region. Locate it: the final
    // record starts at the offset just past the second-to-last record. We flip a
    // byte near the physical end (within the last record's payload) — robust
    // because the last record is the longest-lived candidate for a torn write.
    let bytes = std::fs::read(&log).expect("read");
    let flip_at = bytes.len() - CRC_LEN - 2; // inside the last payload, before CRC
    let mut corrupted = bytes.clone();
    corrupted[flip_at] ^= 0xFF;
    std::fs::write(&log, &corrupted).expect("write corrupted");

    // The CRC of the last record now fails ⇒ treated as a torn tail and dropped.
    let journal = Journal::open(&log).expect("reopen (no error: tail corruption)");
    let recs = journal.records().expect("replay");
    assert_eq!(
        recs.len() as u64,
        N - 1,
        "corrupt final record dropped via CRC"
    );
    for (i, rec) in recs.iter().enumerate() {
        assert_eq!(rec.sequence, i as u64);
        assert_eq!(rec.payload, format!("payload-{i}").into_bytes());
    }
}

#[test]
fn interior_crc_corruption_truncates_trailing_records() {
    // Documents the STATED LIMITATION of the marker-less format (see the crate doc
    // "Failure model"): a CRC failure on an interior record is indistinguishable
    // from a torn tail, so recovery stops there and the trailing — possibly
    // already-acknowledged — records are dropped, with `open` returning Ok (NOT an
    // error). This test pins that behavior so a future resync-marker format that
    // *does* surface it is a deliberate, test-visible change, not an accident.
    let dir = TempDir::new("interior");
    let log = dir.file("events.log");

    const N: u64 = 6;
    {
        let mut journal = Journal::open(&log).expect("open");
        for i in 0..N {
            // Fixed-width payloads so we can locate an interior record.
            journal
                .append(format!("rec{i:04}").as_bytes())
                .expect("append");
        }
    }

    // Each record: 4 (len) + 8 (seq) + 7 (payload "recNNNN") + 4 (crc) = 23 bytes.
    let record_len = HEADER_LEN + 7 + CRC_LEN;
    let bytes = std::fs::read(&log).expect("read");
    // Flip a byte in the payload of record index 2 (an INTERIOR record).
    let flip_at = 2 * record_len + HEADER_LEN + 1;
    let mut corrupted = bytes.clone();
    corrupted[flip_at] ^= 0xFF;
    std::fs::write(&log, &corrupted).expect("write corrupted");

    // Record 2's CRC now fails. Because there are valid records AFTER it, the
    // log does not end here — but our recovery cannot tell interior rot from a
    // torn tail, so it stops at the first bad record (treating it as the tail) and
    // drops the trailing good records, healing by truncation. The survivors are
    // exactly the records BEFORE the corruption — and open() returns Ok.
    let journal = Journal::open(&log).expect("reopen (healed, not errored)");
    let recs = journal.records().expect("replay");
    // Records 0 and 1 are intact and recovered; 2..N are dropped with the tail.
    assert_eq!(recs.len(), 2, "recovery stops at first corrupt record");
    assert_eq!(recs[0].payload, b"rec0000".to_vec());
    assert_eq!(recs[1].payload, b"rec0001".to_vec());
}

/// Frame a record exactly as [`Journal::append`] does, but with a caller-chosen
/// sequence number — so a test can forge a CRC-**valid** record carrying the wrong
/// sequence (the one interior inconsistency the format is able to detect).
fn frame_record(seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&seq.to_le_bytes());
    frame.extend_from_slice(payload);
    let crc = crc32(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

/// Forge a CRC-valid **snapshot** record carrying `watermark` and `snapshot`
/// bytes, framed exactly as [`Journal::compact`] writes it (the SNAPSHOT_MARKER
/// sentinel in the length field + an inner length prefix). Lets a test plant a
/// snapshot record in an illegal position to exercise the misplacement check.
fn forge_snapshot(watermark: u64, snapshot: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&SNAPSHOT_MARKER.to_le_bytes());
    frame.extend_from_slice(&watermark.to_le_bytes());
    frame.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
    frame.extend_from_slice(snapshot);
    let crc = crc32(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

#[test]
fn valid_crc_record_with_nonmonotonic_sequence_is_surfaced() {
    // The detectable interior inconsistency: intact bytes (CRC passes) but a
    // sequence number that breaks strict monotonicity (a reordered/duplicated
    // frame). Truncation cannot explain this, so it must be SURFACED as
    // CorruptInterior — not silently healed. Exercises the lib.rs sequence-break
    // branch that the happy-path tests never reach.
    let dir = TempDir::new("seq-break");
    let log = dir.file("events.log");

    // Two CRC-valid frames, but the second's sequence jumps 0 -> 5 (expected 1).
    let mut bytes = frame_record(0, b"first");
    bytes.extend_from_slice(&frame_record(5, b"second"));
    std::fs::write(&log, &bytes).expect("write forged log");

    match Journal::open(&log) {
        Err(JournalError::CorruptInterior {
            at_sequence,
            reason,
        }) => {
            assert_eq!(
                at_sequence, 1,
                "break detected at the expected next sequence"
            );
            assert!(
                reason.contains("monotonic"),
                "reason names the sequence break: {reason}"
            );
        }
        other => panic!("expected CorruptInterior, got {other:?}"),
    }
}

#[test]
fn kill_restart_replay_is_byte_identical() {
    // Models a hard process kill: we append a deterministic stream, drop the
    // handle WITHOUT any graceful close (each append already fsync'd), then
    // "restart" by reopening and replaying. The recovered byte stream must be
    // bit-identical to what was appended — the determinism contract that
    // crash-recovery state rebuild relies on.
    let dir = TempDir::new("kill-restart");
    let log = dir.file("events.log");

    // A deterministic, varied set of payloads (incl. embedded NULs and high
    // bytes) so the comparison is sensitive to any framing/offset error.
    let mut appended: Vec<Vec<u8>> = Vec::new();
    for i in 0u64..40 {
        let mut p = Vec::new();
        p.extend_from_slice(&i.to_le_bytes());
        p.extend_from_slice(&[0x00, 0xFF, 0xAA, 0x55]);
        p.extend_from_slice(format!("mark-{i}").as_bytes());
        appended.push(p);
    }

    {
        let mut journal = Journal::open(&log).expect("open");
        for p in &appended {
            journal.append(p).expect("append");
        }
        // Simulate a kill: just drop the handle (no flush/close ceremony —
        // durability already came from the per-append fsync).
    }

    // First restart.
    let recovered_a: Vec<Vec<u8>> = Journal::open(&log)
        .expect("restart 1")
        .records()
        .expect("replay 1")
        .into_iter()
        .map(|r| r.payload)
        .collect();
    assert_eq!(
        recovered_a, appended,
        "replay byte-identical to appended stream"
    );

    // Second restart (idempotent recovery): replay again, must be identical.
    let recovered_b: Vec<Vec<u8>> = Journal::open(&log)
        .expect("restart 2")
        .records()
        .expect("replay 2")
        .into_iter()
        .map(|r| r.payload)
        .collect();
    assert_eq!(
        recovered_b, recovered_a,
        "repeated recovery is bit-identical"
    );
}

#[test]
fn empty_payload_roundtrips() {
    let dir = TempDir::new("empty-payload");
    let log = dir.file("events.log");
    {
        let mut journal = Journal::open(&log).expect("open");
        assert_eq!(journal.append(b"").expect("append empty"), 0);
        assert_eq!(journal.append(b"after").expect("append"), 1);
    }
    let journal = Journal::open(&log).expect("reopen");
    let recs = journal.records().expect("replay");
    assert_eq!(recs.len(), 2);
    assert!(recs[0].payload.is_empty(), "zero-length payload preserved");
    assert_eq!(recs[1].payload, b"after".to_vec());
}

#[test]
fn payload_too_large_is_rejected() {
    let dir = TempDir::new("too-large");
    let log = dir.file("events.log");
    let mut journal = Journal::open(&log).expect("open");
    // Materialize a payload one byte over the recovery safety bound and assert
    // the documented rejection path (a corrupt length field could otherwise
    // demand a wild recovery allocation). The slice is zeroed and short-lived.
    let oversized = vec![0u8; (MAX_PAYLOAD_LEN as usize) + 1];
    match journal.append(&oversized) {
        Err(JournalError::PayloadTooLarge { len }) => {
            assert_eq!(len, oversized.len());
        }
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
    // A modest payload still appends fine, and nothing was written for the reject.
    assert_eq!(journal.append(b"ok").expect("append ok"), 0);
}

/// A typed [`EventCodec`] over the durable bytes — proves the seam composes:
/// encode a domain event, journal it, recover it, decode bit-identically.
#[test]
fn event_codec_seam_roundtrips_a_domain_event() {
    use celnet_types::Vol;

    /// A minimal control-plane event: an accepted ATM vol mark for a tenor.
    #[derive(Debug, Clone, PartialEq)]
    struct AcceptedMark {
        tenor_days: u32,
        atm_vol: Vol,
    }

    struct MarkCodec;
    impl EventCodec for MarkCodec {
        type Event = AcceptedMark;
        type Error = &'static str;

        fn encode(event: &AcceptedMark) -> Vec<u8> {
            let mut b = Vec::with_capacity(12);
            b.extend_from_slice(&event.tenor_days.to_le_bytes());
            // Deterministic byte image of the f64 (to_bits) ⇒ bit-identical replay.
            b.extend_from_slice(&event.atm_vol.0.to_bits().to_le_bytes());
            b
        }

        fn decode(payload: &[u8]) -> std::result::Result<AcceptedMark, &'static str> {
            if payload.len() != 12 {
                return Err("bad AcceptedMark length");
            }
            let tenor_days = u32::from_le_bytes(payload[0..4].try_into().unwrap());
            let bits = u64::from_le_bytes(payload[4..12].try_into().unwrap());
            Ok(AcceptedMark {
                tenor_days,
                atm_vol: Vol(f64::from_bits(bits)),
            })
        }
    }

    let dir = TempDir::new("codec");
    let log = dir.file("marks.log");

    let events = [
        AcceptedMark {
            tenor_days: 7,
            atm_vol: Vol(0.0925),
        },
        AcceptedMark {
            tenor_days: 30,
            atm_vol: Vol(0.1011),
        },
        AcceptedMark {
            tenor_days: 90,
            atm_vol: Vol(0.1180),
        },
    ];

    {
        let mut journal = Journal::open(&log).expect("open");
        for ev in &events {
            journal.append(&MarkCodec::encode(ev)).expect("append");
        }
    }

    let journal = Journal::open(&log).expect("reopen");
    let mut rebuilt = Vec::new();
    journal
        .replay(|rec| rebuilt.push(MarkCodec::decode(&rec.payload).expect("decode")))
        .expect("replay");

    assert_eq!(rebuilt.len(), events.len());
    for (got, want) in rebuilt.iter().zip(events.iter()) {
        assert_eq!(got.tenor_days, want.tenor_days);
        // Bit-identical recovery (the determinism contract recovery relies on).
        assert_eq!(got.atm_vol.0.to_bits(), want.atm_vol.0.to_bits());
    }
}

// ───────────────────────────── Compaction tests ─────────────────────────────

/// A deterministic toy state machine over journalled events: each data event is
/// an 8-byte little-endian delta; the consumer's state is the running sum. Models
/// a consumer that snapshots its rebuilt state (the sum so far) at a watermark and
/// resumes from `snapshot ⊕ residual-tail` after compaction. The gate is that
/// `replay(full)` and `replay(compacted)` rebuild the **same** state.
fn delta(n: i64) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

/// Rebuild the running-sum state from a replayed journal. A leading snapshot
/// record (if present) seeds the sum; every subsequent data record adds its delta.
/// Returns `(state, recovered_records)` so tests assert both the rebuilt state and
/// the exact recovered record stream.
fn rebuild(journal: &Journal) -> (i64, Vec<Record>) {
    let mut state: i64 = 0;
    let mut recs = Vec::new();
    journal
        .replay(|rec| {
            match rec.kind {
                RecordKind::Snapshot => {
                    state = i64::from_le_bytes(rec.payload[..8].try_into().unwrap());
                }
                RecordKind::Data => {
                    state += i64::from_le_bytes(rec.payload[..8].try_into().unwrap());
                }
            }
            recs.push(rec);
        })
        .expect("replay");
    (state, recs)
}

#[test]
fn replay_from_compacted_equals_replay_from_full_bit_identical() {
    // THE CORE GATE: compacting must not change the rebuilt state. We build a log,
    // record the full-replay state (the INDEPENDENT oracle = plain sum of all
    // deltas), compact at a mid-log watermark with a snapshot = the partial sum,
    // then prove the compacted log rebuilds the SAME state — and that the residual
    // data records survive byte-for-byte with their ORIGINAL sequence numbers.
    let dir = TempDir::new("compact-equiv");
    let full_log = dir.file("full.log");
    let comp_log = dir.file("compact.log");

    let deltas: [i64; 12] = [5, -3, 7, 11, -20, 2, 100, -50, 8, 0, -1, 64];

    // Build the FULL (never-compacted) reference log.
    {
        let mut j = Journal::open(&full_log).expect("open full");
        for d in deltas {
            j.append(&delta(d)).expect("append");
        }
    }
    let full = Journal::open(&full_log).expect("reopen full");
    let (full_state, full_recs) = rebuild(&full);
    // Independent oracle: the state is just the plain sum of every delta.
    let oracle: i64 = deltas.iter().sum();
    assert_eq!(full_state, oracle, "full replay state == independent sum");
    assert_eq!(
        full_recs.len(),
        deltas.len(),
        "all data records, no snapshot"
    );

    // Build a second log identically, then compact it at watermark = seq 6 with a
    // snapshot equal to the partial sum of deltas[0..=6] (the consumer's checkpoint).
    let watermark = 6u64;
    {
        let mut j = Journal::open(&comp_log).expect("open comp");
        for d in deltas {
            j.append(&delta(d)).expect("append");
        }
        let partial: i64 = deltas[..=(watermark as usize)].iter().sum();
        j.compact(watermark, &delta(partial)).expect("compact");
        // After compaction the next append must still be monotonic (= old next).
        assert_eq!(
            j.next_sequence(),
            deltas.len() as u64,
            "next_sequence unchanged by compaction"
        );
        assert_eq!(j.last_sequence(), Some(deltas.len() as u64 - 1));
    }

    // Reopen the compacted log and rebuild.
    let comp = Journal::open(&comp_log).expect("reopen comp");
    let (comp_state, comp_recs) = rebuild(&comp);

    // GATE 1: rebuilt state is bit-identical.
    assert_eq!(
        comp_state, full_state,
        "replay-from-compacted state == replay-from-full state"
    );

    // GATE 2: structure — leading snapshot at the watermark, then the residual tail
    // (records 7..=11) byte-for-byte with ORIGINAL sequence numbers preserved.
    assert_eq!(comp_recs[0].kind, RecordKind::Snapshot);
    assert_eq!(
        comp_recs[0].sequence, watermark,
        "snapshot carries watermark"
    );
    let residual = &comp_recs[1..];
    let expected_tail = &full_recs[(watermark as usize + 1)..];
    assert_eq!(residual.len(), expected_tail.len(), "residual tail length");
    for (got, want) in residual.iter().zip(expected_tail.iter()) {
        assert_eq!(got.kind, RecordKind::Data);
        assert_eq!(got.sequence, want.sequence, "original sequence preserved");
        assert_eq!(got.payload, want.payload, "residual payload byte-identical");
    }
}

#[test]
fn compaction_keeps_sequences_globally_monotonic_across_appends() {
    // Compact, then append, then compact again — sequences must never rewind or
    // repeat across the whole history. We assert strict monotonicity over every
    // data record observed across two compaction cycles plus interleaved appends.
    let dir = TempDir::new("compact-monotonic");
    let log = dir.file("events.log");

    {
        let mut j = Journal::open(&log).expect("open");
        for d in 0..10i64 {
            j.append(&delta(d)).expect("append");
        }
        // First compaction at watermark 4 (snapshot = sum 0..=4 = 10).
        j.compact(4, &delta(10)).expect("compact 1");
        assert_eq!(
            j.next_sequence(),
            10,
            "append resumes at 10 after compact 1"
        );

        // Append more — these MUST take sequences 10,11,12 (monotone, no reuse).
        assert_eq!(j.append(&delta(10)).expect("append"), 10);
        assert_eq!(j.append(&delta(11)).expect("append"), 11);
        assert_eq!(j.append(&delta(12)).expect("append"), 12);

        // Second compaction at watermark 11. The snapshot is opaque to the journal;
        // it must equal the consumer's running sum of every delta through seq 11.
        // Deltas: seqs 0..=9 = 0,1,..,9; seq 10 = 10; seq 11 = 11; seq 12 = 12.
        let sum_through_11: i64 = (0..10i64).sum::<i64>() + 10 + 11;
        j.compact(11, &delta(sum_through_11)).expect("compact 2");
        assert_eq!(
            j.next_sequence(),
            13,
            "append resumes at 13 after compact 2"
        );
        assert_eq!(j.append(&delta(99)).expect("append"), 13);
    }

    // Reopen and assert global monotonicity over the recovered data records.
    let j = Journal::open(&log).expect("reopen");
    let recs = j.records().expect("replay");
    let mut last_data: Option<u64> = None;
    let mut saw_snapshot_first = false;
    for (i, rec) in recs.iter().enumerate() {
        match rec.kind {
            RecordKind::Snapshot => {
                assert_eq!(i, 0, "snapshot only ever appears first");
                saw_snapshot_first = true;
            }
            RecordKind::Data => {
                if let Some(prev) = last_data {
                    assert!(
                        rec.sequence > prev,
                        "data sequences strictly increasing: {} after {}",
                        rec.sequence,
                        prev
                    );
                }
                last_data = Some(rec.sequence);
            }
        }
    }
    assert!(saw_snapshot_first, "compacted log has a leading snapshot");
    assert_eq!(
        j.next_sequence(),
        14,
        "global next_sequence past the last append"
    );
}

#[test]
fn crash_during_compaction_heals_to_old_complete_log() {
    // CRASH-SAFETY (mid-temp-write): a compaction that dies after writing a PARTIAL
    // temp file but BEFORE the atomic rename leaves the live log untouched. We
    // simulate this by writing a truncated `.compact` temp ourselves and leaving
    // the live log in place — then proving recovery reads the COMPLETE OLD log,
    // never the torn temp.
    let dir = TempDir::new("crash-pre-rename");
    let log = dir.file("events.log");
    let deltas: [i64; 6] = [1, 2, 3, 4, 5, 6];
    {
        let mut j = Journal::open(&log).expect("open");
        for d in deltas {
            j.append(&delta(d)).expect("append");
        }
    }
    let old_bytes = std::fs::read(&log).expect("read old");

    // Simulate a crash mid-compaction: a half-written temp exists, no rename done.
    let tmp = {
        let mut s = log.as_os_str().to_os_string();
        s.push(".compact");
        PathBuf::from(s)
    };
    // A plausible partial fresh image: a snapshot frame + a torn residual frame.
    let mut partial = forge_snapshot(3, &delta(10));
    let residual = frame_record(4, &delta(4));
    partial.extend_from_slice(&residual[..residual.len() - 5]); // chop the tail
    std::fs::write(&tmp, &partial).expect("write partial temp");

    // Recovery: the live log is still the OLD complete log (rename never happened).
    let j = Journal::open(&log).expect("reopen — old log intact");
    let (state, recs) = rebuild(&j);
    assert_eq!(state, deltas.iter().sum::<i64>(), "old-log state intact");
    assert_eq!(recs.len(), deltas.len(), "no snapshot — old log unchanged");
    assert_eq!(recs[0].kind, RecordKind::Data, "old log: pure data records");
    // The live log bytes are byte-identical to before the crashed compaction.
    assert_eq!(
        std::fs::read(&log).expect("read"),
        old_bytes,
        "live log untouched by the crashed compaction"
    );

    // And a fresh compaction cleans up the stale temp and succeeds.
    {
        let mut j = Journal::open(&log).expect("open");
        j.compact(3, &delta(10)).expect("compact over stale temp");
    }
    let j = Journal::open(&log).expect("reopen after real compact");
    let (state2, recs2) = rebuild(&j);
    assert_eq!(
        state2,
        deltas.iter().sum::<i64>(),
        "post-compact state matches"
    );
    assert_eq!(recs2[0].kind, RecordKind::Snapshot, "now compacted");
}

#[test]
fn crash_after_compaction_reads_complete_new_log() {
    // CRASH-SAFETY (post-rename): once `compact` returns Ok, the rename + dir fsync
    // are durable, so a crash AFTER it reads the COMPLETE NEW (compacted) log. We
    // model the "crash" as dropping the handle right after compact and reopening
    // cold — the compacted log must be intact and complete, with no lingering temp.
    let dir = TempDir::new("crash-post-rename");
    let log = dir.file("events.log");
    let deltas: [i64; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
    {
        let mut j = Journal::open(&log).expect("open");
        for d in deltas {
            j.append(&delta(d)).expect("append");
        }
        let partial: i64 = deltas[..=5].iter().sum();
        j.compact(5, &delta(partial)).expect("compact");
        // Drop the handle here == a crash immediately after compact returned Ok.
    }

    // No `.compact` temp must linger after a successful compaction.
    let tmp = {
        let mut s = log.as_os_str().to_os_string();
        s.push(".compact");
        PathBuf::from(s)
    };
    assert!(!tmp.exists(), "temp removed/renamed away after compaction");

    // Cold reopen reads the complete new log.
    let j = Journal::open(&log).expect("reopen cold");
    let (state, recs) = rebuild(&j);
    assert_eq!(state, deltas.iter().sum::<i64>(), "new-log rebuild correct");
    assert_eq!(recs[0].kind, RecordKind::Snapshot);
    assert_eq!(recs[0].sequence, 5);
    // Residual = records 6,7 with original sequences.
    assert_eq!(recs.len(), 3, "snapshot + 2 residual");
    assert_eq!(recs[1].sequence, 6);
    assert_eq!(recs[2].sequence, 7);
    // And it round-trips through a second reopen identically (idempotent recovery).
    let j2 = Journal::open(&log).expect("reopen again");
    let (state2, recs2) = rebuild(&j2);
    assert_eq!(state2, state);
    assert_eq!(recs2, recs, "repeated recovery bit-identical");
}

#[test]
fn compact_everything_leaves_snapshot_only_and_append_continues() {
    // Watermark == last sequence: the residual tail is EMPTY, so the compacted log
    // is a single snapshot record. Appends after that continue monotonically.
    let dir = TempDir::new("compact-all");
    let log = dir.file("events.log");
    let deltas: [i64; 5] = [3, 3, 3, 3, 3];
    {
        let mut j = Journal::open(&log).expect("open");
        for d in deltas {
            j.append(&delta(d)).expect("append");
        }
        // Compact everything (watermark = last seq = 4); snapshot = full sum 15.
        j.compact(4, &delta(15)).expect("compact all");
        assert_eq!(
            j.next_sequence(),
            5,
            "next still 5 (monotone) with empty tail"
        );
        // Append continues from 5.
        assert_eq!(j.append(&delta(7)).expect("append"), 5);
    }
    let j = Journal::open(&log).expect("reopen");
    let (state, recs) = rebuild(&j);
    assert_eq!(state, 15 + 7, "snapshot(15) + one new delta(7)");
    assert_eq!(recs.len(), 2, "snapshot + 1 new data record");
    assert_eq!(recs[0].kind, RecordKind::Snapshot);
    assert_eq!(recs[0].sequence, 4);
    assert_eq!(recs[1].kind, RecordKind::Data);
    assert_eq!(recs[1].sequence, 5, "new append took the next monotone seq");
}

#[test]
fn compaction_watermark_beyond_log_is_rejected() {
    // A watermark above the highest durable sequence would have the snapshot claim
    // coverage of records that do not exist — rejected, log left untouched.
    let dir = TempDir::new("compact-overclaim");
    let log = dir.file("events.log");
    {
        let mut j = Journal::open(&log).expect("open");
        for d in 0..3i64 {
            j.append(&delta(d)).expect("append");
        }
        // last_sequence == 2; watermark 3 over-claims.
        match j.compact(3, &delta(3)) {
            Err(JournalError::CorruptInterior {
                at_sequence,
                reason,
            }) => {
                assert_eq!(at_sequence, 3);
                assert!(
                    reason.contains("watermark"),
                    "reason names watermark: {reason}"
                );
            }
            other => panic!("expected rejection, got {other:?}"),
        }
        // Log untouched: still 3 plain data records, next_sequence still 3.
        assert_eq!(j.next_sequence(), 3);
    }
    let j = Journal::open(&log).expect("reopen");
    let recs = j.records().expect("replay");
    assert_eq!(recs.len(), 3);
    assert!(recs.iter().all(|r| r.kind == RecordKind::Data));
}

#[test]
fn compaction_on_empty_log_is_rejected() {
    // An empty log has no durable sequence to checkpoint — any watermark over-claims.
    let dir = TempDir::new("compact-empty");
    let log = dir.file("events.log");
    let mut j = Journal::open(&log).expect("open");
    match j.compact(0, &delta(0)) {
        Err(JournalError::CorruptInterior { .. }) => {}
        other => panic!("expected rejection on empty log, got {other:?}"),
    }
}

#[test]
fn torn_tail_after_compaction_heals_to_snapshot_plus_good_residual() {
    // A crash mid-APPEND after a compaction (torn final residual record) must heal
    // exactly like the non-compacted case: drop the torn record, keep the snapshot
    // + the good residual prefix, and let the next append reuse the dropped slot.
    let dir = TempDir::new("compact-then-torn");
    let log = dir.file("events.log");
    let deltas: [i64; 7] = [1, 1, 1, 1, 1, 1, 1];
    {
        let mut j = Journal::open(&log).expect("open");
        for d in deltas {
            j.append(&delta(d)).expect("append");
        }
        j.compact(3, &delta(4)).expect("compact"); // snapshot = sum 0..=3 = 4
    }
    // Chop a few bytes off the physical end → the last residual record is torn.
    let full_len = std::fs::metadata(&log).expect("meta").len();
    let f = OpenOptions::new()
        .write(true)
        .open(&log)
        .expect("open trunc");
    f.set_len(full_len - 3).expect("truncate");
    f.sync_all().expect("sync");
    drop(f);

    let j = Journal::open(&log).expect("reopen after torn tail");
    let (state, recs) = rebuild(&j);
    // Snapshot(4) + residual records 4,5 survive; record 6 was torn off.
    assert_eq!(recs[0].kind, RecordKind::Snapshot);
    assert_eq!(recs[0].sequence, 3);
    let residual_seqs: Vec<u64> = recs[1..].iter().map(|r| r.sequence).collect();
    assert_eq!(residual_seqs, vec![4, 5], "torn final residual dropped");
    assert_eq!(state, 4 + 1 + 1, "snapshot + two surviving deltas");
    // next_sequence falls to the dropped slot (6) — still monotone vs survivors.
    assert_eq!(j.next_sequence(), 6, "next reuses the torn record's slot");
}

#[test]
fn misplaced_snapshot_record_is_surfaced_as_interior_corruption() {
    // A CRC-valid snapshot record anywhere but first is interior corruption the
    // format CAN detect — it must be surfaced, not silently healed.
    let dir = TempDir::new("snapshot-misplaced");
    let log = dir.file("events.log");
    // data@0, then a (CRC-valid) snapshot frame in the middle — illegal placement.
    let mut bytes = frame_record(0, &delta(1));
    bytes.extend_from_slice(&forge_snapshot(0, &delta(99)));
    std::fs::write(&log, &bytes).expect("write forged");
    match Journal::open(&log) {
        Err(JournalError::CorruptInterior { reason, .. }) => {
            assert!(
                reason.contains("snapshot"),
                "reason names snapshot: {reason}"
            );
        }
        other => panic!("expected CorruptInterior, got {other:?}"),
    }
}

#[test]
fn snapshot_record_roundtrips_through_the_public_api() {
    // A direct check that a compacted log's snapshot payload survives byte-for-byte
    // through open → replay (independent of the running-sum machine): forge a log,
    // compact with arbitrary high/zero bytes in the snapshot, recover it.
    let dir = TempDir::new("snapshot-roundtrip");
    let log = dir.file("events.log");
    let snap = [0x00u8, 0xFF, 0xAA, 0x55, 0x01, 0x80, 0x7F];
    {
        let mut j = Journal::open(&log).expect("open");
        for i in 0..4u64 {
            j.append(format!("e{i}").as_bytes()).expect("append");
        }
        j.compact(2, &snap).expect("compact");
    }
    let j = Journal::open(&log).expect("reopen");
    let recs = j.records().expect("replay");
    assert_eq!(recs[0].kind, RecordKind::Snapshot);
    assert_eq!(
        recs[0].payload,
        snap.to_vec(),
        "snapshot bytes byte-identical"
    );
    assert_eq!(recs[0].sequence, 2);
    assert_eq!(recs[1].sequence, 3, "residual record 3 preserved");
    assert_eq!(recs[1].payload, b"e3".to_vec());
}
