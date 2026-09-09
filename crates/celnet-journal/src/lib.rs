//! Celnet durable event journal — the standalone crash-recovery substrate
//! (`docs/SCALE-OUT.md` §8; closes the "designed-only" durable-log gap).
//!
//! An **fsync'd, append-only, sequence-ordered** log of the must-order durable
//! events (accepted market-state updates + the quote/trade lifecycle). A
//! control-plane / edge consumer (the engine's booking + accepted-market-state
//! path, where blue-green handoff and the audit sink already live) appends to it;
//! on restart it [`Journal::replay`]s the log to rebuild the live book / marked-
//! surface state **bit-identically** (determinism: libm + counter-based RNG ⇒
//! identical reconstruction). Each record carries a monotonic sequence and a
//! CRC-32 checksum; a torn tail (crash mid-append) is detected and truncated
//! cleanly so recovery never reads a partial record.
//!
//! # Where this is *not* used
//!
//! The journal performs IO and `fsync` and therefore **must never** be called
//! from the pinned, zero-alloc, busy-poll hot pricing path in `celnet-engine`.
//! Journaling is a control-plane concern: the price() loop stays lock/log/alloc/
//! IO-free, and recovery rebuild happens at **startup**, not on the hot path.
//!
//! # On-disk format
//!
//! The log is a flat file of back-to-back records. Every record (data and
//! snapshot) begins with a fixed 8-byte **sync word** ([`SYNC_WORD`], ASCII
//! `"CLNJRNL\0"` little-endian) — a start-of-frame magic that lets recovery
//! resynchronize and, crucially, tell an interior CRC failure apart from a torn
//! tail (see the failure model below). A normal **data** record is:
//!
//! ```text
//! ┌────────────┬──────────────┬──────────────┬─────────────────┬──────────────┐
//! │ SYNC_WORD  │ payload_len  │  sequence    │   payload       │   crc32      │
//! │  u64 LE    │  u32 LE      │  u64 LE      │  payload_len B  │  u32 LE      │
//! └────────────┴──────────────┴──────────────┴─────────────────┴──────────────┘
//! ```
//!
//! The CRC-32 (IEEE, see [`crc32`]) covers the sync word, the framed header **and**
//! the payload (`SYNC_WORD || payload_len || sequence || payload`), so a flipped
//! byte anywhere in the record — including a corrupted sync byte — is detected. A
//! record whose frame is truncated (a short read at EOF), or whose start does not
//! carry the sync word, is treated as a torn tail: recovery stops at the last good
//! record and truncates the file to that offset.
//!
//! A leading **snapshot** record (written only by [`Journal::compact`]) reuses the
//! exact same frame, distinguished losslessly by a **sentinel in the `payload_len`
//! field**: `payload_len == SNAPSHOT_MARKER` (`u32::MAX`). A real data payload can
//! never reach that value — it is bounded by [`MAX_PAYLOAD_LEN`] (64 MiB), and any
//! `payload_len > MAX_PAYLOAD_LEN` was already treated as corruption — so the
//! sentinel is unambiguous and the **data-record layout is byte-for-byte
//! unchanged** (no per-record type tag is added to the common case). For a
//! snapshot record the `sequence` field carries the checkpoint **watermark** and
//! the *true* snapshot length is stored as a leading `u32` of its payload region:
//!
//! ```text
//! ┌────────────┬──────────────┬──────────────┬──────────────┬────────────┬──────────┐
//! │ SYNC_WORD  │ SNAPSHOT_MARK│  watermark   │ snapshot_len │ snapshot B │  crc32   │
//! │  u64 LE    │  u32 = MAX   │  u64 LE      │  u32 LE      │ snapshot_l │  u32 LE  │
//! └────────────┴──────────────┴──────────────┴──────────────┴────────────┴──────────┘
//! ```
//!
//! The CRC still covers everything before it (including the sync word), so a
//! corrupt snapshot record is detected exactly like a data record.
//!
//! # Durability contract
//!
//! [`Journal::append`] returns only after the bytes are written **and** the OS
//! file buffers are flushed to stable storage via `fsync` (`File::sync_data`).
//! Once `append` returns `Ok(seq)`, the record survives a crash. (Hardware that
//! lies about flush — e.g. a volatile disk write cache with FUA disabled — is
//! outside the software durability boundary; the contract is the syscall.) The
//! file's *directory entry* is made durable too: [`Journal::open`] fsyncs the
//! parent directory once (covering first creation), so the first record cannot be
//! lost to an un-synced directory on the filesystems where that matters.
//!
//! # Failure model
//!
//! The per-record [`SYNC_WORD`] is the resync marker that lets recovery separate
//! the two post-crash conditions a marker-less format conflated — so an interior
//! CRC failure is **surfaced**, not silently healed:
//!
//! * A **torn tail** (a crash mid-`append`) is the expected post-crash state. It
//!   manifests as one of: zero bytes at a record boundary (clean EOF); a short read
//!   of the sync word, header, payload or CRC trailer of the final frame; or
//!   trailing bytes at the physical tail whose leading 8 bytes are not the sync
//!   word. In every case recovery stops at the last good record and truncates,
//!   returning **no error** — correct, since the torn bytes were never acknowledged
//!   (`append` had not returned `Ok`).
//! * An **interior CRC failure** — an intact sync word followed by a *complete*
//!   frame body (payload/snapshot bytes **and** the CRC trailer all present) whose
//!   CRC nonetheless fails — is **bit-rot of a fully-written record**, not a torn
//!   tail. A torn tail cannot reach this state: an interrupted append leaves the
//!   body *short* (a missing/partial CRC trailer, classified as a torn tail above),
//!   and it cannot forge a complete CRC trailer for bytes it never wrote. So this
//!   case is surfaced as [`JournalError::CorruptInterior`] rather than truncating
//!   the (possibly already-acknowledged) records after it. This closes the
//!   limitation the marker-less format documented.
//! * The other interior inconsistencies the format detects — a CRC-**valid** data
//!   record whose sequence number is non-monotonic, or a CRC-valid snapshot record
//!   found anywhere but first — are likewise surfaced as
//!   [`JournalError::CorruptInterior`] rather than healed.
//!
//! > **Conservative bias on adversarial garbage.** Trailing garbage from an
//! > interrupted append that happens to be ≥ 8 bytes but is *not* the sync word is
//! > handled as a torn tail. The pathological case "garbage that exactly equals the
//! > sync word, followed by a complete-but-CRC-bad body" is astronomically
//! > improbable (an interrupted append cannot produce a complete CRC trailer for
//! > bytes it never wrote); were it ever to occur, surfacing it as
//! > `CorruptInterior` is the *safe* failure — stop and report rather than silently
//! > drop committed records. This bias is intentional, not hand-waved.
//!
//! # Checkpointing & compaction
//!
//! An unbounded append-only log replays in time linear in its length. The bound
//! is a **checkpoint + compaction** cycle, implemented by [`Journal::compact`]:
//!
//! 1. A consumer periodically snapshots its rebuilt state and records the highest
//!    sequence the snapshot covers (a *checkpoint watermark*).
//! 2. [`Journal::compact`] writes a fresh log to a sibling temp file containing,
//!    in order: a leading **snapshot record** carrying the watermark sequence and
//!    the consumer's opaque snapshot bytes, then every **residual** data record
//!    (sequence *strictly greater than* the watermark) copied **byte-for-byte**
//!    with its original sequence preserved. The temp file is `fsync`'d, then
//!    atomically `rename(2)`'d over the live log, then the parent directory is
//!    `fsync`'d so the rename itself is durable.
//! 3. Startup ([`Journal::open`]) loads the snapshot record's sequence as the
//!    replay seed, then replays only the residual tail. [`Journal::replay`] yields
//!    the snapshot bytes (as the first [`Record`], distinguished by
//!    [`Record::kind`] `== `[`RecordKind::Snapshot`]) followed by the residual data
//!    records, so a consumer rebuilds `snapshot ⊕ tail` instead of the whole
//!    history.
//!
//! ## Crash-safety of compaction
//!
//! Compaction is **atomic with respect to a crash** by construction. The only
//! durable mutation of the live path is the `rename(2)`, which POSIX guarantees is
//! atomic: an observer (including post-crash recovery) sees the live path bound to
//! *either* the old inode (the complete pre-compaction log) *or* the new inode
//! (the complete compacted log), **never** a partially-written file. Concretely:
//!
//! * A crash **before** the rename leaves the live log untouched and only an
//!   orphan temp file behind (removed at the start of the next compaction);
//!   recovery reads the complete old log.
//! * A crash **during** the temp write (the temp file is half-written) is the same
//!   case — the live log is still the complete old log; the half-written temp is
//!   never named into place.
//! * A crash **after** the rename returns reads the complete new (compacted) log.
//! * Because the temp file is `fsync`'d *before* the rename and the parent
//!   directory is `fsync`'d *after*, the new log's bytes and the rename are both on
//!   stable storage once `compact` returns `Ok`.
//!
//! There is no window in which the live path names a torn file: the journal's own
//! torn-tail healing ([`Journal::open`]) is a backstop for an interrupted
//! *append*, never required for an interrupted *compaction*.
//!
//! ## Sequence monotonicity across compactions
//!
//! Sequence numbers stay **globally monotonic** across compactions. Residual
//! records keep their original (post-watermark) sequence numbers, so they remain
//! strictly increasing; the snapshot record carries the watermark sequence and
//! seeds the post-snapshot replay expectation to `watermark + 1`. A subsequent
//! [`Journal::append`] continues from the recovered `next_sequence` — the maximum
//! of (last residual sequence + 1) and (watermark + 1) — so no sequence is ever
//! reused or rewound by a compaction.

