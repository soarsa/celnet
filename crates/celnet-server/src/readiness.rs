//! The `/readyz`-style blue-green readiness gate and graceful-drain counter
//! (`docs/ARCHITECTURE.md` §5).
//!
//! A stateful low-latency engine cannot rely on a Kubernetes rolling update to
//! avoid dropped connections: it implements an **app-level** drain. This gate is
//! the small, lock-free state machine the cutover is built on:
//!
//! ```text
//!   STARTING ──mark_ready()──▶ READY ──begin_drain()──▶ DRAINING
//!      ▲                                                    │
//!      └──────────────── (process exits) ◀──────────────────┘
//! ```
//!
//! * **`STARTING`** — the process is up but the pricing core is not yet warm; the
//!   `/readyz` probe reports *not ready* so the orchestrator's `SO_REUSEPORT`
//!   steering keeps new connections on the old instance.
//! * **`READY`** — warm and serving; `/readyz` reports *ready* and new
//!   connections are accepted.
//! * **`DRAINING`** — a cutover has begun: `/readyz` reports *not ready* (so new
//!   connections steer to the freshly-warmed replacement), while in-flight
//!   requests are allowed to finish. [`ReadinessGate::await_drained`] blocks the
//!   shutdown until the in-flight count reaches zero (or a timeout elapses),
//!   giving the *zero dropped connections / zero in-flight loss* guarantee.
//!
//! The state and the in-flight counter are plain atomics — the gate is read on
//! every probe and bumped on every request, so it never locks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

/// The blue-green lifecycle state of the edge — mirrors the proto `ServiceState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// Process up, core not yet warm; not accepting traffic.
    Starting,
    /// Warm and serving; the cutover gate reports ready.
    Ready,
    /// Draining in-flight work for a graceful cutover; not accepting traffic.
    Draining,
}

impl ServiceState {
    /// The compact tag stored in the atomic.
    const fn tag(self) -> u8 {
        match self {
            ServiceState::Starting => 0,
            ServiceState::Ready => 1,
            ServiceState::Draining => 2,
        }
    }

    /// Decode a tag previously produced by [`ServiceState::tag`].
    const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ServiceState::Ready,
            2 => ServiceState::Draining,
            _ => ServiceState::Starting,
        }
    }
}

/// A lock-free readiness / drain gate shared by every serving task.
///
/// Cheap to clone behind an [`Arc`]; every method is a single atomic op plus, for
/// the drain wait, a `Notify` wakeup. The gate is the single source of truth for
/// both the `/readyz` probe and the graceful-drain barrier.
#[derive(Debug)]
pub struct ReadinessGate {
    state: AtomicU8,
    in_flight: AtomicU64,
    drained: Notify,
}

