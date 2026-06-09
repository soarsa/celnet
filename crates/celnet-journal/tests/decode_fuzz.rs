//! Property-based adversarial-bytes hardening of the crash-recovery parser.
//!
//! After a crash the journal file on disk is *exactly* whatever bytes the kernel
//! managed to flush — i.e. arbitrary. `Journal::open` scans that file front-to-
//! back, heals a torn final record by truncation, and surfaces detectable
//! interior corruption as a typed error; `replay` then re-reads the recovered
//! prefix. Both are untrusted-input byte parsers, and a crafted `payload_len` /
//! `snapshot_len` field is a classic length-amplification point.
//!
//! Contract on **arbitrary** on-disk bytes:
//!   * **no panic** — `open`/`scan`/`read_one`/`read_snapshot`/`replay` never
//!     index out of bounds, never `unwrap` an attacker length, never wild-allocate
//!     (the `MAX_PAYLOAD_LEN` bound + short-read handling hold);
//!   * **heal-or-typed-Err** — `open` returns `Ok(Journal)` (having truncated any
//!     torn tail) or a typed [`celnet_journal::JournalError`]; `replay` likewise;
//!   * **truncation only shrinks** — the recovered durable region is never longer
//!     than the input.
//!
//! This is the same property the nightly `cargo-fuzz` `journal_recover` target
//! asserts under coverage-guided search; the fuzz crate is nightly-only and
//! excluded from the workspace gate, so this re-establishes it inside `just check`.
//!
//! The strategy mixes uniform-random bytes with structurally-valid frames (built
//! by appending real records, then mutated) so the CRC / sequence-monotonicity /
//! snapshot-marker / length-overrun branches are all reached.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use proptest::prelude::*;

use celnet_journal::{Journal, JournalError};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A self-cleaning unique temp directory (std-only, no external `tempfile` dep —
/// mirrors the crate's own test helper).
struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut path = std::env::temp_dir();
        path.push(format!("celnet-journal-fuzz-{pid}-{nanos}-{n}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir { path }
    }

    fn file(&self) -> std::path::PathBuf {
        self.path.join("log.journal")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Adversarial on-disk byte buffers (≤ 4 KiB):
///   * uniform-random bytes,
///   * a real journal image (some appended records) optionally mutated (flip a
///     byte to break a CRC; truncate to make a torn tail), and
///   * a buffer whose head is the SNAPSHOT_MARKER sentinel (`u32::MAX`) so the
///     snapshot-record branch (`read_snapshot`, inner length prefix) is exercised.
fn adversarial_journal_bytes() -> impl Strategy<Value = Vec<u8>> {
    let random = prop::collection::vec(any::<u8>(), 0..4096);

    let real_image = (
        prop::collection::vec(prop::collection::vec(any::<u8>(), 0..128), 0..16),
        any::<Option<usize>>(),
        any::<Option<usize>>(),
    )
        .prop_map(|(payloads, flip, truncate)| {
            // Build a genuine journal by appending the payloads, then read the
            // file bytes back and corrupt them — the structurally-valid-but-torn
            // corner that pure noise reaches only rarely.
            let dir = TempDir::new();
            let path = dir.file();
            {
                let mut j = Journal::open(&path).expect("open fresh journal");
                for p in &payloads {
                    // Payloads are < MAX_PAYLOAD_LEN, so append always succeeds.
                    j.append(p).expect("append");
                }
            }
            let mut bytes = std::fs::read(&path).unwrap_or_default();
            if let Some(i) = flip
                && !bytes.is_empty()
            {
                let idx = i % bytes.len();
                bytes[idx] ^= 0xff;
            }
            if let Some(t) = truncate
                && !bytes.is_empty()
            {
                bytes.truncate(t % bytes.len());
            }
            bytes
        });

    // A buffer that starts with the sync word + the snapshot sentinel + a
    // (possibly hostile) inner length, to drive `read_snapshot` and its length
    // bound. The sync word (ASCII "CLNJRNL\0" LE) must lead the frame under the
    // per-record sync-word format or recovery classifies the start as a torn tail
    // before reaching the snapshot branch.
    let snapshot_shaped =
        (any::<u32>(), prop::collection::vec(any::<u8>(), 0..256)).prop_map(|(inner_len, tail)| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&u64::from_le_bytes(*b"CLNJRNL\0").to_le_bytes()); // SYNC_WORD
            bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // SNAPSHOT_MARKER
            bytes.extend_from_slice(&7u64.to_le_bytes()); // watermark
            bytes.extend_from_slice(&inner_len.to_le_bytes()); // attacker inner length
            bytes.extend_from_slice(&tail);
            bytes
        });

    prop_oneof![random, real_image, snapshot_shaped]
}

proptest! {
    // Recovery does filesystem IO, so use a smaller-but-still-substantial budget.
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `Journal::open` + `replay` never panic on an arbitrary on-disk file; the
    /// recovered length never exceeds the input, and replay is self-consistent.
    #[test]
    fn open_and_replay_total(data in adversarial_journal_bytes()) {
        let dir = TempDir::new();
        let path = dir.file();
        {
            let mut f = std::fs::File::create(&path).expect("create log file");
            f.write_all(&data).expect("write log bytes");
        }
        let original_len = data.len() as u64;

        match Journal::open(&path) {
            Ok(journal) => {
                // Recovery truncates; it never grows the file.
                prop_assert!(journal.len_bytes() <= original_len);
                match journal.records() {
                    Ok(records) => {
                        let mut counted = 0u64;
                        if let Ok(n) = journal.replay(|_r| counted += 1) {
                            prop_assert_eq!(n, records.len() as u64);
                            prop_assert_eq!(counted, n);
                        }
                    }
                    Err(JournalError::CorruptInterior { .. } | JournalError::Io(_)) => {}
                    Err(JournalError::PayloadTooLarge { .. }) => {
                        prop_assert!(false, "replay never produces PayloadTooLarge");
                    }
                }
            }
            Err(JournalError::CorruptInterior { .. } | JournalError::Io(_)) => {}
            Err(JournalError::PayloadTooLarge { .. }) => {
                prop_assert!(false, "open never produces PayloadTooLarge");
            }
        }
    }
}