#![forbid(unsafe_code)]

mod crc32;
pub mod async_journal;

pub use async_journal::{AppendReceipt, AsyncJournal, CxlPmemJournal, DurabilityPolicy};
pub use crc32::crc32;

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Per-record sync word at the head of every frame (data and snapshot). A fixed,
/// non-zero magic so recovery can tell an **interior** CRC failure (intact sync
/// word, failing CRC ⇒ [`JournalError::CorruptInterior`]) apart from a torn tail
/// (absent/short sync word at EOF, or a sync word that does not match ⇒ heal). It
/// is **inside** the CRC coverage, so a flipped sync byte is still caught. The
/// bytes are ASCII `"CLNJRNL\0"` interpreted little-endian — human-recognizable in
/// a hex dump and, as the start-of-frame magic, the resync point the old marker-
/// less format lacked.
const SYNC_WORD: u64 = u64::from_le_bytes(*b"CLNJRNL\0");
/// Width of the sync word prefix, in bytes.
const SYNC_LEN: usize = 8;

/// Size of a record's fixed header: `payload_len: u32` + `sequence: u64`.
const HEADER_LEN: usize = 4 + 8;
/// Size of the trailing CRC-32: `u32`.
const CRC_LEN: usize = 4;

/// A defensive upper bound on a single payload, guarding recovery against a
/// corrupt length field that would otherwise demand a wild allocation.
///
/// 64 MiB is far above any real journalled control-plane event (an accepted
/// market-state delta or a trade lifecycle record); a `payload_len` larger than
/// this in a record header is itself treated as corruption (torn tail).
const MAX_PAYLOAD_LEN: u32 = 64 * 1024 * 1024;

