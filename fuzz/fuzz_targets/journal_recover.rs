//! Fuzz target: arbitrary bytes on disk -> `celnet_journal::Journal::open` +
//! `replay` (the crash-recovery / torn-tail parser).
//!
//! The journal is the durable substrate: on restart it scans a flat file of
//! back-to-back CRC-framed records front-to-back, heals a torn final record
//! (crash mid-append) by truncation, and surfaces detectable interior corruption
//! as a typed error. After a crash the on-disk bytes are *exactly* whatever the
//! kernel managed to flush — i.e. arbitrary — so the open/scan/replay path is a
//! genuinely untrusted byte parser, and a hostile `payload_len`/`snapshot_len`
//! field is the length-amplification point (`read_one`/`read_snapshot` bound the
//! allocation by `MAX_PAYLOAD_LEN` and a short read).
//!
//! This target writes the fuzzer's arbitrary bytes to a real temp file and drives
//! the actual recovery code (no re-implementation), asserting:
//!   * `open` never panics — it returns `Ok(Journal)` (healing a torn tail) or a
//!     typed [`celnet_journal::JournalError`] (`Io` / `CorruptInterior`); it never
//!     wild-allocates on a crafted length field, and
//!   * `replay`/`records` on a successfully-opened journal never panics and is
//!     consistent (the recovered byte length never exceeds the original input;
//!     truncation only ever shrinks the file).
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run journal_recover -- -max_total_time=120

#![no_main]

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use libfuzzer_sys::fuzz_target;

use celnet_journal::{Journal, JournalError};

static SEQ: AtomicU64 = AtomicU64::new(0);

fuzz_target!(|data: &[u8]| {
    // A unique temp path per iteration (the recovery path opens/truncates a real
    // file). We clean it up at the end of the iteration.
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut dir = std::env::temp_dir();
    dir.push(format!("celnet-fuzz-journal-{pid}-{n}"));
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("log.journal");

    // Lay down the arbitrary bytes as the (possibly torn/corrupt) log file.
    {
        let Ok(mut f) = std::fs::File::create(&path) else {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        if f.write_all(data).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
    }
    let original_len = data.len() as u64;

    // Contract 1: open/scan/torn-tail recovery never panics — Ok or typed Err.
    match Journal::open(&path) {
        Ok(journal) => {
            // The healed (durable) region can only ever be a prefix of the input:
            // recovery truncates, it never grows the file.
            assert!(
                journal.len_bytes() <= original_len,
                "recovered len {} > input {}",
                journal.len_bytes(),
                original_len
            );
            // Contract 2: replay of the recovered prefix never panics and is
            // internally consistent (its count matches `records()`).
            match journal.records() {
                Ok(records) => {
                    let mut counted = 0u64;
                    let replayed = journal.replay(|_rec| counted += 1);
                    if let Ok(n_replayed) = replayed {
                        assert_eq!(
                            n_replayed,
                            records.len() as u64,
                            "replay count disagrees with records()"
                        );
                        assert_eq!(counted, n_replayed, "callback count disagrees");
                    }
                }
                Err(JournalError::CorruptInterior { .. } | JournalError::Io(_)) => {}
                // PayloadTooLarge is an append-side error, never produced on read.
                Err(JournalError::PayloadTooLarge { .. }) => {
                    unreachable!("replay never produces PayloadTooLarge")
                }
            }
        }
        Err(JournalError::CorruptInterior { .. } | JournalError::Io(_)) => {
            // Detectable interior corruption / a real IO fault — honest rejections.
        }
        Err(JournalError::PayloadTooLarge { .. }) => {
            unreachable!("open never produces PayloadTooLarge")
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
});
