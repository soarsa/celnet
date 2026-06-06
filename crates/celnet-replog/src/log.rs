//! The durable replicated log: a [`celnet_journal::Journal`]-backed sequence of
//! [`LogEntry`]s with the two mutations Raft requires — **append** (used by both
//! a leader extending its log and a follower mirroring the leader) and
//! **truncate-then-append** (used by a follower whose tail conflicts with the
//! leader's, Raft §5.3).
//!
//! # Why this wrapper exists (and how truncation is made durable)
//!
//! [`celnet_journal::Journal`] is an *append-only* WAL: it has no in-place
//! truncate. Raft's log-matching repair, however, requires a follower to **delete
//! a divergent suffix** of its durable log and overwrite it with the leader's
//! entries. We honour that durably — the journal on disk is the source of truth,
//! never a memory-only mask — with an **atomic rewrite**:
//!
//! 1. Read the surviving prefix `[0, keep)` of valid records from the live
//!    journal file.
//! 2. Write those records, followed by the new tail, into a sibling temp file
//!    (`<path>.rewrite`), each as a canonical journal record, and `fsync` it.
//! 3. `rename(2)` the temp file over the live path. `rename` is atomic on POSIX,
//!    so a crash mid-rewrite leaves either the *complete* old log or the
//!    *complete* new log — never a half-truncated state.
//! 4. `fsync` the parent directory so the rename itself is durable, then re-open
//!    the journal at the new contents.
//!
//! This mirrors the compaction recipe the journal's own module docs describe as
//! the forward-compatible bound (write fresh → fsync → atomic rename), applied
//! here to the conflicting-tail case. The sequence numbers in the rewritten file
//! stay `0,1,2,…` contiguous (the journal's monotonicity invariant), because a
//! Raft log index *is* the journal sequence by construction (entries are appended
//! in index order from 0).
//!
//! # Indices
//!
//! A log index is the journal sequence: the first entry is index 0. [`Log::len`]
//! is the count of entries; [`Log::last_index`] is `len-1` (or `None` if empty);
//! [`Log::last_term`] is the term of the last entry (0 if empty, the Raft
//! convention for "no prior entry"). [`Log::term_at`] returns the term stored at
//! a given index if present.

use std::fs;
use std::path::{Path, PathBuf};

use celnet_journal::{Journal, crc32};

use crate::entry::LogEntry;

/// A durable, index-addressed replicated log over a [`Journal`].
///
/// The in-memory `terms` vector (one `u64` per entry) is a cache of each entry's
/// term, rebuilt from the durable journal on open; it lets the hot log-matching
/// checks (`term_at`, `last_term`) answer without re-reading the file, while the
/// journal file remains the durable source of truth that every mutation `fsync`s.
pub struct Log {
    journal: Journal,
    path: PathBuf,
    /// `terms[i]` is the term of the entry at index `i`. Length == entry count.
    terms: Vec<u64>,
}