/// Sentinel value in a record's `payload_len` field marking it as a **snapshot**
/// record (written only by [`Journal::compact`]). A data record's `payload_len`
/// is bounded by [`MAX_PAYLOAD_LEN`] (64 MiB) ≪ `u32::MAX`, so this value can
/// never collide with a real data record — keeping the data-record layout
/// byte-for-byte unchanged while distinguishing the snapshot losslessly.
const SNAPSHOT_MARKER: u32 = u32::MAX;

/// Errors surfaced by the journal.
///
/// Note that a **torn or corrupt final record is *not* an error**: it is the
/// expected post-crash state and is handled transparently by truncation during
/// [`Journal::open`]. Errors here are genuine faults (IO failure, a corrupt
/// record that is *not* the tail, payload too large to be appended).
#[derive(Debug)]
pub enum JournalError {
    /// An underlying filesystem IO error.
    Io(io::Error),
    /// A record carried an interior inconsistency that truncation cannot explain,
    /// so it is surfaced rather than silently healed. Carries the expected sequence
    /// number at the bad record. Raised in three cases:
    ///
    /// * **interior CRC failure** — an intact [`SYNC_WORD`] followed by a *complete*
    ///   frame body (all bytes through the CRC trailer present) whose CRC fails:
    ///   bit-rot of a fully-written record, not a torn tail (a torn tail leaves the
    ///   body short, healed silently). This is the discrimination the per-record
    ///   sync word makes possible;
    /// * a CRC-**valid** data record whose sequence is non-monotonic (a
    ///   reordered/duplicated frame);
    /// * a CRC-valid snapshot record placed anywhere but the start of the log.
    ///
    /// A short body at EOF (an interrupted append) is *not* this error — it is a
    /// torn tail, healed by truncation (see [`Journal::open`] and the failure model).
    CorruptInterior {
        /// The expected (monotonic) sequence number at the failing record.
        at_sequence: u64,
        /// Human-readable reason for the surfaced interior inconsistency.
        reason: &'static str,
    },
    /// An `append` payload (or a `compact` snapshot) exceeded [`MAX_PAYLOAD_LEN`].
    PayloadTooLarge {
        /// The rejected payload length in bytes.
        len: usize,
    },
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JournalError::Io(e) => write!(f, "journal io error: {e}"),
            JournalError::CorruptInterior {
                at_sequence,
                reason,
            } => write!(
                f,
                "journal interior corruption at sequence {at_sequence}: {reason}"
            ),
            JournalError::PayloadTooLarge { len } => {
                write!(
                    f,
                    "journal payload too large: {len} bytes (max {MAX_PAYLOAD_LEN})"
                )
            }
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            JournalError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for JournalError {
    fn from(e: io::Error) -> Self {
        JournalError::Io(e)
    }
}

/// Result alias for journal operations.
pub type Result<T> = std::result::Result<T, JournalError>;

/// Whether a recovered [`Record`] is a normal data event or the leading
/// snapshot record written by [`Journal::compact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    /// A normal data record — an appended domain event.
    Data,
    /// The leading snapshot/checkpoint record of a compacted log. Its `payload`
    /// is the consumer's opaque snapshot bytes; its `sequence` is the checkpoint
    /// watermark (the highest data sequence the snapshot covers).
    Snapshot,
}

/// One valid, recovered record: its kind, monotonic sequence and opaque payload.
///
/// The journal is payload-agnostic — it stores and returns raw bytes. A typed
/// consumer pairs this with an [`EventCodec`] to decode the bytes back into a
/// domain event for state rebuild. A [`RecordKind::Snapshot`] record (present iff
/// the log has been compacted) appears first; its payload is the snapshot image,
/// not a data event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Whether this is a data record or the leading compaction snapshot.
    pub kind: RecordKind,
    /// For a data record, the strictly-monotonic sequence assigned at append
    /// time; for a snapshot record, the checkpoint watermark it covers.
    pub sequence: u64,
    /// Opaque, codec-defined payload bytes (snapshot image for a snapshot record).
    pub payload: Vec<u8>,
}

