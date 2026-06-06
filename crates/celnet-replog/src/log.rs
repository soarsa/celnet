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
//! # Indices and the base-index offset (log compaction, §7)
//!
//! Without compaction, a Raft log index coincides with the journal sequence: the
//! first entry is index 0 at physical position 0. **Log prefix discard**
//! ([`Log::discard_prefix`], used after a snapshot — see [`crate::compaction`])
//! breaks that 1:1 mapping: the physical journal no longer starts at index 0.
//!
//! We restore correctness with a **base-index offset**. [`Log`] tracks:
//!
//! * `base_index` — the *absolute* Raft index of physical position 0 (i.e.
//!   `last_included_index + 1` after a snapshot, or `0` with no snapshot).
//! * `snapshot_index` / `snapshot_term` — the boundary `(last_included_index,
//!   last_included_term)` of the most recent snapshot whose prefix was discarded
//!   (`None` / `0` if no prefix has ever been discarded).
//!
//! Physical position `p` holds **absolute index `base_index + p`**. Every public
//! accessor speaks *absolute* indices and translates internally:
//! `physical = absolute - base_index`. The on-disk journal keeps its own
//! contiguous sequences `0,1,2,…` (its monotonicity invariant); only the
//! *interpretation* of position ↔ index shifts by `base_index`.
//!
//! `term_at` answers correctly for the **snapshot boundary** index too: even
//! though `log[last_included_index]`'s body is discarded, `term_at(snapshot_index)
//! == Some(snapshot_term)`, so `matches_prev(last_included_index,
//! last_included_term)` still succeeds — a leader replicating the very first entry
//! *after* the snapshot boundary matches. Indices strictly below the snapshot
//! boundary are gone (subsumed by the snapshot) and `term_at` returns `None`.
//!
//! [`Log::len`] is the count of *physically retained* entries; [`Log::last_index`]
//! is the absolute index of the last retained entry (or, if the tail is empty but
//! a snapshot exists, the snapshot's `last_included_index`); [`Log::last_term`] is
//! the term of the last entry (the snapshot term if the tail is empty, else 0 for
//! a wholly-empty log — the Raft convention for "no prior entry").

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
    /// `terms[p]` is the term of the entry at **physical position** `p`, i.e. at
    /// absolute index `base_index + p`. Length == retained entry count.
    terms: Vec<u64>,
    /// Absolute Raft index of physical position 0 (`last_included_index + 1` after
    /// a snapshot prefix discard, else `0`).
    base_index: u64,
    /// `(last_included_index, last_included_term)` of the most recent discarded
    /// prefix, or `None` if no prefix has ever been discarded. `snapshot_index`,
    /// when present, equals `base_index - 1`.
    snapshot_index: Option<u64>,
    /// Term of the entry at `snapshot_index` (meaningful only when
    /// `snapshot_index` is `Some`), retained so log-matching still succeeds at the
    /// snapshot boundary after the prefix body is gone.
    snapshot_term: u64,
}