impl Default for ReadinessGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadinessGate {
    /// A new gate in [`ServiceState::Starting`] with no in-flight work.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(ServiceState::Starting.tag()),
            in_flight: AtomicU64::new(0),
            drained: Notify::new(),
        }
    }

    /// The current lifecycle state.
    #[must_use]
    pub fn state(&self) -> ServiceState {
        ServiceState::from_tag(self.state.load(Ordering::Acquire))
    }

    /// Whether the edge is accepting new traffic (true only in
    /// [`ServiceState::Ready`]) — the `/readyz` answer.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.state() == ServiceState::Ready
    }

    /// Transition `STARTING → READY`. Idempotent; a no-op once draining.
    ///
    /// Returns `true` if the gate is now ready. Once a drain has begun the gate
    /// never returns to ready (a draining instance is being retired).
    pub fn mark_ready(&self) -> bool {
        // Only promote out of STARTING; never un-drain.
        let _ = self.state.compare_exchange(
            ServiceState::Starting.tag(),
            ServiceState::Ready.tag(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.is_ready()
    }

    /// Transition to [`ServiceState::Draining`] for a graceful cutover.
    ///
    /// New readiness checks immediately fail; in-flight work keeps running. If
    /// there is already no in-flight work, the drained notification fires at once
    /// so a concurrent [`ReadinessGate::await_drained`] returns promptly.
    pub fn begin_drain(&self) {
        self.state
            .store(ServiceState::Draining.tag(), Ordering::Release);
        if self.in_flight.load(Ordering::Acquire) == 0 {
            self.drained.notify_waiters();
        }
    }

    /// The number of requests currently being served.
    #[must_use]
    pub fn in_flight(&self) -> u64 {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Register the start of a request, returning an RAII [`InFlightGuard` ] that
    /// decrements the counter when dropped.
    ///
    /// Bumping the counter on entry and releasing it on drop is what lets a drain
    /// observe true zero in-flight work even if a handler panics or returns
    /// early.
    #[must_use]
    pub fn enter(self: &Arc<Self>) -> InFlightGuard {
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        InFlightGuard {
            gate: Arc::clone(self),
        }
    }

    /// Wait until the in-flight count reaches zero or `timeout` elapses.
    ///
    /// Used by the graceful shutdown to hold the cutover open until in-flight
    /// requests finish. Returns `true` if the edge fully drained, `false` if the
    /// timeout elapsed first (the caller may then force the cutover). Polls under
    /// a `Notify` wakeup so a completing request that drives the count to zero
    /// wakes the waiter immediately.
    pub async fn await_drained(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.in_flight.load(Ordering::Acquire) == 0 {
                return true;
            }
            // Arm the notification *before* re-checking to avoid a lost wakeup.
            let notified = self.drained.notified();
            if self.in_flight.load(Ordering::Acquire) == 0 {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match tokio::time::timeout(deadline - now, notified).await {
                Ok(()) => {} // woken by a completing request; re-check the count.
                Err(_) => return self.in_flight.load(Ordering::Acquire) == 0,
            }
        }
    }
}

/// RAII guard that holds an in-flight slot on a [`ReadinessGate`].
///
/// Dropping it decrements the in-flight counter and, if that drove the count to
/// zero, wakes any task awaiting a drain. Created via [`ReadinessGate::enter`].
#[derive(Debug)]
pub struct InFlightGuard {
    gate: Arc<ReadinessGate>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        // `fetch_sub` returns the *previous* value; reaching 1→0 means drained.
        if self.gate.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.gate.drained.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_not_ready_then_becomes_ready() {
        let gate = ReadinessGate::new();
        assert_eq!(gate.state(), ServiceState::Starting);
        assert!(!gate.is_ready());
        assert!(gate.mark_ready());
        assert_eq!(gate.state(), ServiceState::Ready);
        assert!(gate.is_ready());
    }

    #[test]
    fn drain_makes_not_ready_and_is_terminal() {
        let gate = ReadinessGate::new();
        gate.mark_ready();
        gate.begin_drain();
        assert_eq!(gate.state(), ServiceState::Draining);
        assert!(!gate.is_ready());
        // A drained instance never returns to ready.
        assert!(!gate.mark_ready());
        assert_eq!(gate.state(), ServiceState::Draining);
    }

    #[test]
    fn in_flight_guard_counts() {
        let gate = Arc::new(ReadinessGate::new());
        assert_eq!(gate.in_flight(), 0);
        let g1 = gate.enter();
        let g2 = gate.enter();
        assert_eq!(gate.in_flight(), 2);
        drop(g1);
        assert_eq!(gate.in_flight(), 1);
        drop(g2);
        assert_eq!(gate.in_flight(), 0);
    }

    #[tokio::test]
    async fn await_drained_returns_immediately_when_idle() {
        let gate = ReadinessGate::new();
        gate.begin_drain();
        assert!(gate.await_drained(Duration::from_secs(1)).await);
    }

    #[tokio::test]
    async fn await_drained_waits_for_in_flight_then_returns() {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let guard = gate.enter();
        gate.begin_drain();
        // Release the in-flight slot shortly; the drain wait must then complete.
        let g2 = Arc::clone(&gate);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(guard);
        });
        assert!(g2.await_drained(Duration::from_secs(2)).await);
        assert_eq!(g2.in_flight(), 0);
    }

    #[tokio::test]
    async fn await_drained_times_out_when_work_never_finishes() {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let _guard = gate.enter(); // never released within the timeout
        gate.begin_drain();
        assert!(!gate.await_drained(Duration::from_millis(30)).await);
    }
}