/// The typed-event seam: encode a domain event to opaque bytes for the journal,
/// and decode recovered bytes back to the event for state rebuild.
///
/// The journal stores only `&[u8]`; this trait keeps the *meaning* of the bytes
/// in the consumer (the engine's control plane), so the durable substrate stays
/// generic and the wire/event contract evolves without touching the journal
/// (guardrail: one current contract, no versioned APIs in the journal itself).
///
/// Encoding **must be deterministic** — the same event encodes to identical
/// bytes every time — so that replay reconstructs state bit-identically.
pub trait EventCodec {
    /// The in-memory domain event type.
    type Event;
    /// Decode failure type.
    type Error;

    /// Serialize `event` to its durable byte form. Must be deterministic.
    fn encode(event: &Self::Event) -> Vec<u8>;

    /// Deserialize a recovered payload back into a domain event.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if `payload` is not a valid encoding (e.g. the
    /// codec contract changed underneath an old file).
    fn decode(payload: &[u8]) -> std::result::Result<Self::Event, Self::Error>;
}

/// A durable, append-only, sequence-ordered event journal backed by a single
/// file. Open/create with [`Journal::open`]; append with [`Journal::append`];
/// rebuild with [`Journal::replay`]; bound replay time with [`Journal::compact`].
#[derive(Debug)]
pub struct Journal {
    /// The append handle, always positioned at the logical end (last good byte).
    file: File,
    /// Path, retained for `replay` (which opens an independent read handle) and
    /// for diagnostics.
    path: PathBuf,
    /// The sequence number assigned to the next appended record. Equals
    /// `last_sequence + 1`, or `0` for a fresh/empty log.
    next_sequence: u64,
    /// Byte offset of the logical end of valid data (where the next append goes).
    end_offset: u64,
}

impl Journal {
    /// Open the journal at `path`, creating it if absent.
    ///
    /// On open the file is scanned front-to-back: each record's [`SYNC_WORD`], CRC
    /// and the strict sequence monotonicity (`0, 1, 2, …`, or `watermark+1, …` after
    /// a leading snapshot) are validated. A **torn tail** (a crash mid-`append`) —
    /// a short read of the final frame, or trailing bytes not led by the sync word —
    /// is healed: the file is truncated to the end of the last good record and the
    /// caller sees a clean, consistent log (no partial record, no error).
    ///
    /// An **interior CRC failure** (an intact sync word + a complete frame body
    /// whose CRC fails), a *sequence break* on a CRC-valid data record, or a
    /// snapshot record found after the start, are each surfaced as
    /// [`JournalError::CorruptInterior`] rather than hidden — a torn tail cannot
    /// produce any of these (it leaves the body short or the sync word absent).
    ///
    /// # Errors
    ///
    /// - [`JournalError::Io`] on filesystem failure.
    /// - [`JournalError::CorruptInterior`] on a detectable interior inconsistency
    ///   (interior CRC failure under an intact sync word, non-monotonic sequence, or
    ///   a misplaced snapshot record).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        let scan = Self::scan(&file)?;

        // If the scan found a torn/garbage tail past the last good record,
        // truncate it away so the file holds only fully-durable records.
        let file_len = file.metadata()?.len();
        if scan.good_end_offset < file_len {
            file.set_len(scan.good_end_offset)?;
            // Persist the truncation itself.
            file.sync_data()?;
        }

        // Make the file's *directory entry* durable. `append`'s per-record
        // `sync_data` flushes record bytes (and the size metadata needed to read
        // them back), but on a crash immediately after the file was first created
        // the directory entry linking the inode can still be lost on some
        // filesystems — so the very first record's durability contract is not met
        // by a data-only fsync. Syncing the parent directory once on open (covering
        // creation and any torn-tail truncation) closes that hole; subsequent
        // appends then need only `sync_data`, since the entry is already persisted.
        sync_parent_dir(&path)?;

        // Position the append cursor at the logical end.
        file.seek(SeekFrom::Start(scan.good_end_offset))?;

