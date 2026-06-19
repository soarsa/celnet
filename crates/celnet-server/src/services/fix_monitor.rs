//! A bounded, in-memory capture of recent FIX session traffic for the monitor UI.
//!
//! Every inbound frame a managed acceptor reads (and every outbound frame it
//! writes back) is recorded here as a [`FixMessageEvent`] tagged with the managing
//! connection's id, a monotonic capture sequence (the poll cursor), the decoded
//! `MsgType`, and the raw message (SOH rendered as `|` for display). The
//! `FixAdminService.ListMessages` RPC serves a cursored tail of this buffer so the
//! Connections monitor screen can follow a session's traffic without holding a
//! stream open.
//!
//! It is deliberately a **ring buffer**, not a durable log: the newest `capacity`
//! events are retained and older ones evicted, so a long-lived session can be
//! monitored at bounded memory. The capture is best-effort observability and never
//! sits on the pricing hot path — it runs on the per-session async task, behind one
//! short `std::sync::Mutex` push.

use std::collections::VecDeque;
use std::sync::Mutex;

/// The default number of recent events retained per process (across all sessions).
pub const DEFAULT_CAPACITY: usize = 4096;

/// Which way a captured frame was travelling, from the acceptor's vantage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixDirection {
    /// A frame the acceptor **received** from the counterparty.
    Inbound,
    /// A frame the acceptor **sent** to the counterparty.
    Outbound,
}

impl FixDirection {
    /// A stable lowercase token (wire/display).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FixDirection::Inbound => "inbound",
            FixDirection::Outbound => "outbound",
        }
    }
}

/// One captured FIX frame.
#[derive(Debug, Clone)]
pub struct FixMessageEvent {
    /// Monotonic per-process capture sequence (the client's poll cursor).
    pub seq: u64,
    /// The managing connection's id (`FixConnectionDef::id`), or a synthetic id for
    /// the legacy env-seeded acceptor.
    pub connection_id: String,
    /// Travel direction from the acceptor's vantage.
    pub direction: FixDirection,
    /// The FIX `MsgType(35)` value, e.g. `"R"`, `"S"`, `"A"`.
    pub msg_type: String,
    /// A human label for the `MsgType`, e.g. `"QuoteRequest"`.
    pub summary: String,
    /// Edge capture timestamp (epoch nanos), from the shared clock.
    pub epoch_nanos: i64,
    /// The raw FIX message with SOH (`0x01`) rendered as `|` for display.
    pub raw: String,
}

struct Inner {
    seq: u64,
    events: VecDeque<FixMessageEvent>,
}

/// The shared capture buffer. Cheap to clone-share behind an `Arc`.
pub struct FixMonitor {
    inner: Mutex<Inner>,
    capacity: usize,
}

impl std::fmt::Debug for FixMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixMonitor")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl FixMonitor {
    /// A monitor retaining the newest `capacity` events.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                seq: 0,
                events: VecDeque::with_capacity(capacity.min(1024)),
            }),
            capacity: capacity.max(1),
        }
    }

    /// A monitor with [`DEFAULT_CAPACITY`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Record one captured frame, assigning it the next capture sequence and
    /// evicting the oldest event once `capacity` is exceeded. Never blocks on the
    /// pricing core; the lock is held only for the push.
    pub fn record(
        &self,
        connection_id: &str,
        direction: FixDirection,
        raw: &[u8],
        epoch_nanos: i64,
    ) {
        let (msg_type, summary) = classify(raw);
        let event_raw = render_raw(raw);
        // A poisoned lock (a prior panic while recording) must not take down the
        // session — recover the guard and continue; observability is best-effort.
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        g.seq += 1;
        let seq = g.seq;
        g.events.push_back(FixMessageEvent {
            seq,
            connection_id: connection_id.to_owned(),
            direction,
            msg_type,
            summary,
            epoch_nanos,
            raw: event_raw,
        });
        while g.events.len() > self.capacity {
            g.events.pop_front();
        }
    }

    /// The events captured **after** `after_seq` (a cursor of `0` returns the whole
    /// retained buffer), optionally filtered to one `connection_id`, capped at
    /// `limit`. Returns the matched events (oldest-first) plus the highest capture
    /// sequence currently assigned, so the caller advances its cursor even when the
    /// filtered slice is empty.
    #[must_use]
    pub fn since(
        &self,
        connection_id: Option<&str>,
        after_seq: u64,
        limit: usize,
    ) -> (Vec<FixMessageEvent>, u64) {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let latest = g.seq;
        let matched = g
            .events
            .iter()
            .filter(|e| e.seq > after_seq)
            .filter(|e| connection_id.is_none_or(|id| e.connection_id == id))
            .take(limit)
            .cloned()
            .collect();
        (matched, latest)
    }

    /// Like [`since`](Self::since) but restricted to events whose
    /// `connection_id` is in `allowed` — the **desk-scoped** poll a non-admin
    /// session takes (`allowed` is the set of connection ids its desk owns). An
    /// empty set matches nothing (a desk-scoped caller with no visible
    /// connections gets an empty page), while the cursor still advances to the
    /// latest sequence so the client keeps following the tail.
    #[must_use]
    pub fn since_in(
        &self,
        allowed: &std::collections::HashSet<String>,
        after_seq: u64,
        limit: usize,
    ) -> (Vec<FixMessageEvent>, u64) {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let latest = g.seq;
        let matched = g
            .events
            .iter()
            .filter(|e| e.seq > after_seq)
            .filter(|e| allowed.contains(&e.connection_id))
            .take(limit)
            .cloned()
            .collect();
        (matched, latest)
    }
}