impl Log {
    /// Open (or create) the durable log at `path`, recovering its entries. The
    /// log opens with **no base offset** (`base_index == 0`); a caller that holds
    /// a durable snapshot adopts the boundary via [`Log::adopt_snapshot_boundary`]
    /// so the absolute-index accessors are correct over the already-discarded
    /// prefix.
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
            base_index: 0,
            snapshot_index: None,
            snapshot_term: 0,
        })
    }

    /// Adopt a snapshot boundary `(last_included_index, last_included_term)` whose
    /// prefix has **already been discarded** from the physical log (recovery
    /// path): the retained physical entries begin at absolute index
    /// `last_included_index + 1`. This sets `base_index` accordingly so every
    /// absolute-index accessor is correct without rewriting the journal.
    ///
    /// The first physically-retained entry (if any) must carry absolute index
    /// `last_included_index + 1`; this is asserted in debug builds.
    pub fn adopt_snapshot_boundary(&mut self, last_included_index: u64, last_included_term: u64) {
        let base = last_included_index + 1;
        debug_assert!(
            self.entry_at_physical(0)
                .ok()
                .flatten()
                .is_none_or(|e| e.index == base),
            "retained physical head must be at last_included_index + 1"
        );
        self.base_index = base;
        self.snapshot_index = Some(last_included_index);
        self.snapshot_term = last_included_term;
    }

    /// The absolute index of physical position 0 (`last_included_index + 1` after
    /// a snapshot, else `0`).
    #[must_use]
    pub fn base_index(&self) -> u64 {
        self.base_index
    }

    /// The boundary index of the most recent discarded prefix, or `None`.
    #[must_use]
    pub fn snapshot_index(&self) -> Option<u64> {
        self.snapshot_index
    }

    /// The number of **physically retained** entries in the log (the tail after
    /// any discarded prefix).
    #[must_use]
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// Whether the log holds no **physically retained** entries (a fresh log, or
    /// one whose entire tail has been subsumed by a snapshot).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The highest **absolute** index present, or `None` if nothing is known.
    ///
    /// When the physical tail is empty but a snapshot boundary has been adopted,
    /// this is the snapshot's `last_included_index` (the highest index the log
    /// accounts for), so a follower whose tail is wholly subsumed still reports
    /// the correct high-water.
    #[must_use]
    pub fn last_index(&self) -> Option<u64> {
        match (self.terms.len() as u64).checked_sub(1) {
            Some(last_phys) => Some(self.base_index + last_phys),
            None => self.snapshot_index,
        }
    }

    /// The term of the last entry. When the physical tail is empty this is the
    /// snapshot term (if a snapshot exists), else `0` for a wholly-empty log
    /// (Raft's convention for "no previous entry", making the up-to-date
    /// comparison total).
    #[must_use]
    pub fn last_term(&self) -> u64 {
        match self.terms.last().copied() {
            Some(t) => t,
            None => self.snapshot_term, // 0 when no snapshot, the snapshot term otherwise
        }
    }

    /// The term of the entry at **absolute** `index`, if it is known.
    ///
    /// Returns the snapshot term when `index == last_included_index` (so
    /// log-matching at the snapshot boundary succeeds even though the entry body
    /// is discarded). Indices strictly below the snapshot boundary are gone and
    /// yield `None`; indices in the retained tail read the cached term.
    #[must_use]
    pub fn term_at(&self, index: u64) -> Option<u64> {
        if self.snapshot_index == Some(index) {
            return Some(self.snapshot_term);
        }
        let phys = index.checked_sub(self.base_index)?;
        usize::try_from(phys)
            .ok()
            .and_then(|p| self.terms.get(p).copied())
    }

    /// Whether the log contains (or accounts for) an entry at absolute
    /// `prev_index` whose term equals `prev_term` — the Raft §5.3 log-matching
    /// precondition for accepting an AppendEntries with this `(prev_index,
    /// prev_term)`. A sentinel `prev_index == EMPTY_PREV` (no preceding entry) is
    /// always satisfied, and `prev_index == last_included_index` matches against
    /// the retained snapshot term (so replication right after the snapshot
    /// boundary succeeds).
    #[must_use]
    pub fn matches_prev(&self, prev_index: u64, prev_term: u64) -> bool {
        if prev_index == EMPTY_PREV {
            return true; // the leader claims no preceding entry → always matches
        }
        self.term_at(prev_index) == Some(prev_term)
    }

    /// Append one entry at the end of the log (its **absolute** index must equal
    /// `base_index + len`, i.e. the next absolute index). Durable on return
    /// (`fsync`'d via the journal).
    ///
    /// # Errors
    ///
    /// Propagates journal append IO errors.
    pub fn append(&mut self, entry: &LogEntry) -> std::io::Result<()> {
        debug_assert_eq!(
            entry.index,
            self.base_index + self.terms.len() as u64,
            "append must extend the log contiguously at the next absolute index"
        );
        self.journal.append(&entry.encode()).map_err(to_io)?;
        self.terms.push(entry.term);
        Ok(())
    }

    /// Decode the entry at **physical position** `p`, if present (internal helper;
    /// callers outside this module use the absolute-index [`Log::entry_at`]).
    fn entry_at_physical(&self, p: usize) -> std::io::Result<Option<LogEntry>> {
        let recs = self.journal.records().map_err(to_io)?;
        match recs.into_iter().nth(p) {
            Some(rec) => Ok(LogEntry::decode(&rec.payload).ok()),
            None => Ok(None),
        }
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
        let cut_index = tail[0].index; // absolute index of the first divergent entry
        // Translate the absolute cut point to a physical position over the retained
        // tail. After log-matching the cut never falls below the snapshot boundary
        // (a committed, snapshotted prefix is never overwritten — Raft safety), so
        // `cut_index >= base_index`.
        debug_assert!(
            cut_index >= self.base_index,
            "reconcile must never cut below the discarded-prefix boundary"
        );
        let cut_phys = (cut_index - self.base_index) as usize;
        // Does the cut point fall inside the retained tail (a real conflict needing
        // truncation) or exactly at the end (a pure append)?
        if cut_phys < self.terms.len() {
            self.truncate_and_append(cut_phys, tail)?;
        } else {
            debug_assert_eq!(
                cut_phys,
                self.terms.len(),
                "AppendEntries must be contiguous after log-matching"
            );
            for e in tail {
                self.append(e)?;
            }
        }
        Ok(self.last_index())
    }

    /// Durably truncate the retained tail to keep physical `[0, keep_phys)` and
    /// append `tail`, via an atomic write-fresh → fsync → rename (module docs
    /// step 1–4). `keep_phys` is a **physical** position over the retained tail.
    fn truncate_and_append(&mut self, keep_phys: usize, tail: &[LogEntry]) -> std::io::Result<()> {
        // 1) Materialize the surviving prefix's entry payloads from the live log.
        let mut surviving: Vec<Vec<u8>> = Vec::with_capacity(keep_phys + tail.len());
        {
            let recs = self.journal.records().map_err(to_io)?;
            for rec in recs.into_iter().take(keep_phys) {
                surviving.push(rec.payload);
            }
        }
        if surviving.len() != keep_phys {
            return Err(std::io::Error::other(format!(
                "log rewrite: expected {keep_phys} surviving records, found {}",
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
        self.terms.truncate(keep_phys);
        for e in tail {
            self.terms.push(e.term);
        }
        Ok(())
    }

    /// Durably discard the committed prefix `[base_index, last_included_index]`
    /// (Raft §7 log compaction), retaining the tail `[last_included_index + 1, ..]`
    /// and shifting `base_index` to `last_included_index + 1`. The physical journal
    /// file is **really shrunk** on disk (the surviving tail is rewritten via the
    /// same atomic write-fresh → fsync → rename → dir-fsync discipline the
    /// conflicting-tail truncation uses); this is never a memory-only mask.
    ///
    /// `last_included_term` must be the term of the entry at `last_included_index`
    /// — captured by the caller from the snapshot. After this call `term_at(
    /// last_included_index) == Some(last_included_term)` so log-matching still
    /// succeeds at the new boundary even though that entry's body is gone.
    ///
    /// A no-op (already discarded at or beyond `last_included_index`) returns
    /// `Ok(())` without touching the disk. **Never** discards beyond the entries
    /// the log physically holds (the caller only ever passes a committed,
    /// applied index, which is always present), and never discards an entry that
    /// is not durably present.
    ///
    /// # Errors
    ///
    /// Propagates journal IO / rewrite errors, or an [`std::io::Error`] if
    /// `last_included_index` is outside the retained range (a caller bug — a
    /// committed index is always within range).
    pub fn discard_prefix(
        &mut self,
        last_included_index: u64,
        last_included_term: u64,
    ) -> std::io::Result<()> {
        // Idempotent: nothing to do if the prefix is already gone.
        if last_included_index < self.base_index {
            return Ok(());
        }
        let drop_count = (last_included_index - self.base_index + 1) as usize;
        if drop_count > self.terms.len() {
            return Err(std::io::Error::other(format!(
                "discard_prefix: last_included_index {last_included_index} beyond retained \
                 range (base {}, len {})",
                self.base_index,
                self.terms.len()
            )));
        }
        // Validate the caller-supplied boundary term against what we hold.
        debug_assert_eq!(
            self.terms[drop_count - 1],
            last_included_term,
            "discard_prefix boundary term must match the entry being subsumed"
        );

        // Materialize the surviving tail's entry payloads (physical positions
        // [drop_count, len)).
        let mut surviving: Vec<Vec<u8>> = Vec::with_capacity(self.terms.len() - drop_count);
        {
            let recs = self.journal.records().map_err(to_io)?;
            for rec in recs.into_iter().skip(drop_count) {
                surviving.push(rec.payload);
            }
        }
        let expected = self.terms.len() - drop_count;
        if surviving.len() != expected {
            return Err(std::io::Error::other(format!(
                "discard_prefix: expected {expected} surviving records, found {}",
                surviving.len()
            )));
        }

        // Atomically rewrite the journal to hold only the surviving tail (the
        // journal re-numbers its own sequences 0,1,2,… — the absolute indices are
        // recovered via base_index, not the sequence).
        let tmp = rewrite_tmp_path(&self.path);
        write_fresh_journal(&tmp, &surviving)?;
        fs::rename(&tmp, &self.path)?;
        sync_parent_dir(&self.path)?;
        self.journal = Journal::open(&self.path).map_err(to_io)?;

        // Shift the base and drop the discarded terms from the cache.
        self.terms.drain(0..drop_count);
        self.base_index = last_included_index + 1;
        self.snapshot_index = Some(last_included_index);
        self.snapshot_term = last_included_term;
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

    /// The decoded entry at **absolute** `index`, if it is physically retained.
    /// An index below the discarded-prefix boundary (subsumed by a snapshot)
    /// yields `None` — its body is gone, captured in the snapshot instead.
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn entry_at(&self, index: u64) -> std::io::Result<Option<LogEntry>> {
        let Some(phys) = index.checked_sub(self.base_index) else {
            return Ok(None); // below the discarded prefix
        };
        let Ok(p) = usize::try_from(phys) else {
            return Ok(None);
        };
        self.entry_at_physical(p)
    }

    /// Every retained entry from **absolute** `from` to the last, decoded, in
    /// order (used by a leader to gather the slice it must replicate to a lagging
    /// follower). A `from` below the discarded-prefix boundary is clamped to the
    /// first retained entry — the caller (leader) detects that case via
    /// [`Log::base_index`] / [`Log::snapshot_index`] and ships a snapshot instead
    /// of entries when the follower needs something the log no longer holds.
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn entries_from(&self, from: u64) -> std::io::Result<Vec<LogEntry>> {
        // Translate absolute → physical; clamp a sub-base request to position 0.
        let start = from.saturating_sub(self.base_index);
        let Ok(start) = usize::try_from(start) else {
            return Ok(Vec::new());
        };
        let recs = self.journal.records().map_err(to_io)?;
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

    #[test]
    fn discard_prefix_shrinks_disk_and_keeps_absolute_indices() {
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        for i in 0..6u64 {
            log.append(&e(1, i, i as u8)).unwrap();
        }
        assert_eq!(log.len(), 6);
        assert_eq!(log.base_index(), 0);

        // Discard [0, 2]; retain [3, 5]. Boundary entry-2 has term 1.
        log.discard_prefix(2, 1).unwrap();

        // The PHYSICAL log really shrank on disk.
        assert_eq!(log.len(), 3, "retained tail count");
        assert_eq!(
            log.entry_bytes().unwrap().len(),
            3,
            "physical records on disk"
        );
        assert_eq!(log.base_index(), 3);
        assert_eq!(log.snapshot_index(), Some(2));

        // Absolute-index accessors stay correct over the shifted log.
        assert_eq!(log.last_index(), Some(5));
        assert_eq!(log.term_at(2), Some(1)); // snapshot boundary term retained
        assert_eq!(log.term_at(3), Some(1)); // first retained entry
        assert_eq!(log.term_at(1), None); // subsumed by the snapshot, body gone
        assert_eq!(log.entry_at(1).unwrap(), None);
        let e3 = log.entry_at(3).unwrap().unwrap();
        assert_eq!(e3.index, 3);
        assert_eq!(e3.payload, vec![3u8]);

        // matches_prev succeeds AT the snapshot boundary (term-1) — so a leader
        // replicating index 3 right after the boundary matches.
        assert!(log.matches_prev(2, 1));
        assert!(!log.matches_prev(2, 9)); // wrong boundary term
        assert!(log.matches_prev(5, 1)); // a retained entry

        // entries_from over the retained range.
        let from3 = log.entries_from(3).unwrap();
        assert_eq!(from3.len(), 3);
        assert_eq!(from3[0].index, 3);
        assert_eq!(from3[2].index, 5);
    }

    #[test]
    fn discard_prefix_survives_reopen_with_adopt_boundary() {
        let path = temp_log();
        {
            let mut log = Log::open(&path).unwrap();
            for i in 0..5u64 {
                log.append(&e(2, i, (10 + i) as u8)).unwrap();
            }
            log.discard_prefix(2, 2).unwrap();
        }
        // Re-open: the physical log holds only the retained tail; recovery adopts
        // the boundary so absolute indices are correct again.
        let mut log = Log::open(&path).unwrap();
        assert_eq!(log.len(), 2, "only the retained tail is on disk");
        // Before adopting, the head physical entry carries absolute index 3.
        assert_eq!(log.entry_at_physical(0).unwrap().unwrap().index, 3);
        log.adopt_snapshot_boundary(2, 2);
        assert_eq!(log.base_index(), 3);
        assert_eq!(log.last_index(), Some(4));
        assert_eq!(log.term_at(2), Some(2)); // boundary
        assert_eq!(log.term_at(3), Some(2));
        assert!(log.matches_prev(2, 2));
        let e4 = log.entry_at(4).unwrap().unwrap();
        assert_eq!(e4.index, 4);
        assert_eq!(e4.payload, vec![14u8]);
    }

    #[test]
    fn append_after_discard_extends_at_absolute_index() {
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        for i in 0..4u64 {
            log.append(&e(1, i, i as u8)).unwrap();
        }
        log.discard_prefix(1, 1).unwrap(); // retain [2,3], base=2
        assert_eq!(log.last_index(), Some(3));
        log.append(&e(1, 4, 44)).unwrap(); // next absolute index is 4
        assert_eq!(log.last_index(), Some(4));
        assert_eq!(log.term_at(4), Some(1));
        let e4 = log.entry_at(4).unwrap().unwrap();
        assert_eq!(e4.payload, vec![44u8]);
    }

    #[test]
    fn discard_prefix_idempotent_below_base() {
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        for i in 0..4u64 {
            log.append(&e(1, i, i as u8)).unwrap();
        }
        log.discard_prefix(2, 1).unwrap(); // base = 3
        // A repeat at or below the existing boundary is a no-op.
        log.discard_prefix(2, 1).unwrap();
        log.discard_prefix(1, 1).unwrap();
        assert_eq!(log.base_index(), 3);
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn discard_whole_tail_then_replicate_after_boundary() {
        // Discard everything physically held; tail becomes empty but last_index
        // still reflects the snapshot boundary and matches_prev works there.
        let path = temp_log();
        let mut log = Log::open(&path).unwrap();
        for i in 0..3u64 {
            log.append(&e(4, i, i as u8)).unwrap();
        }
        log.discard_prefix(2, 4).unwrap(); // retain nothing, base = 3
        assert!(log.is_empty());
        assert_eq!(log.last_index(), Some(2));
        assert_eq!(log.last_term(), 4);
        assert!(log.matches_prev(2, 4));
        // A leader can now replicate index 3 right after the boundary.
        let last = log.reconcile(&[e(4, 3, 33)]).unwrap().unwrap();
        assert_eq!(last, 3);
        assert_eq!(log.entry_at(3).unwrap().unwrap().payload, vec![33u8]);
    }
}