        Ok(Journal {
            file,
            path,
            next_sequence: scan.next_sequence,
            end_offset: scan.good_end_offset,
        })
    }

    /// Append `payload` as a new record, returning its assigned sequence number.
    ///
    /// # Durability
    ///
    /// This call returns `Ok(seq)` only after the record's bytes have been
    /// written and `fsync`'d (`File::sync_data`). Once it returns, the record
    /// survives a crash. This is the journal's durability point.
    ///
    /// # Errors
    ///
    /// - [`JournalError::PayloadTooLarge`] if `payload` exceeds the recovery
    ///   safety bound ([`MAX_PAYLOAD_LEN`]).
    /// - [`JournalError::Io`] on a write/flush failure. On a partial write the
    ///   on-disk frame is left for the next [`Journal::open`] to detect and
    ///   truncate as a torn tail; in-memory state (`next_sequence`, cursor) is
    ///   left untouched so the journal handle remains consistent.
    pub fn append(&mut self, payload: &[u8]) -> Result<u64> {
        let len_u32 = u32::try_from(payload.len())
            .ok()
            .filter(|&l| l <= MAX_PAYLOAD_LEN)
            .ok_or(JournalError::PayloadTooLarge { len: payload.len() })?;

        let seq = self.next_sequence;

        // Build the framed record in a single buffer, then issue one write so a
        // crash produces either nothing or a prefix (detected as a torn tail) —
        // never an interleaved frame. CRC covers sync word + header + payload.
        let frame = frame_record(seq, len_u32, payload);

        // Ensure we write at the logical end (defensive against an external seek).
        self.file.seek(SeekFrom::Start(self.end_offset))?;
        self.file.write_all(&frame)?;
        // Durability point: flush data to stable storage before acknowledging.
        self.file.sync_data()?;

        // Commit in-memory state only after the durable write succeeds.
        self.end_offset += frame.len() as u64;
        self.next_sequence += 1;
        Ok(seq)
    }

    /// Replay every valid record in order, invoking `f` for each.
    ///
    /// Opens an independent read handle, so replay does not disturb the append
    /// cursor and may be called at startup before any appends. The journal has
    /// already been healed on [`Journal::open`], so replay sees only good
    /// records; it re-validates CRC and sequence defensively and re-reports
    /// interior corruption should the file have changed underneath.
    ///
    /// On a compacted log the first [`Record`] yielded is the
    /// [`RecordKind::Snapshot`] checkpoint (so the consumer restores it), followed
    /// by the residual data records.
    ///
    /// # Errors
    ///
    /// - [`JournalError::Io`] on read failure.
    /// - [`JournalError::CorruptInterior`] if a non-final record is corrupt.
    pub fn replay<F>(&self, mut f: F) -> Result<u64>
    where
        F: FnMut(Record),
    {
        let read_file = File::open(&self.path)?;
        let mut reader = BufReader::new(read_file);
        let mut expected_seq = 0u64;
        let mut count = 0u64;
        let mut at_start = true;

        // Stops on `Eof`/`TornTail` (the log already ends there); an interior
        // inconsistency propagates as `CorruptInterior` via `?`. A leading snapshot
        // record is yielded to `f` (so the consumer rebuilds from it) and reseeds
        // the expected data sequence to watermark+1.
        while let ReadOutcome::Record(rec) = read_one(&mut reader, expected_seq, at_start)? {
            expected_seq = rec.sequence + 1;
            at_start = false;
            count += 1;
            f(rec);
        }
        Ok(count)
    }

    /// Collect every valid record into a `Vec`, in sequence order.
    ///
    /// Convenience wrapper over [`Journal::replay`] for consumers that prefer a
    /// materialized list to a callback.
    ///
    /// # Errors
    ///
    /// As [`Journal::replay`].
    pub fn records(&self) -> Result<Vec<Record>> {
        let mut out = Vec::new();
        self.replay(|r| out.push(r))?;
        Ok(out)
    }

    /// The sequence number of the most recently appended (durable) record, or
    /// `None` if the log is empty.
    #[must_use]
    pub fn last_sequence(&self) -> Option<u64> {
        self.next_sequence.checked_sub(1)
    }

    /// The sequence number that the next [`Journal::append`] will assign.
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// The byte length of the valid (durable) region of the log file.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.end_offset
    }

    /// Compact the log against a checkpoint, replacing it (atomically) with a
    /// leading **snapshot record** followed by only the **residual tail** —
    /// every data record whose sequence is strictly greater than `watermark`.
    ///
    /// `snapshot` is the consumer's opaque image of the state covered by all data
    /// records up to and including `watermark` (e.g. a serialized book/marked-
    /// surface). After compaction, [`Journal::replay`] yields the snapshot record
    /// first (so the consumer restores it), then the residual data records, so a
    /// rebuild is `snapshot ⊕ tail` instead of replaying the whole history.
    ///
    /// # Atomicity & crash-safety
    ///
    /// The new log is written to a sibling temp file, `fsync`'d, then atomically
    /// `rename(2)`'d over the live path, after which the parent directory is
    /// `fsync`'d. A crash at any point leaves the live path bound to **either** the
    /// complete pre-compaction log **or** the complete compacted log — never a torn
    /// file. See the crate-level "Crash-safety of compaction".
    ///
    /// # Sequence monotonicity
    ///
    /// Residual records keep their original sequence numbers and the snapshot
    /// record carries `watermark`, so sequences remain globally monotonic across
    /// the compaction; a subsequent [`Journal::append`] continues from the
    /// (unchanged-or-advanced) `next_sequence`. The watermark may equal the current
    /// [`Journal::last_sequence`] (compact everything, empty residual tail) or sit
    /// below it (keep a tail); a watermark **above** the last durable sequence is
    /// rejected, since the snapshot would claim coverage of records that do not
    /// exist.
    ///
    /// # Errors
    ///
    /// - [`JournalError::PayloadTooLarge`] if `snapshot` exceeds [`MAX_PAYLOAD_LEN`].
    /// - [`JournalError::CorruptInterior`] (reason names the watermark) if
    ///   `watermark` exceeds the highest durable sequence (incl. an empty log).
    /// - [`JournalError::Io`] on a filesystem failure (the live log is left intact —
    ///   the failure is before or during the temp write, never a half-rename).
    pub fn compact(&mut self, watermark: u64, snapshot: &[u8]) -> Result<()> {
        let snapshot_len = u32::try_from(snapshot.len())
            .ok()
            .filter(|&l| l <= MAX_PAYLOAD_LEN)
            .ok_or(JournalError::PayloadTooLarge {
                len: snapshot.len(),
            })?;

        // The watermark cannot claim coverage past what is durably in the log.
        // `last_sequence()` is None for an empty log; any watermark then over-claims.
        match self.last_sequence() {
            Some(last) if watermark <= last => {}
            _ => {
                return Err(JournalError::CorruptInterior {
                    at_sequence: watermark,
                    reason: "compaction watermark exceeds the highest durable sequence",
                });
            }
        }

        // 1) Materialize the residual records (sequence > watermark), byte-copied
        //    with their original sequences preserved, in order.
        let residual: Vec<Record> = {
            let mut out = Vec::new();
            self.replay(|rec| {
                // Replaying a not-yet-compacted log yields only `Data` records; be
                // defensive and keep only post-watermark data records regardless.
                if rec.kind == RecordKind::Data && rec.sequence > watermark {
                    out.push(rec);
                }
            })?;
            out
        };

        // 2) Build the fresh log image: snapshot record, then residual data records.
        let mut fresh = Vec::new();
        fresh.extend_from_slice(&frame_snapshot(watermark, snapshot_len, snapshot));
        for rec in &residual {
            let len_u32 = u32::try_from(rec.payload.len())
                .expect("residual payload length already validated on append");
            fresh.extend_from_slice(&frame_record(rec.sequence, len_u32, &rec.payload));
        }

        // 3) Write the fresh image to a sibling temp file and fsync it.
        let tmp = compaction_tmp_path(&self.path);
        // Remove any stale temp from a previously-interrupted compaction so we
        // never read its bytes; the only durable mutation remains the rename.
        let _ = std::fs::remove_file(&tmp);
        {
            let mut tmp_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            tmp_file.write_all(&fresh)?;
            tmp_file.sync_data()?;
        }

        // 4) Atomically rename the temp over the live path (POSIX-atomic), then
        //    make the rename itself durable by fsync'ing the parent directory.
        std::fs::rename(&tmp, &self.path)?;
        sync_parent_dir(&self.path)?;

        // 5) Re-open the now-compacted log to refresh the cursor/sequence state.
        //    `open` re-scans, seeds from the snapshot, and positions the append
        //    cursor at the new logical end — keeping `next_sequence` monotonic.
        let reopened = Journal::open(&self.path)?;
        self.file = reopened.file;
        self.next_sequence = reopened.next_sequence;
        self.end_offset = reopened.end_offset;
        Ok(())
    }

    /// Scan a file from the start, validating records, and report where the last
    /// good record ends and what the next sequence number is.
    ///
    /// A leading snapshot record (from a prior compaction) reseeds the expected
    /// data sequence to `watermark + 1`; data records then continue strictly
    /// monotonically from there. `next_sequence` is the expected sequence past the
    /// last good record, which never rewinds below `watermark + 1` even when the
    /// residual tail is empty.
    fn scan(file: &File) -> Result<Scan> {
        let read_file = file.try_clone()?;
        let mut reader = BufReader::new(read_file);
        reader.seek(SeekFrom::Start(0))?;

        let mut good_end_offset = 0u64;
        let mut expected_seq = 0u64;
        let mut at_start = true;

        while let ReadOutcome::Record(rec) = read_one(&mut reader, expected_seq, at_start)? {
            // Snapshot (watermark) seeds to watermark+1; data advances by one.
            expected_seq = rec.sequence + 1;
            at_start = false;
            good_end_offset = reader.stream_position()?;
        }

        Ok(Scan {
            good_end_offset,
            next_sequence: expected_seq,
        })
    }
}