impl Default for FixMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Render a raw FIX frame for display: SOH (`0x01`) → `|`, other bytes as UTF-8
/// (lossy), so a monitor row shows `8=FIX.4.4|9=...|35=R|...` verbatim.
fn render_raw(raw: &[u8]) -> String {
    let mapped: Vec<u8> = raw
        .iter()
        .map(|&b| if b == 0x01 { b'|' } else { b })
        .collect();
    String::from_utf8_lossy(&mapped).into_owned()
}

/// Extract the `MsgType(35)` value from a raw frame and map it to a human label.
/// Returns `("", "Unknown")` when tag 35 is absent (a malformed/partial frame).
fn classify(raw: &[u8]) -> (String, String) {
    let code = msg_type_field(raw).unwrap_or_default();
    let label = label_for(&code).to_owned();
    (code, label)
}

/// The value of tag `35` (`MsgType`) in a SOH-delimited frame, if present.
fn msg_type_field(raw: &[u8]) -> Option<String> {
    for field in raw.split(|&b| b == 0x01) {
        if let Some(value) = field.strip_prefix(b"35=") {
            return Some(String::from_utf8_lossy(value).into_owned());
        }
    }
    None
}

/// The human label for a FIX 4.4 `MsgType` code (the subset this venue speaks plus
/// the session-level admin messages a monitor wants named).
fn label_for(code: &str) -> &'static str {
    match code {
        "0" => "Heartbeat",
        "1" => "TestRequest",
        "2" => "ResendRequest",
        "3" => "Reject",
        "4" => "SequenceReset",
        "5" => "Logout",
        "A" => "Logon",
        "R" => "QuoteRequest",
        "S" => "Quote",
        "AG" => "QuoteResponse",
        "AJ" => "QuoteAcknowledgement",
        "b" => "MassQuoteAcknowledgement",
        "D" => "NewOrderSingle",
        "AB" => "NewOrderMultileg",
        "8" => "ExecutionReport",
        "9" => "OrderCancelReject",
        "j" => "BusinessMessageReject",
        "" => "Unknown",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(msg_type: &str) -> Vec<u8> {
        // A minimal SOH-delimited frame carrying BeginString, BodyLength, MsgType.
        format!("8=FIX.4.4\x019=10\x0135={msg_type}\x0110=000\x01").into_bytes()
    }

    #[test]
    fn classifies_and_renders() {
        let m = FixMonitor::new();
        m.record("c1", FixDirection::Inbound, &frame("R"), 1_000);
        let (events, latest) = m.since(None, 0, 100);
        assert_eq!(latest, 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, "R");
        assert_eq!(events[0].summary, "QuoteRequest");
        assert_eq!(events[0].direction, FixDirection::Inbound);
        assert!(events[0].raw.contains("35=R"));
        assert!(
            !events[0].raw.contains('\u{1}'),
            "SOH must be rendered as |"
        );
    }

    #[test]
    fn cursor_returns_only_newer_events() {
        let m = FixMonitor::new();
        m.record("c1", FixDirection::Inbound, &frame("A"), 1);
        m.record("c1", FixDirection::Outbound, &frame("A"), 2);
        let (_first, latest1) = m.since(None, 0, 100);
        m.record("c1", FixDirection::Inbound, &frame("R"), 3);
        let (delta, latest2) = m.since(None, latest1, 100);
        assert_eq!(delta.len(), 1, "only the event after the cursor");
        assert_eq!(delta[0].msg_type, "R");
        assert_eq!(latest2, 3);
    }

    #[test]
    fn filters_by_connection() {
        let m = FixMonitor::new();
        m.record("a", FixDirection::Inbound, &frame("0"), 1);
        m.record("b", FixDirection::Inbound, &frame("0"), 2);
        let (only_b, _) = m.since(Some("b"), 0, 100);
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].connection_id, "b");
    }

    #[test]
    fn since_in_filters_to_the_allowed_set() {
        let m = FixMonitor::new();
        m.record("a", FixDirection::Inbound, &frame("R"), 1);
        m.record("b", FixDirection::Inbound, &frame("R"), 2);
        m.record("c", FixDirection::Inbound, &frame("R"), 3);
        let allowed = std::collections::HashSet::from(["a".to_string(), "c".to_string()]);
        let (events, latest) = m.since_in(&allowed, 0, 100);
        assert_eq!(
            latest, 3,
            "cursor advances to the latest regardless of filter"
        );
        let ids: Vec<&str> = events.iter().map(|e| e.connection_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"], "only allowed-set connections returned");

        // An empty allow-set (a desk-scoped caller with no visible connections)
        // returns no events, but still reports the latest cursor.
        let (none, latest2) = m.since_in(&std::collections::HashSet::new(), 0, 100);
        assert!(none.is_empty());
        assert_eq!(latest2, 3);
    }

    #[test]
    fn evicts_oldest_past_capacity() {
        let m = FixMonitor::with_capacity(2);
        for _ in 0..5 {
            m.record("c", FixDirection::Inbound, &frame("0"), 0);
        }
        let (events, latest) = m.since(None, 0, 100);
        assert_eq!(events.len(), 2, "only the newest two retained");
        assert_eq!(latest, 5, "the cursor keeps counting past evictions");
        assert_eq!(events[0].seq, 4);
        assert_eq!(events[1].seq, 5);
    }
}
