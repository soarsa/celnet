//! The durable consensus **snapshot**: a compacted capture of the applied state
//! machine at a committed point, the substrate for **log prefix discard** (Raft
//! §7 log compaction).
//!
//! Provenance: this implements the snapshotting of Ongaro & Ousterhout (Raft,
//! USENIX ATC 2014, §7 — and the Ongaro thesis §5 treatment). Per the naming
//! guardrail, public identifiers are purpose-named ([`Snapshot`],
//! [`SnapshotStore`]); the "Raft/snapshot" provenance lives only in doc comments.
//!
//! # Why snapshots exist
//!
//! An append-only replicated log replays in time linear in its length, and grows
//! without bound as proposals stream in. Once a prefix `[0, last_included_index]`
//! is **committed and applied**, its individual entries no longer need to be
//! replayed: their *cumulative effect* is exactly the applied
//! [`crate::state::BookState`] at that point. A snapshot captures that applied
//! state plus the `(last_included_index, last_included_term)` boundary, so the
//! log can discard the now-redundant prefix ([`crate::log::Log`]) and recovery
//! can seed the state machine from the snapshot and replay only the retained tail.
//!
//! # What a snapshot captures
//!
//! * `last_included_index` — the absolute log index of the **last entry the
//!   snapshot subsumes**. Every entry `[0, last_included_index]` is reflected in
//!   the captured state and may be discarded from the log.
//! * `last_included_term` — the term of `log[last_included_index]`. This is
//!   retained so the log-matching check (`matches_prev`) can still be satisfied
//!   for an AppendEntries whose `prev_log_index == last_included_index`, even
//!   though that entry's body is gone (see [`crate::log::Log`]).
//! * the serialized applied [`crate::state::BookState`] at that index, via its
//!   canonical [`crate::state::BookState::encode`] (deterministic, bit-exact).
//!
//! # On-disk format (all little-endian)
//!
//! ```text
//! ┌────────────────────┬────────────────────┬────────────┬───────────────┬───────┐
//! │ last_included_index │ last_included_term │ state_len  │ state_bytes   │ crc32 │
//! │       u64           │       u64          │   u64      │  state_len B  │  u32  │
//! └────────────────────┴────────────────────┴────────────┴───────────────┴───────┘
//! ```
//!
//! The trailing CRC-32 covers everything before it. A missing file means "no
//! snapshot yet". A torn or CRC-failing file (a crash mid-write) is treated as
//! **no snapshot** — conservative and safe, because the durable log still holds
//! every entry the (un-installed) snapshot would have subsumed: a snapshot is
//! only ever installed *after* it is durably written, and the prefix is only
//! discarded *after* that. The write is atomic (temp file → fsync → rename →
//! parent-dir fsync), mirroring [`crate::persist`], so a reader sees either the
//! whole old snapshot or the whole new one, never a torn record.
//!
//! # Ordering invariant (never lose a committed entry)
//!
//! The snapshot is written, then the log prefix is discarded — never the reverse.
//! If a crash interrupts after the snapshot write but before the discard, the log
//! still holds the prefix and recovery simply reconciles the (redundant) overlap;
//! if it interrupts before the snapshot is durable, the snapshot reads back as
//! absent and the full log is replayed. Either way the committed state is exact.

use std::fs;
use std::path::{Path, PathBuf};

use celnet_journal::crc32;

use crate::state::BookState;

/// A durable, CRC-protected capture of the applied state machine at a committed
/// boundary `(last_included_index, last_included_term)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Absolute log index of the last entry this snapshot subsumes (inclusive).
    pub last_included_index: u64,
    /// Term of the entry at `last_included_index` (retained for log-matching at
    /// the snapshot boundary after the prefix is discarded).
    pub last_included_term: u64,
    /// The applied state machine at `last_included_index`.
    pub state: BookState,
}

impl Snapshot {
    /// Construct a snapshot capturing `state` at `(last_included_index,
    /// last_included_term)`.
    #[must_use]
    pub fn new(last_included_index: u64, last_included_term: u64, state: BookState) -> Self {
        Self {
            last_included_index,
            last_included_term,
            state,
        }
    }

    /// Encode the snapshot to its canonical durable/wire byte form (the on-disk
    /// format documented at the module level). Encoding is a pure function of the
    /// fields, so two nodes with the same applied state at the same boundary
    /// encode to byte-identical snapshots.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let state_bytes = self.state.encode();
        let mut buf = Vec::with_capacity(8 + 8 + 8 + state_bytes.len() + 4);
        buf.extend_from_slice(&self.last_included_index.to_le_bytes());
        buf.extend_from_slice(&self.last_included_term.to_le_bytes());
        buf.extend_from_slice(&(state_bytes.len() as u64).to_le_bytes());
        buf.extend_from_slice(&state_bytes);
        let crc = crc32(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf
    }