/// Result of scanning a log file on open.
struct Scan {
    /// Byte offset just past the last fully-valid record.
    good_end_offset: u64,
    /// Sequence number the next append should use.
    next_sequence: u64,
}

/// Outcome of attempting to read one record from the current reader position.
enum ReadOutcome {
    /// A fully-valid record.
    Record(Record),
    /// Clean end of file at a record boundary.
    Eof,
    /// A truncated **final** record, or a start that does not carry the sync word
    /// (torn tail) — stop here and heal by truncation.
    TornTail,
}

/// Read and validate a single record at the reader's current position.
///
/// `expected_seq` is the sequence number a **data** record must carry for strict
/// monotonicity. A leading **snapshot** record is exempt from that check —
/// instead its own sequence (the checkpoint watermark) reseeds the caller's
/// expectation. `at_start` is true only for the very first record of the file.
/// Returns:
/// - `Record` on success (and leaves the reader at the next record),
/// - `Eof` if the reader is exactly at end-of-file (clean boundary),
/// - `TornTail` if the frame start is not the sync word, or the frame is truncated
///   (a short read of sync word/header/payload/CRC — crash mid-append),
/// - `Err(CorruptInterior)` if the frame is fully present under an intact sync word
///   but its CRC fails (interior bit-rot), a CRC-valid data record's sequence breaks
///   monotonicity, or a snapshot record appears anywhere but first.
fn read_one<R: Read>(reader: &mut R, expected_seq: u64, at_start: bool) -> Result<ReadOutcome> {
    // 1. Sync word: distinguishes the start of a framed record from a torn/EOF
    //    boundary. Zero bytes available is a clean EOF; a short read (1..SYNC_LEN)
    //    is a torn final frame whose header never fully flushed.
    let mut sync = [0u8; SYNC_LEN];
    match read_full_or_short(reader, &mut sync)? {
        FillState::Empty => return Ok(ReadOutcome::Eof),
        FillState::Short => return Ok(ReadOutcome::TornTail),
        FillState::Full => {}
    }
    if u64::from_le_bytes(sync) != SYNC_WORD {
        // No framed record begins here. At the physical tail this is a torn final
        // frame (or trailing garbage from an interrupted append) — heal it.
        return Ok(ReadOutcome::TornTail);
    }

    // 2. Logical header (payload_len + sequence). A short read is a torn tail.
    let mut header = [0u8; HEADER_LEN];
    match read_full_or_short(reader, &mut header)? {
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
        FillState::Full => {}
    }

    let len_field = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let sequence = u64::from_le_bytes([
        header[4], header[5], header[6], header[7], header[8], header[9], header[10], header[11],
    ]);

    // A snapshot record is flagged by the SNAPSHOT_MARKER sentinel in the length
    // field — handled separately (it carries an extra inner length prefix).
    if len_field == SNAPSHOT_MARKER {
        return read_snapshot(reader, &sync, &header, sequence, expected_seq, at_start);
    }

    // A length beyond the safety bound (but not the sentinel) can only be
    // corruption in the (final) length field; treat as a torn tail rather than
    // attempting a wild read.
    if len_field > MAX_PAYLOAD_LEN {
        return Ok(ReadOutcome::TornTail);
    }
    let payload_len = len_field;

    // Read the payload; a short read is a torn tail.
    let mut payload = vec![0u8; payload_len as usize];
    match read_full_or_short(reader, &mut payload)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }

    // Read the trailing CRC; a short read is a torn tail.
    let mut crc_bytes = [0u8; CRC_LEN];
    match read_full_or_short(reader, &mut crc_bytes)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let stored_crc = u32::from_le_bytes(crc_bytes);

    // Recompute the CRC over sync word + header + payload and compare.
    let mut framed = Vec::with_capacity(SYNC_LEN + HEADER_LEN + payload.len());
    framed.extend_from_slice(&sync);
    framed.extend_from_slice(&header);
    framed.extend_from_slice(&payload);
    let actual_crc = crc32(&framed);

    if actual_crc != stored_crc {
        // INTERIOR-vs-TORN discrimination. An intact sync word means a real,
        // fully-framed record started here, and the full body (payload + CRC
        // trailer) is present — a torn tail would have left the body *short* (the
        // CRC trailer absent/partial, caught above as `Short`) or the next sync
        // word absent. So an intact sync word with a complete body but a failing
        // CRC is **interior corruption** (bit-rot of a fully-written record), not a
        // torn tail. Surface it rather than silently truncating the records after.
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "record CRC failed with an intact sync word (interior corruption)",
        });
    }

    // CRC is valid ⇒ the record is intact. A data record's sequence must be
    // strictly monotonic; a valid record with the wrong sequence is genuine
    // interior corruption.
    if sequence != expected_seq {
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "sequence number is not strictly monotonic",
        });
    }

    Ok(ReadOutcome::Record(Record {
        kind: RecordKind::Data,
        sequence,
        payload,
    }))
}

