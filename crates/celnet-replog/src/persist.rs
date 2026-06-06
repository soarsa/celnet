//! Durable Raft persistent state: `current_term`, `voted_for`, and a durable
//! `commit_index` watermark (Raft §5).
//!
//! Raft requires `current_term`/`voted_for` to survive a crash *before* a node
//! responds to any RPC that depends on them: a node must never vote twice in a
//! term, nor "forget" a term it has already advanced to, or election safety (§5.4)
//! breaks. We additionally persist the **commit index** watermark: the highest
//! index a majority is known to hold (so it can never be lost or truncated). This
//! makes crash-recovery exact — a restarted node *knows* which prefix is committed
//! and may safely re-apply it to its state machine — while keeping the
//! conflicting-tail truncation safe: only entries *above* the durable commit
//! watermark are ever uncommitted and thus eligible for truncation (the Raft
//! safety property guarantees a committed entry is never overwritten). All three
//! live in a tiny fixed-size, CRC-protected, `fsync`'d file beside the journal.
//!
//! # On-disk format (33 bytes, all little-endian)
//!
//! ```text
//! ┌──────────────┬──────────────┬──────────────┬──────────────┬───────────────┬───────┐
//! │ current_term │  voted_for   │ voted_flag u8│ commit_index │ commit_flag u8│ crc32 │
//! │   u64        │   u64        │(0=None/1=Some)│   u64        │(0=None/1=Some)│  u32  │ +pad
//! └──────────────┴──────────────┴──────────────┴──────────────┴───────────────┴───────┘
//! ```
//!
//! The CRC covers everything before it. A torn or CRC-failing file (a crash
//! mid-write) reads back as the safe default `(term 0, voted None, commit None)`.
//! Writes are atomic (temp file → fsync → rename → dir-fsync), so a reader sees
//! either the old durable state or the new one, never a torn scalar. The
//! commit-index watermark is only ever *advanced* on disk, never rewound.

use std::fs;
use std::path::{Path, PathBuf};

use celnet_journal::crc32;

/// The durable Raft scalars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PersistentState {
    /// Latest term the node has seen (monotonically increasing).
    pub current_term: u64,
    /// The candidate id this node voted for in `current_term`, or `None`.
    pub voted_for: Option<u64>,
    /// Highest index known committed (a majority held it), or `None`. Only ever
    /// advanced, never rewound — a committed entry is never lost or truncated.
    pub commit_index: Option<u64>,
}

/// A durable store for [`PersistentState`], backed by an atomically-rewritten,
/// CRC-protected file.
pub struct PersistStore {
    path: PathBuf,
    state: PersistentState,
}

impl PersistStore {
    /// Open (or initialize) the persistent state at `path`.
    ///
    /// A missing, torn, or CRC-failing file yields the safe default
    /// `(term 0, None)` — never an error (a crash mid-write is expected and
    /// recovered conservatively, see the module docs).
    ///
    /// # Errors
    ///
    /// Propagates a genuine filesystem IO error (other than file-not-found).
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let state = match fs::read(&path) {
            Ok(bytes) => decode(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PersistentState::default(),
            Err(e) => return Err(e),
        };
        Ok(Self { path, state })
    }

    /// The current durable state.
    #[must_use]
    pub fn state(&self) -> PersistentState {
        self.state
    }

    /// The current term.
    #[must_use]
    pub fn current_term(&self) -> u64 {
        self.state.current_term
    }

    /// The id voted for in the current term, if any.
    #[must_use]
    pub fn voted_for(&self) -> Option<u64> {
        self.state.voted_for
    }

    /// The durable commit-index watermark, if any.
    #[must_use]
    pub fn commit_index(&self) -> Option<u64> {
        self.state.commit_index
    }

    /// Durably set `current_term` and `voted_for` (preserving the commit watermark)
    /// in one atomic write.
    ///
    /// # Errors
    ///
    /// Propagates the atomic-write IO error.
    pub fn save(&mut self, current_term: u64, voted_for: Option<u64>) -> std::io::Result<()> {
        self.write(PersistentState {
            current_term,
            voted_for,
            commit_index: self.state.commit_index,
        })
    }

    /// Durably advance the commit-index watermark (monotonic; a request to lower
    /// it is ignored). Preserves `current_term`/`voted_for`.
    ///
    /// # Errors
    ///
    /// Propagates the atomic-write IO error.
    pub fn save_commit_index(&mut self, commit_index: Option<u64>) -> std::io::Result<()> {
        // Monotonic guard: never rewind a committed watermark.
        let advanced = match (self.state.commit_index, commit_index) {
            (Some(cur), Some(new)) => Some(cur.max(new)),
            (Some(cur), None) => Some(cur),
            (None, x) => x,
        };
        self.write(PersistentState {
            current_term: self.state.current_term,
            voted_for: self.state.voted_for,
            commit_index: advanced,
        })
    }

