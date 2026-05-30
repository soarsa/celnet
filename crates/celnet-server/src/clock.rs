//! The edge wall-clock for message timestamping.
//!
//! Pricing logic is strictly wall-clock-free (determinism discipline): a price is
//! a pure function of its market inputs. But the **edge** must stamp messages with
//! real time — a [`celnet_proto::Quote`]'s `epoch_nanos` / `valid_until_nanos`
//! last-look window, an [`celnet_proto::Execution`]'s booking time, and the
//! `epoch_nanos` of a stream snapshot/update/heartbeat — so a counterparty can
//! reason about staleness and a last-look deadline. That timestamping lives here,
//! at the async edge, entirely outside the pricing core.
//!
//! The [`Clock`] is an injectable source so tests can drive deterministic time
//! (e.g. to force a quote past its validity deadline without sleeping): the
//! default reads the system clock; a [`Clock::fixed`] returns a frozen instant and
//! a [`Clock::manual`] advances under explicit control.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The backing time source of a [`Clock`].
#[derive(Debug)]
enum Source {
    /// Read the real system clock on every call.
    System,
    /// A monotonic counter advanced explicitly by tests; never reads the OS.
    Manual(AtomicI64),
}

/// An injectable nanosecond wall-clock for edge message timestamping.
///
/// Cheap to clone behind an [`Arc`]; every method is a single read (or, for the
/// manual source, an atomic op).
#[derive(Debug, Clone)]
pub struct Clock {
    source: Arc<Source>,
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

impl Clock {
    /// A clock reading the real system wall-clock (nanoseconds since the Unix
    /// epoch, UTC). The edge default.
    #[must_use]
    pub fn system() -> Self {
        Self {
            source: Arc::new(Source::System),
        }
    }

    /// A manual clock starting at `start_nanos`, advanced only by
    /// [`Clock::advance`]. Used by tests to drive last-look expiry deterministically
    /// without sleeping.
    #[must_use]
    pub fn manual(start_nanos: i64) -> Self {
        Self {
            source: Arc::new(Source::Manual(AtomicI64::new(start_nanos))),
        }
    }

    /// Advance a manual clock by `delta_nanos`, returning the new value. A no-op on
    /// a system clock.
    pub fn advance(&self, delta_nanos: i64) -> i64 {
        match self.source.as_ref() {
            Source::Manual(v) => v.fetch_add(delta_nanos, Ordering::AcqRel) + delta_nanos,
            Source::System => self.now_nanos(),
        }
    }

    /// The current time in nanoseconds since the Unix epoch (UTC).
    #[must_use]
    pub fn now_nanos(&self) -> i64 {
        match self.source.as_ref() {
            Source::System => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
                .unwrap_or(0),
            Source::Manual(v) => v.load(Ordering::Acquire),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic_nonzero() {
        let c = Clock::system();
        let a = c.now_nanos();
        let b = c.now_nanos();
        assert!(a > 0);
        assert!(b >= a);
    }

    #[test]
    fn manual_clock_advances_under_control() {
        let c = Clock::manual(1_000);
        assert_eq!(c.now_nanos(), 1_000);
        assert_eq!(c.advance(500), 1_500);
        assert_eq!(c.now_nanos(), 1_500);
    }

    #[test]
    fn manual_clock_is_cloneable_shared() {
        let c = Clock::manual(0);
        let d = c.clone();
        c.advance(42);
        assert_eq!(d.now_nanos(), 42);
    }
}