/// Read and validate a snapshot record whose header was already consumed (its
/// `payload_len` field held [`SNAPSHOT_MARKER`]). The body is an inner `u32`
/// length prefix, the snapshot bytes, then the CRC over the whole frame.
///
/// A snapshot record is legal **only** as the very first record of a compacted
/// log; a CRC-valid snapshot record found anywhere else is interior corruption
/// truncation cannot explain, so it is surfaced rather than healed. A snapshot
/// record with an intact sync word and a complete body but a failing CRC is
/// likewise interior corruption; a truncated (short-body) snapshot record is a
/// torn tail like any other.
fn read_snapshot<R: Read>(
    reader: &mut R,
    sync: &[u8; SYNC_LEN],
    header: &[u8; HEADER_LEN],
    watermark: u64,
    expected_seq: u64,
    at_start: bool,
) -> Result<ReadOutcome> {
    // Inner length prefix.
    let mut inner_len_bytes = [0u8; 4];
    match read_full_or_short(reader, &mut inner_len_bytes)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let snap_len = u32::from_le_bytes(inner_len_bytes);
    if snap_len > MAX_PAYLOAD_LEN {
        return Ok(ReadOutcome::TornTail);
    }

    // Snapshot bytes.
    let mut snapshot = vec![0u8; snap_len as usize];
    match read_full_or_short(reader, &mut snapshot)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }

    // Trailing CRC.
    let mut crc_bytes = [0u8; CRC_LEN];
    match read_full_or_short(reader, &mut crc_bytes)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let stored_crc = u32::from_le_bytes(crc_bytes);

    // CRC covers sync word + header (incl. the sentinel + watermark) + inner length
    // + snapshot bytes.
    let mut framed = Vec::with_capacity(SYNC_LEN + HEADER_LEN + 4 + snapshot.len());
    framed.extend_from_slice(sync);
    framed.extend_from_slice(header);
    framed.extend_from_slice(&inner_len_bytes);
    framed.extend_from_slice(&snapshot);
    if crc32(&framed) != stored_crc {
        // As in `read_one`: an intact sync word plus a complete body (snapshot
        // bytes + CRC trailer present) with a failing CRC is interior corruption,
        // not a torn tail (a torn tail leaves the body short, caught above).
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "snapshot CRC failed with an intact sync word (interior corruption)",
        });
    }

    // CRC valid ⇒ intact. A snapshot record is only legal at the start of the log.
    if !at_start {
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "snapshot record appears after the start of the log",
        });
    }

    Ok(ReadOutcome::Record(Record {
        kind: RecordKind::Snapshot,
        sequence: watermark,
        payload: snapshot,
    }))
}

