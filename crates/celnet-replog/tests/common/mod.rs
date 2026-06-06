//! Shared test scaffolding: a unique temp directory per node and a hard
//! wall-clock deadline so every replication test fails fast, never hangs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Hard upper bound on any single test body — a regression fails loudly here.
pub(crate) const TEST_DEADLINE: Duration = Duration::from_secs(15);

/// Per-RPC socket timeout: small enough that a dead peer fails fast, large
/// enough to never flake under parallel-nextest contention on loopback.
pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(2);

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh, unique journal path under the OS temp dir for this test process.
pub(crate) fn temp_journal(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut dir = std::env::temp_dir();
    dir.push(format!("celnet-replog-{tag}-{pid}-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.push("log.journal");
    dir
}

/// Assert we are still within the test deadline, panicking loudly otherwise.
#[track_caller]
pub(crate) fn assert_within_deadline(start: Instant) {
    assert!(
        start.elapsed() < TEST_DEADLINE,
        "test exceeded {:?} deadline (possible deadlock/regression)",
        TEST_DEADLINE
    );
}

/// Spin until `cond` is true or the deadline elapses, then return whether it
/// became true. A short sleep yields the scheduler without busy-burning a core.
pub(crate) fn wait_until(start: Instant, mut cond: impl FnMut() -> bool) -> bool {
    while start.elapsed() < TEST_DEADLINE {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    cond()
}
