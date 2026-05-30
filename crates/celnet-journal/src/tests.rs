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