/// How much of a target buffer a read managed to fill.
#[derive(Debug)]
enum FillState {
    /// Zero bytes read at the very first attempt (clean EOF boundary).
    Empty,
    /// Some but not all bytes read (truncated frame).
    Short,
    /// Buffer fully filled.
    Full,
}

/// Frame one **data** record into a single contiguous buffer in the canonical
/// on-disk layout (`SYNC_WORD || payload_len || sequence || payload || crc32`).
///
/// The CRC-32 covers the sync word, the header and the payload, so any flipped
/// byte — sync word, length, sequence, or payload — is detected on read. Used by
/// both [`Journal::append`] and [`Journal::compact`] (for the byte-copied residual
/// data records), so the two paths cannot drift in framing.
fn frame_record(sequence: u64, payload_len: u32, payload: &[u8]) -> Vec<u8> {
    debug_assert!(
        payload_len <= MAX_PAYLOAD_LEN,
        "data payload over the bound"
    );
    let mut frame = Vec::with_capacity(SYNC_LEN + HEADER_LEN + payload.len() + CRC_LEN);
    frame.extend_from_slice(&SYNC_WORD.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&sequence.to_le_bytes());
    frame.extend_from_slice(payload);
    let checksum = crc32(&frame);
    frame.extend_from_slice(&checksum.to_le_bytes());
    frame
}

/// Frame the leading **snapshot** record of a compacted log: the
/// [`SNAPSHOT_MARKER`] sentinel in the `payload_len` field, the checkpoint
/// `watermark` in the `sequence` field, then the true snapshot length (`u32`)
/// followed by the snapshot bytes, then the CRC over all of it. Distinguished
/// from a data record purely by the sentinel — the data-record layout is
/// unchanged.
fn frame_snapshot(watermark: u64, snap_len: u32, snapshot: &[u8]) -> Vec<u8> {
    debug_assert_eq!(snap_len as usize, snapshot.len());
    debug_assert!(snap_len <= MAX_PAYLOAD_LEN, "snapshot over the bound");
    let mut frame = Vec::with_capacity(SYNC_LEN + HEADER_LEN + 4 + snapshot.len() + CRC_LEN);
    frame.extend_from_slice(&SYNC_WORD.to_le_bytes());
    frame.extend_from_slice(&SNAPSHOT_MARKER.to_le_bytes());
    frame.extend_from_slice(&watermark.to_le_bytes());
    frame.extend_from_slice(&snap_len.to_le_bytes());
    frame.extend_from_slice(snapshot);
    let checksum = crc32(&frame);
    frame.extend_from_slice(&checksum.to_le_bytes());
    frame
}

/// The sibling temp path used for the atomic compaction rewrite.
fn compaction_tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".compact");
    PathBuf::from(s)
}

/// Fsync the directory containing `path` so a freshly-created (or just-truncated /
/// just-renamed) journal file's directory entry is itself durable.
///
/// Directory fsync is a POSIX concept (a directory is a file whose contents are
/// its entries); on Unix we open the parent directory read-only and `sync_all`.
/// A bare relative filename has an empty parent, which denotes the current
/// directory (`"."`).
#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    File::open(&dir)?.sync_all()
}

/// On non-Unix targets directory fsync is not the durability mechanism (file
/// creation is ordered by the platform's own semantics), so this is a no-op.
/// Celnet's runtime targets are macOS and Linux (both Unix).
#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Read exactly `buf.len()` bytes, tolerating short reads at EOF.
///
/// Distinguishes a clean boundary (0 bytes available) from a torn frame (a
/// partial fill), which the caller maps to `Eof` vs. `TornTail`.
fn read_full_or_short<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<FillState> {
    // A zero-length request is trivially, fully satisfied without touching the
    // reader — this is the legitimate empty-payload case, not an EOF boundary.
    if buf.is_empty() {
        return Ok(FillState::Full);
    }
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(if filled == 0 {
        FillState::Empty
    } else if filled < buf.len() {
        FillState::Short
    } else {
        FillState::Full
    })
}

#[cfg(test)]
mod tests;