    /// Decode a snapshot from its byte form, validating the CRC and the embedded
    /// [`BookState`].
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::Malformed`] on a short buffer, an overrunning
    /// `state_len`, a CRC mismatch (corrupt/tampered), or a state payload that
    /// fails to decode.
    pub fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        const HEADER: usize = 8 + 8 + 8;
        if bytes.len() < HEADER + 4 {
            return Err(SnapshotError::Malformed);
        }
        let last_included_index = u64::from_le_bytes(bytes[0..8].try_into().expect("8 bytes"));
        let last_included_term = u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
        let state_len = u64::from_le_bytes(bytes[16..24].try_into().expect("8 bytes")) as usize;
        let state_end = HEADER
            .checked_add(state_len)
            .ok_or(SnapshotError::Malformed)?;
        let crc_end = state_end.checked_add(4).ok_or(SnapshotError::Malformed)?;
        if bytes.len() != crc_end {
            return Err(SnapshotError::Malformed);
        }
        let stored_crc = u32::from_le_bytes(bytes[state_end..crc_end].try_into().expect("4 bytes"));
        if crc32(&bytes[..state_end]) != stored_crc {
            return Err(SnapshotError::Malformed);
        }
        let state =
            BookState::decode(&bytes[HEADER..state_end]).map_err(|_| SnapshotError::Malformed)?;
        Ok(Self {
            last_included_index,
            last_included_term,
            state,
        })
    }
}

/// A durable store for a single [`Snapshot`], backed by an atomically-rewritten,
/// CRC-protected sibling file beside the journal.
pub struct SnapshotStore {
    path: PathBuf,
}

impl SnapshotStore {
    /// Bind a snapshot store at `path` (the snapshot sibling file, e.g.
    /// `<journal>.snapshot`).
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The snapshot file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the durable snapshot, or `None` if none exists yet (or a torn/CRC-
    /// failing file — treated conservatively as absent, see the module docs).
    ///
    /// # Errors
    ///
    /// Propagates a genuine filesystem IO error (other than file-not-found).
    pub fn load(&self) -> std::io::Result<Option<Snapshot>> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Snapshot::decode(&bytes).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Whether a (well-formed) snapshot is present on disk.
    ///
    /// # Errors
    ///
    /// Propagates a genuine filesystem IO error.
    pub fn exists(&self) -> std::io::Result<bool> {
        Ok(self.load()?.is_some())
    }

    /// Durably and atomically write `snapshot` (temp file → fsync → rename →
    /// parent-dir fsync). On return the snapshot survives a crash.
    ///
    /// # Errors
    ///
    /// Propagates the atomic-write IO error.
    pub fn save(&self, snapshot: &Snapshot) -> std::io::Result<()> {
        write_atomic(&self.path, &snapshot.encode())
    }
}

/// A failure decoding a [`Snapshot`] from bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    /// The buffer is too short, the declared `state_len` overruns, the CRC
    /// mismatches, or the embedded state payload is invalid.
    Malformed,
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Malformed => write!(f, "snapshot malformed"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// The conventional snapshot sibling path beside a journal path.
#[must_use]
pub fn snapshot_path(journal_path: &Path) -> PathBuf {
    let mut s = journal_path.as_os_str().to_os_string();
    s.push(".snapshot");
    PathBuf::from(s)
}

/// Write `bytes` to `path` atomically: temp file → fsync → rename → dir fsync.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_data()?;
    }
    fs::rename(&tmp, path)?;
    sync_parent_dir(path)?;
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::BookUpdate;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "celnet-replog-snap-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.push("log.journal.snapshot");
        dir
    }

    fn sample_state() -> BookState {
        let mut s = BookState::new();
        s.apply(&BookUpdate::Add { key: 1, delta: 0.1 });
        s.apply(&BookUpdate::Add { key: 1, delta: 0.2 });
        s.apply(&BookUpdate::Set {
            key: 7,
            value: f64::from_bits(0x3ff0_0000_0000_0001),
        });
        s
    }

    #[test]
    fn round_trips_bit_identically_in_memory() {
        let snap = Snapshot::new(42, 7, sample_state());
        let back = Snapshot::decode(&snap.encode()).expect("decodes");
        assert_eq!(back.last_included_index, 42);
        assert_eq!(back.last_included_term, 7);
        assert_eq!(snap.state.to_bits(), back.state.to_bits());
        // Pure function of the fields.
        assert_eq!(snap.encode(), back.encode());
    }

    #[test]
    fn save_and_load_through_disk() {
        let path = temp_path();
        let store = SnapshotStore::new(&path);
        assert!(store.load().unwrap().is_none(), "no snapshot yet");
        assert!(!store.exists().unwrap());

        let snap = Snapshot::new(10, 3, sample_state());
        store.save(&snap).unwrap();
        assert!(store.exists().unwrap());

        // A fresh store at the same path reads the durable snapshot back.
        let reloaded = SnapshotStore::new(&path).load().unwrap().expect("present");
        assert_eq!(reloaded.last_included_index, 10);
        assert_eq!(reloaded.last_included_term, 3);
        assert_eq!(reloaded.state.to_bits(), snap.state.to_bits());
    }

    #[test]
    fn overwrite_replaces_atomically() {
        let path = temp_path();
        let store = SnapshotStore::new(&path);
        store.save(&Snapshot::new(5, 1, BookState::new())).unwrap();
        let s2 = Snapshot::new(20, 4, sample_state());
        store.save(&s2).unwrap();
        let got = store.load().unwrap().expect("present");
        assert_eq!(got.last_included_index, 20);
        assert_eq!(got.state.to_bits(), s2.state.to_bits());
    }

    #[test]
    fn crc_flip_reads_as_absent() {
        let path = temp_path();
        let store = SnapshotStore::new(&path);
        let mut bytes = Snapshot::new(9, 2, sample_state()).encode();
        bytes[0] ^= 0xff; // corrupt the index → CRC mismatch
        std::fs::write(&path, &bytes).unwrap();
        assert!(
            store.load().unwrap().is_none(),
            "corrupt → treated as absent"
        );
    }

    #[test]
    fn truncated_file_reads_as_absent() {
        let path = temp_path();
        let store = SnapshotStore::new(&path);
        let bytes = Snapshot::new(9, 2, sample_state()).encode();
        std::fs::write(&path, &bytes[..bytes.len() - 3]).unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn snapshot_error_display_is_exact() {
        // Pins the Display string (kills `SnapshotError::fmt -> Ok(Default::default())`).
        assert_eq!(SnapshotError::Malformed.to_string(), "snapshot malformed");
    }

    #[test]
    fn decode_rejects_below_header_plus_crc_and_accepts_the_real_minimum() {
        // The header+crc floor is HEADER(24) + crc(4) = 28 bytes (`bytes.len() <
        // HEADER + 4`, line 118). The smallest ACTUALLY-decodable snapshot is larger
        // (36 bytes): it must additionally carry the embedded empty-state's 8-byte
        // count. We pin the floor here; the `< with <=` boundary at exactly 28 is a
        // provably-equivalent mutant (no 28-byte input ever decodes Ok — the embedded
        // state still needs its 8-byte count — so `< 28` and `<= 28` return the
        // identical `Malformed` for every input), justified in
        // `.config/mutants-celnet-replog.toml`.
        let empty = Snapshot::new(0, 0, BookState::new());
        let bytes = empty.encode();
        assert_eq!(
            bytes.len(),
            36,
            "empty snapshot is 36 bytes (header+crc+state count)"
        );
        // Anything below the 28-byte floor is rejected by the length guard.
        assert_eq!(
            Snapshot::decode(&bytes[..27]),
            Err(SnapshotError::Malformed)
        );
        // The real minimum decodes back to an empty book.
        let back = Snapshot::decode(&bytes).expect("the 36-byte empty snapshot decodes");
        assert_eq!(back.last_included_index, 0);
        assert!(back.state.is_empty());
    }

    #[test]
    fn load_missing_file_is_none_not_an_error() {
        // An absent snapshot file must load to None via the NotFound match guard
        // (line 174) — kills `replace match guard ... with true`, which would treat
        // every io error as file-not-found. A genuinely-absent file is None.
        let path = temp_path();
        assert!(!path.exists());
        let store = SnapshotStore::new(&path);
        assert!(store.load().unwrap().is_none());
        assert!(!store.exists().unwrap());
    }

    #[test]
    fn load_non_notfound_io_error_propagates() {
        // A path whose PARENT is a regular file makes `fs::read` fail with a
        // non-NotFound error; the real code propagates it (`Err(e) => Err(e)`), while
        // the `with true` guard mutant would mask it as None. Pin that it propagates.
        let file = temp_path();
        std::fs::write(&file, b"x").unwrap();
        let nested = file.join("snap"); // <regular-file>/snap → not a directory
        let store = SnapshotStore::new(&nested);
        assert!(
            store.load().is_err(),
            "a non-NotFound IO error must propagate, not become a None load"
        );
    }

    #[test]
    fn empty_state_snapshot_round_trips() {
        let snap = Snapshot::new(0, 1, BookState::new());
        let back = Snapshot::decode(&snap.encode()).expect("decodes");
        assert!(back.state.is_empty());
        assert_eq!(back.last_included_index, 0);
    }
}