impl Log {
    /// Open (or create) the durable log at `path`, recovering its entries.
    ///
    /// # Errors
    ///
    /// Propagates journal-open / read IO errors.
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let journal = Journal::open(&path).map_err(to_io)?;
        let mut terms = Vec::new();
        journal
            .replay(|rec| {
                if let Ok(entry) = LogEntry::decode(&rec.payload) {
                    terms.push(entry.term);
                }
            })
            .map_err(to_io)?;
        Ok(Self {
            journal,
            path,
            terms,
        })
    }

    /// The number of entries in the log.
    #[must_use]
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// Whether the log holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The highest index present, or `None` if the log is empty.
    #[must_use]
    pub fn last_index(&self) -> Option<u64> {
        (self.terms.len() as u64).checked_sub(1)
    }

    /// The term of the last entry, or `0` if the log is empty (Raft's convention
    /// for "no previous entry", which makes the up-to-date comparison total).
    #[must_use]
    pub fn last_term(&self) -> u64 {
        self.terms.last().copied().unwrap_or(0)
    }

    /// The term of the entry at `index`, if such an entry exists.
    #[must_use]
    pub fn term_at(&self, index: u64) -> Option<u64> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.terms.get(i).copied())
    }

    /// Whether the log contains an entry at `prev_index` whose term equals
    /// `prev_term` — the Raft §5.3 log-matching precondition for accepting an
    /// AppendEntries with this `(prev_index, prev_term)`. A sentinel
    /// `prev_index == EMPTY_PREV` (no preceding entry) is always satisfied.
    #[must_use]
    pub fn matches_prev(&self, prev_index: u64, prev_term: u64) -> bool {
        if prev_index == EMPTY_PREV {
            return true; // the leader claims no preceding entry → always matches
        }
        self.term_at(prev_index) == Some(prev_term)
    }

    /// Append one entry at the end of the log (its index must equal the current
    /// length). Durable on return (`fsync`'d via the journal).
    ///
    /// # Errors
    ///
    /// Propagates journal append IO errors.
    pub fn append(&mut self, entry: &LogEntry) -> std::io::Result<()> {
        debug_assert_eq!(
            entry.index as usize,
            self.terms.len(),
            "append must extend the log contiguously"
        );
        self.journal.append(&entry.encode()).map_err(to_io)?;
        self.terms.push(entry.term);
        Ok(())
    }

    /// Reconcile the log with a run of leader `entries` that begin at
    /// `entries[0].index`, applying the Raft §5.3 conflicting-tail rule:
    ///
    /// * For each incoming entry, if the log already holds an entry at that index
    ///   with the **same term**, it is identical (log-matching guarantees it) and
    ///   is skipped (idempotent — no rewrite).
    /// * At the **first** incoming entry whose index either is beyond the log or
    ///   conflicts (same index, *different* term), the divergent suffix from that
    ///   index onward is **durably truncated** and the remaining incoming entries
    ///   are appended.
    ///
    /// Returns the new last index of the log. A pure-append (no conflict) takes
    /// the cheap journal-append path; a genuine conflict triggers one durable
    /// atomic rewrite (see the module docs).
    ///
    /// # Errors
    ///
    /// Propagates journal IO / rewrite errors.
    pub fn reconcile(&mut self, entries: &[LogEntry]) -> std::io::Result<Option<u64>> {
        if entries.is_empty() {
            return Ok(self.last_index());
        }
        // Find the first incoming entry that is new or conflicting.
        let mut first_new = 0usize;
        while first_new < entries.len() {
            let e = &entries[first_new];
            match self.term_at(e.index) {
                Some(t) if t == e.term => first_new += 1, // already present, identical
                Some(_) => break,                         // conflict: same index, diff term
                None => break,                            // beyond the end: pure new tail
            }
        }
        if first_new == entries.len() {
            return Ok(self.last_index()); // everything already present, idempotent
        }
        let tail = &entries[first_new..];
        let cut_index = tail[0].index;
        // Does the cut point fall inside the existing log (a real conflict needing
        // truncation) or exactly at the end (a pure append)?
        if (cut_index as usize) < self.terms.len() {
            self.truncate_and_append(cut_index, tail)?;
        } else {
            debug_assert_eq!(
                cut_index as usize,
                self.terms.len(),
                "AppendEntries must be contiguous after log-matching"
            );
            for e in tail {
                self.append(e)?;
            }
        }
        Ok(self.last_index())
    }

    /// Durably truncate the log to keep `[0, cut_index)` and append `tail`,
    /// via an atomic write-fresh → fsync → rename (module docs step 1–4).
    fn truncate_and_append(&mut self, cut_index: u64, tail: &[LogEntry]) -> std::io::Result<()> {
        // 1) Materialize the surviving prefix's entry payloads from the live log.
        let keep = cut_index as usize;
        let mut surviving: Vec<Vec<u8>> = Vec::with_capacity(keep + tail.len());
        {
            let recs = self.journal.records().map_err(to_io)?;
            for rec in recs.into_iter().take(keep) {
                surviving.push(rec.payload);
            }
        }
        if surviving.len() != keep {
            return Err(std::io::Error::other(format!(
                "log rewrite: expected {keep} surviving records, found {}",
                surviving.len()
            )));
        }
        for e in tail {
            surviving.push(e.encode());
        }

        // 2) Write the fresh log to a sibling temp file and fsync it.
        let tmp = rewrite_tmp_path(&self.path);
        write_fresh_journal(&tmp, &surviving)?;

        // 3) Atomically rename the temp over the live path (POSIX-atomic).
        fs::rename(&tmp, &self.path)?;

        // 4) Make the rename durable (parent-dir fsync) and re-open the journal.
        sync_parent_dir(&self.path)?;
        self.journal = Journal::open(&self.path).map_err(to_io)?;

        // Rebuild the in-memory term cache to exactly match the rewritten file.
        self.terms.truncate(keep);
        for e in tail {
            self.terms.push(e.term);
        }
        Ok(())
    }

    /// The encoded entry bytes of every durable record, in index order — the
    /// on-disk log for byte-identical cross-node comparison.
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn entry_bytes(&self) -> std::io::Result<Vec<Vec<u8>>> {
        let recs = self.journal.records().map_err(to_io)?;
        Ok(recs.into_iter().map(|r| r.payload).collect())
    }

    /// The decoded entry at `index`, if present.
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn entry_at(&self, index: u64) -> std::io::Result<Option<LogEntry>> {
        let recs = self.journal.records().map_err(to_io)?;
        let Ok(i) = usize::try_from(index) else {
            return Ok(None);
        };
        match recs.into_iter().nth(i) {
            Some(rec) => Ok(LogEntry::decode(&rec.payload).ok()),
            None => Ok(None),
        }
    }

    /// Every entry in `[from, last]` inclusive, decoded, in order (used by a
    /// leader to gather the slice it must replicate to a lagging follower).
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn entries_from(&self, from: u64) -> std::io::Result<Vec<LogEntry>> {
        let recs = self.journal.records().map_err(to_io)?;
        let Ok(start) = usize::try_from(from) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for rec in recs.into_iter().skip(start) {
            if let Ok(e) = LogEntry::decode(&rec.payload) {
                out.push(e);
            }
        }
        Ok(out)
    }
}

