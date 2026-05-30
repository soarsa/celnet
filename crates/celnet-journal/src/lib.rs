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
//! The log is a flat file of back-to-back records. Each record is:
//!
//! ```text
//! ┌──────────────┬──────────────┬─────────────────┬──────────────┐
//! │ payload_len  │  sequence    │   payload       │   crc32      │
//! │  u32 LE      │  u64 LE      │  payload_len B  │  u32 LE      │
//! └──────────────┴──────────────┴─────────────────┴──────────────┘
//! ```
//!
//! The CRC-32 (IEEE, see [`crc32`]) covers the framed header **and** the payload
//! (`payload_len || sequence || payload`), so a flipped byte anywhere in the
//! record is detected. A record whose bytes are present but whose CRC fails, or
//! whose frame is truncated (a short read at EOF), is treated as a torn tail:
//! recovery stops at the last good record and truncates the file to that offset.
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
//! # Failure model & a stated limitation
//!
//! This format is **marker-less** (no resync/sync-word between records), chosen
//! for simplicity and minimal per-record overhead. The consequence, stated
//! honestly rather than papered over:
//!
//! * A **torn tail** (truncated final frame, or a CRC failure on the last record)
//!   is the expected post-crash state — recovery stops at the last good record and
//!   truncates, returning no error. This is correct: the torn bytes were never
//!   acknowledged (`append` had not returned `Ok`).
//! * A **CRC failure on an *interior* record** (e.g. silent bit-rot under an
//!   otherwise-intact tail) is **indistinguishable** from a torn tail without a
//!   resync marker, so it is handled the same way: recovery stops there and the
//!   trailing — possibly already-acknowledged — records are truncated. `open`
//!   returns `Ok`, **not** an error. This is a genuine limitation: a single rotted
//!   interior byte can silently drop committed records after it. Mitigations are
//!   (a) the checkpoint/compaction cycle below, which bounds the residual tail and
//!   thus the exposure window, and (b) external integrity scrubbing for cold logs.
//! * The one interior inconsistency the format *can* cheaply detect — a
//!   CRC-**valid** record whose sequence number is non-monotonic — **is** surfaced
//!   as [`JournalError::CorruptInterior`] rather than healed.
//!
//! (A future framed format with a per-record resync marker would let recovery
//! distinguish interior CRC corruption from a torn tail and surface it; that is a
//! deliberate non-goal of this first cut, recorded here, not silently assumed.)
//!
//! # Checkpointing & compaction (design note — not yet implemented)
//!
//! An unbounded append-only log replays in time linear in its length. The
//! intended bound is a **checkpoint + compaction** cycle, designed here and
//! honestly *not* implemented in this crate yet:
//!
//! 1. A consumer periodically snapshots its rebuilt state and records the
//!    highest sequence the snapshot covers (a *checkpoint watermark*).
//! 2. Compaction then writes a fresh log containing only records *after* the
//!    watermark (plus, optionally, a leading snapshot record), `fsync`s it, and
//!    atomically renames it over the old log (`rename(2)` is atomic on POSIX),
//!    so a crash mid-compaction leaves either the old or the new complete log —
//!    never a half-state.
//! 3. Startup then loads the snapshot, replays only the residual tail, and
//!    continues. Sequence numbers stay globally monotonic across compactions by
//!    seeding the compacted log's first sequence from the watermark.
//!
//! This keeps the durable format and the present API forward-compatible with
//! compaction without committing a half-built mechanism now (guardrail: no
//! placeholders — the unbuilt part is documented, not faked).

#![forbid(unsafe_code)]

mod crc32;

pub use crc32::crc32;

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

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
    /// A CRC-**valid** record carried a non-monotonic sequence number — a
    /// structurally detectable interior inconsistency (a reordered/duplicated
    /// frame) that truncation cannot explain, so it is surfaced rather than
    /// silently healed. Carries the expected sequence number at the bad record.
    ///
    /// Note the scope precisely: this is **not** raised for a CRC *failure*. A
    /// failed CRC is indistinguishable, in a marker-less format, from a torn tail,
    /// so it is handled as such (recovery stops and truncates — see
    /// [`Journal::open`] and the failure-model note there). This variant covers
    /// only the case the format *can* detect: intact bytes, wrong sequence.
    CorruptInterior {
        /// The expected (monotonic) sequence number at the failing record.
        at_sequence: u64,
        /// Human-readable reason (currently always a sequence-monotonicity break).
        reason: &'static str,
    },
    /// An `append` payload exceeded [`MAX_PAYLOAD_LEN`].
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