    /// Atomic write with no-op avoidance.
    fn write(&mut self, next: PersistentState) -> std::io::Result<()> {
        if next == self.state {
            return Ok(());
        }
        write_atomic(&self.path, &encode(&next))?;
        self.state = next;
        Ok(())
    }
}

/// Length of the CRC-protected body (before the trailing crc32), incl. padding.
const BODY_LEN: usize = 8 + 8 + 1 + 8 + 1 + 2; // term, voted, vflag, commit, cflag, pad
/// Total record length: body + crc32.
const RECORD_LEN: usize = BODY_LEN + 4;

/// Encode the fixed-size CRC-protected record.
fn encode(s: &PersistentState) -> Vec<u8> {
    let mut body = Vec::with_capacity(RECORD_LEN);
    body.extend_from_slice(&s.current_term.to_le_bytes());
    body.extend_from_slice(&s.voted_for.unwrap_or(0).to_le_bytes());
    body.push(u8::from(s.voted_for.is_some()));
    body.extend_from_slice(&s.commit_index.unwrap_or(0).to_le_bytes());
    body.push(u8::from(s.commit_index.is_some()));
    body.extend_from_slice(&[0u8; 2]); // pad to align the crc
    debug_assert_eq!(body.len(), BODY_LEN);
    let crc = crc32(&body);
    body.extend_from_slice(&crc.to_le_bytes());
    body
}

/// Decode the record, validating the CRC. Returns `None` on any malformation.
fn decode(bytes: &[u8]) -> Option<PersistentState> {
    if bytes.len() != RECORD_LEN {
        return None;
    }
    let body = &bytes[..BODY_LEN];
    let stored_crc = u32::from_le_bytes(bytes[BODY_LEN..RECORD_LEN].try_into().ok()?);
    if crc32(body) != stored_crc {
        return None;
    }
    let current_term = u64::from_le_bytes(body[0..8].try_into().ok()?);
    let voted_raw = u64::from_le_bytes(body[8..16].try_into().ok()?);
    let voted_for = (body[16] == 1).then_some(voted_raw);
    let commit_raw = u64::from_le_bytes(body[17..25].try_into().ok()?);
    let commit_index = (body[25] == 1).then_some(commit_raw);
    Some(PersistentState {
        current_term,
        voted_for,
        commit_index,
    })
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
            "celnet-replog-persist-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.push("raft.state");
        dir
    }

    #[test]
    fn round_trips_through_disk() {
        let path = temp_path();
        {
            let mut s = PersistStore::open(&path).unwrap();
            assert_eq!(s.current_term(), 0);
            assert_eq!(s.voted_for(), None);
            s.save(5, Some(40_001)).unwrap();
        }
        let s = PersistStore::open(&path).unwrap();
        assert_eq!(s.current_term(), 5);
        assert_eq!(s.voted_for(), Some(40_001));
    }

    #[test]
    fn clears_vote_on_new_term() {
        let path = temp_path();
        let mut s = PersistStore::open(&path).unwrap();
        s.save(3, Some(7)).unwrap();
        s.save(4, None).unwrap(); // new term, vote cleared
        assert_eq!(s.current_term(), 4);
        assert_eq!(s.voted_for(), None);
        let reopened = PersistStore::open(&path).unwrap();
        assert_eq!(reopened.voted_for(), None);
    }

    #[test]
    fn commit_watermark_persists_and_is_monotonic() {
        let path = temp_path();
        {
            let mut s = PersistStore::open(&path).unwrap();
            assert_eq!(s.commit_index(), None);
            s.save(2, Some(1)).unwrap();
            s.save_commit_index(Some(3)).unwrap();
            // A request to rewind is ignored (monotonic).
            s.save_commit_index(Some(1)).unwrap();
            assert_eq!(s.commit_index(), Some(3));
            // And term/vote are preserved across a commit-index write.
            assert_eq!(s.current_term(), 2);
            assert_eq!(s.voted_for(), Some(1));
        }
        let s = PersistStore::open(&path).unwrap();
        assert_eq!(s.commit_index(), Some(3));
        assert_eq!(s.current_term(), 2);
        assert_eq!(s.voted_for(), Some(1));
    }

    #[test]
    fn corrupt_file_reads_as_default() {
        let path = temp_path();
        std::fs::write(&path, b"garbage-not-24-bytes").unwrap();
        let s = PersistStore::open(&path).unwrap();
        assert_eq!(s.state(), PersistentState::default());
    }

    #[test]
    fn crc_flip_reads_as_default() {
        let path = temp_path();
        let mut bytes = encode(&PersistentState {
            current_term: 9,
            voted_for: Some(2),
            commit_index: Some(4),
        });
        bytes[0] ^= 0xff; // corrupt the term, CRC now mismatches
        std::fs::write(&path, &bytes).unwrap();
        let s = PersistStore::open(&path).unwrap();
        assert_eq!(s.state(), PersistentState::default());
    }
}