/// Sentinel `prev_index` meaning "no preceding entry" (the leader is sending the
/// very first entry, index 0). Distinct from a real index so `matches_prev`
/// short-circuits to `true`.
pub const EMPTY_PREV: u64 = u64::MAX;

/// The temp path used for the atomic log rewrite.
fn rewrite_tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".rewrite");
    PathBuf::from(s)
}

/// Write a fresh journal file at `tmp` containing exactly `payloads` (each an
/// already-encoded [`LogEntry`]), in canonical journal-record framing, fsync'd.
///
/// We build the file with the journal's own framing so re-opening it via
/// [`Journal::open`] yields byte-identical records — the same `(payload_len,
/// sequence, payload, crc32)` layout the journal writes itself. Sequence numbers
/// are `0,1,2,…` to preserve the journal monotonicity invariant.
fn write_fresh_journal(tmp: &Path, payloads: &[Vec<u8>]) -> std::io::Result<()> {
    use std::io::Write;
    // Remove any stale temp from a previous interrupted rewrite.
    let _ = fs::remove_file(tmp);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(tmp)?;
    let mut buf = Vec::new();
    for (seq, payload) in payloads.iter().enumerate() {
        let len_u32 = u32::try_from(payload.len())
            .map_err(|_| std::io::Error::other("log rewrite payload too large"))?;
        let mut frame = Vec::with_capacity(4 + 8 + payload.len() + 4);
        frame.extend_from_slice(&len_u32.to_le_bytes());
        frame.extend_from_slice(&(seq as u64).to_le_bytes());
        frame.extend_from_slice(payload);
        let checksum = crc32(&frame);
        frame.extend_from_slice(&checksum.to_le_bytes());
        buf.extend_from_slice(&frame);
    }
    file.write_all(&buf)?;
    file.sync_data()?;
    Ok(())
}