/// One valid, recovered record: its monotonic sequence and its opaque payload.
///
/// The journal is payload-agnostic — it stores and returns raw bytes. A typed
/// consumer pairs this with an [`EventCodec`] to decode the bytes back into a
/// domain event for state rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Strictly monotonic sequence number assigned at append time.
    pub sequence: u64,
    /// Opaque, codec-defined payload bytes.
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
/// rebuild with [`Journal::replay`].
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
    /// On open the file is scanned front-to-back: each record's CRC and the
    /// strict sequence monotonicity (`0, 1, 2, …`) are validated. A truncated or
    /// CRC-failing **final** record (a crash mid-`append`) is treated as a torn
    /// tail — the file is truncated to the end of the last good record and the
    /// caller sees a clean, consistent log (no partial record, no error).
    ///
    /// Recovery stops at the **first** record that fails its CRC and truncates
    /// from there: a CRC failure means the bytes from that record onward are not
    /// trustworthy, so the recovered prefix is exactly the contiguous run of
    /// good records. A *sequence break* on an otherwise CRC-valid record (a
    /// reordered/duplicated frame — corruption that truncation cannot explain)
    /// is surfaced as [`JournalError::CorruptInterior`] rather than hidden.
    ///
    /// # Errors
    ///
    /// - [`JournalError::Io`] on filesystem failure.
    /// - [`JournalError::CorruptInterior`] if a CRC-valid record carries a
    ///   non-monotonic sequence number.
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
        // never an interleaved frame. CRC covers header + payload.
        let mut frame = Vec::with_capacity(HEADER_LEN + payload.len() + CRC_LEN);
        frame.extend_from_slice(&len_u32.to_le_bytes());
        frame.extend_from_slice(&seq.to_le_bytes());
        frame.extend_from_slice(payload);
        let checksum = crc32(&frame);
        frame.extend_from_slice(&checksum.to_le_bytes());

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

        // Stops on `Eof`/`TornTail` (the log already ends there); a sequence
        // break inside the file propagates as `CorruptInterior` via `?`.
        while let ReadOutcome::Record(rec) = read_one(&mut reader, expected_seq)? {
            expected_seq = rec.sequence + 1;
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

    /// Scan a file from the start, validating records, and report where the last
    /// good record ends and what the next sequence number is.
    fn scan(file: &File) -> Result<Scan> {
        let read_file = file.try_clone()?;
        let mut reader = BufReader::new(read_file);
        reader.seek(SeekFrom::Start(0))?;

        let mut good_end_offset = 0u64;
        let mut expected_seq = 0u64;

        while let ReadOutcome::Record(rec) = read_one(&mut reader, expected_seq)? {
            expected_seq = rec.sequence + 1;
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
    /// A truncated or CRC-failing **final** record (torn tail) — stop here.
    TornTail,
}

/// Read and validate a single record at the reader's current position.
///
/// `expected_seq` is the sequence number this record must carry for strict
/// monotonicity. Returns:
/// - `Record` on success (and leaves the reader at the next record),
/// - `Eof` if the reader is exactly at end-of-file (clean boundary),
/// - `TornTail` if the frame is truncated or its CRC fails (crash mid-append),
/// - `Err(CorruptInterior)` if the record is well-formed and CRC-valid but its
///   sequence number breaks monotonicity (interior corruption that truncation
///   cannot heal).
fn read_one<R: Read>(reader: &mut R, expected_seq: u64) -> Result<ReadOutcome> {
    // Read the fixed header. A short read here means either a clean EOF (0 bytes)
    // or a torn header (1..HEADER_LEN bytes) — both end the log.
    let mut header = [0u8; HEADER_LEN];
    match read_full_or_short(reader, &mut header)? {
        FillState::Empty => return Ok(ReadOutcome::Eof),
        FillState::Short => return Ok(ReadOutcome::TornTail),
        FillState::Full => {}
    }

    let payload_len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    // A length beyond the safety bound can only be corruption in the (final)
    // length field; treat as a torn tail rather than attempting a wild read.
    if payload_len > MAX_PAYLOAD_LEN {
        return Ok(ReadOutcome::TornTail);
    }
    let sequence = u64::from_le_bytes([
        header[4], header[5], header[6], header[7], header[8], header[9], header[10], header[11],
    ]);

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

    // Recompute the CRC over header + payload and compare.
    let mut framed = Vec::with_capacity(HEADER_LEN + payload.len());
    framed.extend_from_slice(&header);
    framed.extend_from_slice(&payload);
    let actual_crc = crc32(&framed);

    if actual_crc != stored_crc {
        // A CRC failure is, by the journal's failure model, a torn/corrupt tail:
        // the record's bytes are present but did not flush atomically, or rot hit
        // a record. In this marker-less format a CRC failure cannot be told apart
        // from a genuine torn tail, so recovery stops here and truncates — see the
        // crate-level "Failure model & a stated limitation": if the failure is in
        // fact an interior record, the trailing (possibly acknowledged) records are
        // dropped with it. We never surface a partial record to the caller.
        return Ok(ReadOutcome::TornTail);
    }

    // CRC is valid ⇒ the record is intact. Sequence must be strictly monotonic;
    // a valid record with the wrong sequence is genuine interior corruption.
    if sequence != expected_seq {
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "sequence number is not strictly monotonic",
        });
    }

    Ok(ReadOutcome::Record(Record { sequence, payload }))
}

/// How much of a target buffer a read managed to fill.
enum FillState {
    /// Zero bytes read at the very first attempt (clean EOF boundary).
    Empty,
    /// Some but not all bytes read (truncated frame).
    Short,
    /// Buffer fully filled.
    Full,
}

/// Fsync the directory containing `path` so a freshly-created (or just-truncated)
/// journal file's directory entry is itself durable.
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
