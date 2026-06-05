//! Best-effort thread-scheduling control for the low-jitter in-core measurement.
//!
//! The in-core latency truth-gate (`core_load`) needs the *measured* tail to
//! reflect the pricing core's intrinsic cost, not OS scheduling jitter. Two
//! affordances reduce that jitter at the source:
//!
//! * **Core pinning** — handled in `core_load` via `core_affinity` (the same
//!   crate the engine pins its hot core with).
//! * **Elevated scheduling priority / QoS** — handled here, via the safe,
//!   cross-platform [`thread_priority`] crate. On macOS this maps to the
//!   `QOS_CLASS` / Mach thread-policy machinery; on Linux/other-POSIX to the
//!   scheduler priority. We request the maximum the platform will grant *for the
//!   current thread only*.
//!
//! Both are *affordances, never requirements*: elevating priority can legitimately
//! fail (an unprivileged process on a hardened host may not be allowed to raise
//! its own priority), in which case we simply measure at the default priority and
//! report `elevated_priority: false`. The §1.2 budgets pass with large margin
//! even unprioritised — priority elevation only tightens the already-passing tail.
//! We **never** treat a failure to elevate as a reason to relax the gate, and we
//! report the outcome honestly in the snapshot rather than asserting success.
//!
//! No `unsafe` is used (nor permitted: the workspace forbids it) — all the
//! platform FFI is encapsulated inside `thread_priority`'s safe API.

use thread_priority::{ThreadPriority, set_current_thread_priority};

/// Request the maximum scheduling priority / QoS the platform will grant for the
/// **current** thread. Returns `true` if the elevation was accepted.
///
/// This is strictly best-effort: a denial (e.g. insufficient privilege) returns
/// `false` and leaves the thread at its default priority. It never panics and
/// never blocks.
#[must_use]
pub fn request_elevated_priority() -> bool {
    set_current_thread_priority(ThreadPriority::Max).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requesting elevated priority must never panic and must return a bool
    /// (whatever the host policy grants). The measurement does not depend on the
    /// result — it only records it — so either outcome is acceptable here; we
    /// assert only that the call is total.
    #[test]
    fn request_elevated_priority_is_total() {
        let _granted: bool = request_elevated_priority();
    }
}