/// Fsync the directory containing `path` so a rename into it is durable.
#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    fs::File::open(&dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn to_io<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_log() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "celnet-replog-log-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.push("log.journal");
        dir
    }

    fn e(term: u64, index: u64, p: u8) -> LogEntry {
        LogEntry::new(term, index, vec![p])
    }

    #[test]
    fn append_and_recover_terms() {
        let path = temp_log();
        {
            let mut log = Log::open(&path).unwrap();
            log.append(&e(1, 0, 10)).unwrap();
            log.append(&e(1, 1, 11)).unwrap();
            log.append(&e(2, 2, 12)).unwrap();
            assert_eq!(log.len(), 3);
            assert_eq!(log.last_index(), Some(2));
            assert_eq!(log.last_term(), 2);
            assert_eq!(log.term_at(1), Some(1));
        }
        // Reopen: the term cache is rebuilt from disk.
        let log = Log::open(&path).unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log.last_term(), 2);
        assert_eq!(log.term_at(2), Some(2));
    }

    #[test]
    fn matches_prev_enforces_log_matching() {
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        log.append(&e(1, 0, 1)).unwrap();
        log.append(&e(3, 1, 2)).unwrap();
        assert!(log.matches_prev(EMPTY_PREV, 0)); // no preceding entry
        assert!(log.matches_prev(1, 3)); // index 1 has term 3
        assert!(!log.matches_prev(1, 2)); // wrong term
        assert!(!log.matches_prev(2, 1)); // no entry at index 2
    }

    #[test]
    fn reconcile_pure_append() {
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        log.append(&e(1, 0, 1)).unwrap();
        let last = log.reconcile(&[e(1, 1, 2), e(1, 2, 3)]).unwrap().unwrap();
        assert_eq!(last, 2);
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn reconcile_idempotent_skips_present_entries() {
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        log.append(&e(1, 0, 1)).unwrap();
        log.append(&e(1, 1, 2)).unwrap();
        // Re-deliver the exact same entries + one new — only the new one appends.
        let last = log
            .reconcile(&[e(1, 0, 1), e(1, 1, 2), e(2, 2, 3)])
            .unwrap()
            .unwrap();
        assert_eq!(last, 2);
        assert_eq!(log.len(), 3);
        assert_eq!(log.term_at(2), Some(2));
    }

    #[test]
    fn reconcile_truncates_conflicting_tail_durably() {
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        // Existing divergent tail: indices 0,1,2 in an old term 1, with a stale
        // index-2 that the new leader will overwrite.
        log.append(&e(1, 0, 100)).unwrap();
        log.append(&e(1, 1, 101)).unwrap();
        log.append(&e(1, 2, 199)).unwrap(); // stale — conflicts below

        // New leader's entries: index 2 now in term 2 (different term ⇒ conflict),
        // plus a fresh index 3. Index 0,1 match and are kept.
        let last = log
            .reconcile(&[e(1, 1, 101), e(2, 2, 200), e(2, 3, 201)])
            .unwrap()
            .unwrap();
        assert_eq!(last, 3);
        assert_eq!(log.len(), 4);
        assert_eq!(log.term_at(2), Some(2));
        assert_eq!(log.term_at(3), Some(2));

        // The truncation is DURABLE: re-open from disk and confirm.
        drop(log);
        let log = Log::open(&path).unwrap();
        assert_eq!(log.len(), 4);
        assert_eq!(log.term_at(2), Some(2));
        let entry2 = log.entry_at(2).unwrap().unwrap();
        assert_eq!(entry2.payload, vec![200u8]);
        // And the surviving prefix bytes are intact.
        let entry0 = log.entry_at(0).unwrap().unwrap();
        assert_eq!(entry0.payload, vec![100u8]);
    }
}
