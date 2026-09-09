//! The firm-wide **pricing kill-switch** runtime state (server-only control plane).
//!
//! Two independent operator controls, both defaulting **on**:
//!
//! * `outbound_enabled` — when `false`, the server sends **no** outbound pricing to
//!   FIX-connected clients: RFQ auto-quote responses are suppressed and RFS/ESP
//!   continuous streams pause. Inbound LP aggregation keeps running (the "Stop all
//!   pricing" button). Resumes from live on re-enable — no stale replay.
//! * `inbound_enabled` — when `false`, LP-feed ingestion into the aggregated books
//!   stops (no new composite prices enter). Both `false` = "Stop all"; both `true` =
//!   "Resume".
//!
//! The two booleans are the **hot-path gates**: every enforcement seam (the pinned
//! zero-alloc FIX session ticker, the LP ingest edge) reads them through a cheap
//! `Relaxed` atomic load — lock-free, alloc-free, log-free (guardrail 11). Control
//! changes bump a monotonic `version` and are fanned out to every connected client
//! over a [`tokio::sync::watch`] channel so all firm-wide GUIs reflect the halt
//! immediately.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use arc_swap::ArcSwap;
use tokio::sync::watch;

/// An immutable snapshot of the two kill-switch controls plus the monotonic version
/// that stamps the change. Fanned out over the [`watch`] channel and serialized into
/// the WS `pricing_control` frame every client renders its halt banner + button state
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PricingControlState {
    /// Whether the server sends outbound pricing to FIX-connected clients.
    pub outbound_enabled: bool,
    /// Whether LP-feed ingestion into the aggregated books runs.
    pub inbound_enabled: bool,
    /// The monotonic version of this control state (starts at 1; `+1` per change).
    pub version: u64,
}

/// The shared, lock-free firm-wide pricing kill-switch. Held behind an `Arc` and
/// threaded into every enforcement seam (aggregation ingest, FIX sessions) and the
/// control/broadcast plane (the `SetPricingControl` RPC + the WS fan-out).
#[derive(Debug)]
pub struct PricingControl {
    outbound: AtomicBool,
    inbound: AtomicBool,
    version: AtomicU64,
    /// Lock-free set of individually halted/locked instrument IDs (CA-G6).
    locked_instruments: ArcSwap<HashSet<String>>,
    /// The change fan-out channel. The sender always holds the current state; every
    /// WS connection subscribes a receiver and forwards a `pricing_control` frame on
    /// each change to its client.
    tx: watch::Sender<PricingControlState>,
}

impl PricingControl {
    /// Construct the control seeded with the two flags. The version starts at `1` and
    /// the watch channel is primed with the initial state.
    #[must_use]
    pub fn new(outbound_enabled: bool, inbound_enabled: bool) -> Arc<Self> {
        let state = PricingControlState {
            outbound_enabled,
            inbound_enabled,
            version: 1,
        };
        let (tx, _rx) = watch::channel(state);
        Arc::new(Self {
            outbound: AtomicBool::new(outbound_enabled),
            inbound: AtomicBool::new(inbound_enabled),
            version: AtomicU64::new(1),
            locked_instruments: ArcSwap::from_pointee(HashSet::new()),
            tx,
        })
    }

    /// The outbound-pricing gate — a cheap `Relaxed` load. This is a **hot-path** read
    /// (the FIX session ticker calls it every tick / every auto-quote): lock-free,
    /// alloc-free, log-free.
    #[must_use]
    #[inline]
    pub fn outbound_enabled(&self) -> bool {
        self.outbound.load(Ordering::Relaxed)
    }

    /// The inbound-ingest gate — a cheap `Relaxed` load. Read at the top of every LP
    /// ingest before any book lock is taken.
    #[must_use]
    #[inline]
    pub fn inbound_enabled(&self) -> bool {
        self.inbound.load(Ordering::Relaxed)
    }

    /// Apply both controls, bump the monotonic version, publish the new state to every
    /// subscriber, and return it. The stores use `Relaxed` (the gates need no ordering
    /// against other state — a control change is a single operator action, and the
    /// worst race is one extra/one fewer priced tick around the flip).
    pub fn set(&self, outbound: bool, inbound: bool) -> PricingControlState {
        self.outbound.store(outbound, Ordering::Relaxed);
        self.inbound.store(inbound, Ordering::Relaxed);
        // `fetch_add` returns the PRIOR value; the new version is prior + 1.
        let version = self.version.fetch_add(1, Ordering::Relaxed) + 1;
        let state = PricingControlState {
            outbound_enabled: outbound,
            inbound_enabled: inbound,
            version,
        };
        // `send_replace` publishes even when there are no receivers (never errors),
        // so the control plane is decoupled from whether any GUI is connected.
        let _ = self.tx.send_replace(state);
        state
    }

    /// A consistent read of all three fields — the current control state (the WS
    /// connect-time value each client is handed before it starts receiving changes).
    #[must_use]
    pub fn snapshot(&self) -> PricingControlState {
        PricingControlState {
            outbound_enabled: self.outbound.load(Ordering::Relaxed),
            inbound_enabled: self.inbound.load(Ordering::Relaxed),
            version: self.version.load(Ordering::Relaxed),
        }
    }

    /// Check whether an individual instrument is locked (halted) from pricing.
    ///
    /// This is a **hot-path** lock-free read off the atomic pointer (zero locks, zero mutexes,
    /// zero allocations).
    #[must_use]
    #[inline]
    pub fn is_instrument_locked(&self, instrument_id: &str) -> bool {
        let set = self.locked_instruments.load();
        set.contains(instrument_id)
    }

    /// Lock (halt) pricing for a specific instrument. Returns true if the instrument was newly locked.
    ///
    /// Uses lock-free RCU (`arc_swap::ArcSwap::rcu`). Updates the monotonic version and notifies
    /// watch subscribers if changed.
    pub fn lock_instrument(&self, instrument_id: impl Into<String>) -> bool {
        let id = instrument_id.into();
        let mut newly_locked = false;
        self.locked_instruments.rcu(|current| {
            if current.contains(&id) {
                newly_locked = false;
                Arc::clone(current)
            } else {
                newly_locked = true;
                let mut next = (**current).clone();
                next.insert(id.clone());
                Arc::new(next)
            }
        });
        if newly_locked {
            self.notify_change();
        }
        newly_locked
    }

    /// Unlock (resume) pricing for a specific instrument. Returns true if the instrument was unlocked.
    ///
    /// Uses lock-free RCU (`arc_swap::ArcSwap::rcu`). Updates the monotonic version and notifies
    /// watch subscribers if changed.
    pub fn unlock_instrument(&self, instrument_id: &str) -> bool {
        let mut unlocked = false;
        self.locked_instruments.rcu(|current| {
            if current.contains(instrument_id) {
                unlocked = true;
                let mut next = (**current).clone();
                next.remove(instrument_id);
                Arc::new(next)
            } else {
                unlocked = false;
                Arc::clone(current)
            }
        });
        if unlocked {
            self.notify_change();
        }
        unlocked
    }

    /// Return an atomic snapshot of all currently locked instruments.
    #[must_use]
    pub fn locked_instruments(&self) -> Arc<HashSet<String>> {
        self.locked_instruments.load_full()
    }

    fn notify_change(&self) {
        let version = self.version.fetch_add(1, Ordering::Relaxed) + 1;
        let state = PricingControlState {
            outbound_enabled: self.outbound.load(Ordering::Relaxed),
            inbound_enabled: self.inbound.load(Ordering::Relaxed),
            version,
        };
        let _ = self.tx.send_replace(state);
    }

    /// A fresh change-fan-out receiver for one WS connection. The returned receiver's
    /// initial `borrow()` is the current state; the connection forwards the current
    /// value once on connect, then a frame on every subsequent change.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<PricingControlState> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_both_enabled_at_version_one() {
        let control = PricingControl::new(true, true);
        let snap = control.snapshot();
        assert!(snap.outbound_enabled);
        assert!(snap.inbound_enabled);
        assert_eq!(snap.version, 1);
        assert!(control.outbound_enabled());
        assert!(control.inbound_enabled());
    }

    #[test]
    fn set_flips_gates_and_bumps_version_monotonically() {
        let control = PricingControl::new(true, true);

        let s1 = control.set(false, true);
        assert!(!s1.outbound_enabled);
        assert!(s1.inbound_enabled);
        assert_eq!(s1.version, 2);
        assert!(!control.outbound_enabled());
        assert!(control.inbound_enabled());

        let s2 = control.set(false, false);
        assert!(!s2.outbound_enabled);
        assert!(!s2.inbound_enabled);
        assert_eq!(s2.version, 3, "version is strictly increasing per change");
        assert!(!control.inbound_enabled());

        let s3 = control.set(true, true);
        assert!(s3.outbound_enabled && s3.inbound_enabled);
        assert_eq!(s3.version, 4);
        // The gates resume live immediately.
        assert!(control.outbound_enabled() && control.inbound_enabled());
    }

    #[tokio::test]
    async fn subscriber_receives_the_new_state_on_change() {
        let control = PricingControl::new(true, true);
        let mut rx = control.subscribe();
        // The initial borrowed value is the current (both-enabled) state.
        assert_eq!(rx.borrow().version, 1);

        let applied = control.set(false, true);
        rx.changed().await.expect("watch sender is alive");
        let observed = *rx.borrow();
        assert_eq!(observed, applied);
        assert!(!observed.outbound_enabled);
        assert!(observed.inbound_enabled);
        assert_eq!(observed.version, 2);
    }

    #[test]
    fn per_instrument_lock_and_unlock_lifecycle() {
        let control = PricingControl::new(true, true);
        assert!(!control.is_instrument_locked("US912828ZG01"));
        assert!(!control.is_instrument_locked("US912828YK50"));

        // Lock one instrument
        assert!(control.lock_instrument("US912828ZG01"));
        assert!(control.is_instrument_locked("US912828ZG01"));
        assert!(!control.is_instrument_locked("US912828YK50"));

        // Duplicate lock is a no-op returning false
        assert!(!control.lock_instrument("US912828ZG01"));

        // Lock second instrument
        assert!(control.lock_instrument("US912828YK50"));
        assert!(control.is_instrument_locked("US912828ZG01"));
        assert!(control.is_instrument_locked("US912828YK50"));

        let locked = control.locked_instruments();
        assert_eq!(locked.len(), 2);
        assert!(locked.contains("US912828ZG01"));
        assert!(locked.contains("US912828YK50"));

        // Unlock first instrument
        assert!(control.unlock_instrument("US912828ZG01"));
        assert!(!control.is_instrument_locked("US912828ZG01"));
        assert!(control.is_instrument_locked("US912828YK50"));

        // Duplicate unlock is a no-op returning false
        assert!(!control.unlock_instrument("US912828ZG01"));

        // Unlock second instrument
        assert!(control.unlock_instrument("US912828YK50"));
        assert!(!control.is_instrument_locked("US912828YK50"));
        assert_eq!(control.locked_instruments().len(), 0);
    }
}
