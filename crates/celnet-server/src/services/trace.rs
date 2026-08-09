//! End-to-end lift **event tracing** — the per-trace event log that COMPLEMENTS
//! the aggregate per-stage latency histograms (`services::telemetry` /
//! `ListLatencyMetrics`). Where the histograms answer *"how fast is stage X
//! across all flow"*, a trace answers *"what happened, stage by stage, to THIS
//! one lift"*: price → quote → order → last-look → acceptance → risk-routing →
//! deal → hedge, each a timestamped [`TraceStage`] event carrying that stage's
//! specifics, all sharing one `trace_id`
//! (`docs/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md` Part 2).
//!
//! ## Hot-core contract (guardrail 11)
//!
//! Capture is **off the pinned zero-alloc pricing core**. The pinned pricer
//! lives in `celnet-engine`, which `celnet-server` depends on one-way, so it
//! *cannot* reference [`TraceHub`] — the capture calls only ever originate on the
//! async service edges (`services::fix`, `services::rates_book`), which are
//! already off the pricing thread (the exact same tier as `TelemetryHub::
//! record_edge`). A stage site does the minimum: it stamps a monotonic timestamp
//! and `try_send`s a POD-ish [`RawEvent`] onto a **bounded, lossy** offload
//! channel — never a heap-heavy store operation, never a blocking send. A single
//! **drain worker** ([`TraceHub::spawn_drain`]) folds the offload into the capped
//! ring [`TraceStore`]; the query RPCs read that store. So the producer's inline
//! cost is one atomic + one `try_send`, and *all* indexing / eviction runs on the
//! drain tier.
//!
//! ## Trace-id minting + propagation
//!
//! A `trace_id` is minted when a quote/price is created (the FIX auto-quote / MD
//! snapshot emit site). It is carried to the order that lifts that quote via the
//! existing correlation keys — the same `QuoteID(117)` reverse index the FIX
//! session already keeps (`quote_id → trace_id` here), plus a `request_id →
//! trace_id` binding so the booking path (which flows the acceptance `request_id`,
//! not the QuoteID) resolves the same trace. The booked deal's `position_id` is
//! then bound to the trace (`position_id → trace_id`) so the auto-hedge fire —
//! keyed by `HedgeProvenance.parent_position_id` = the deal's `position_id` —
//! inherits it. Where a flow has no prior quote, the order site mints a fresh id.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use celnet_proto::{TraceEvent, TraceStage, TraceSummary};
use tokio::sync::mpsc;

/// Ring capacity: the number of distinct traces the store retains before it
/// evicts the oldest (FIFO). Sized for a deep recent-history window at IB-scale
/// lift rates while staying bounded memory (guardrail 6). Eviction is logged.
const DEFAULT_STORE_CAPACITY: usize = 4096;

/// Capacity of each `key → trace_id` correlation index (quote-id, request-id,
/// position-id). Bounded + FIFO-evicting so a long-running process never grows
/// the maps without bound; comfortably larger than the in-flight lift set.
const DEFAULT_INDEX_CAPACITY: usize = 16_384;

/// Bound of the producer→drain offload channel. A full channel drops-and-counts
/// (lossy telemetry, never blocks the producing stage), mirroring the pinned-core
/// telemetry ring's drop-on-full discipline.
const DEFAULT_OFFLOAD_BOUND: usize = 8192;

/// Default and maximum row cap for `ListTraces` (server-clamped).
const DEFAULT_LIST_LIMIT: usize = 200;
const MAX_LIST_LIMIT: usize = 2000;

/// The per-stage detail payload a capture site supplies. Every field is optional
/// except the always-echoed `side`; the store carries them onto the wire
/// [`TraceEvent`] verbatim (absent → `null`).
#[derive(Debug, Clone, Default)]
pub struct TraceDetails {
    /// Dealt/quoted side from the counterparty's perspective ("buy"/"sell"), or
    /// empty when side-agnostic.
    pub side: String,
    /// The price at this stage (computed / quoted / presented / dealt / hedge).
    pub price: Option<f64>,
    /// The notional / size in scope at this stage.
    pub notional: Option<f64>,
    /// The `QuoteID(117)` tying the quote↔order stages.
    pub quote_id: Option<String>,
    /// The booked `Deal.deal_id`.
    pub deal_id: Option<String>,
    /// The routed `Deal.risk_book_id`.
    pub book_id: Option<String>,
    /// The originating counterparty (FIX session / PartyID).
    pub counterparty: Option<String>,
    /// Stage decision text (last-look / acceptance verdict).
    pub decision: Option<String>,
    /// The fired `HedgeProvenance.hedge_id`.
    pub hedge_id: Option<String>,
    /// Free-form human-readable annotation (RAG band, reject reason, LP won).
    pub detail: Option<String>,
    /// The booked `Deal.position_id` — the durable fill key the hedge stage shares.
    pub position_id: Option<u64>,
}

/// A captured stage event, in transit on the offload channel. Owned (carries its
/// strings) so the producer hands it off and returns immediately. Crate-internal:
/// only the hub constructs it and only the drain worker consumes it.
#[derive(Debug, Clone)]
pub(crate) struct RawEvent {
    trace_id: u64,
    /// [`TraceStage`] discriminant (`as i32`).
    stage: i32,
    timestamp_ns: u64,
    symbol: String,
    details: TraceDetails,
}

/// One stage event as retained in the store (seq is assigned at read time, in
/// timestamp order, so it is not stored).
#[derive(Debug, Clone)]
struct StoredEvent {
    stage: i32,
    timestamp_ns: u64,
    side: String,
    price: Option<f64>,
    notional: Option<f64>,
    quote_id: Option<String>,
    deal_id: Option<String>,
    book_id: Option<String>,
    counterparty: Option<String>,
    decision: Option<String>,
    hedge_id: Option<String>,
    detail: Option<String>,
    position_id: Option<u64>,
}

/// All events of one trace.
#[derive(Debug, Default)]
struct TraceRecord {
    symbol: String,
    counterparty: Option<String>,
    events: Vec<StoredEvent>,
}

/// The capped ring store — the drain-side aggregation, touched only by the drain
/// worker (writes) and the query RPCs (reads), never by the pinned core.
#[derive(Debug)]
struct TraceStore {
    capacity: usize,
    traces: HashMap<u64, TraceRecord>,
    /// Trace ids in first-seen order — the FIFO eviction queue.
    order: VecDeque<u64>,
    /// Lifetime count of traces evicted at capacity.
    evicted: u64,
}

impl TraceStore {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            traces: HashMap::new(),
            order: VecDeque::new(),
            evicted: 0,
        }
    }

    /// Fold one captured event into the store, evicting the oldest trace (FIFO)
    /// when a NEW trace would exceed capacity.
    fn insert(&mut self, ev: RawEvent) {
        if !self.traces.contains_key(&ev.trace_id) {
            while self.traces.len() >= self.capacity {
                if let Some(old) = self.order.pop_front() {
                    self.traces.remove(&old);
                    self.evicted += 1;
                    tracing::warn!(
                        target: "celnet::trace",
                        evicted_trace_id = old,
                        capacity = self.capacity,
                        evicted_total = self.evicted,
                        "trace store at capacity — evicted oldest trace"
                    );
                } else {
                    break;
                }
            }
            self.order.push_back(ev.trace_id);
            self.traces.insert(
                ev.trace_id,
                TraceRecord {
                    symbol: ev.symbol.clone(),
                    counterparty: ev.details.counterparty.clone(),
                    events: Vec::new(),
                },
            );
        }

        let record = self
            .traces
            .get_mut(&ev.trace_id)
            .expect("trace record present after insert");
        // First non-empty symbol / counterparty wins for the summary header.
        if record.symbol.is_empty() && !ev.symbol.is_empty() {
            record.symbol = ev.symbol.clone();
        }
        if record.counterparty.is_none() {
            record.counterparty = ev.details.counterparty.clone();
        }
        record.events.push(StoredEvent {
            stage: ev.stage,
            timestamp_ns: ev.timestamp_ns,
            side: ev.details.side,
            price: ev.details.price,
            notional: ev.details.notional,
            quote_id: ev.details.quote_id,
            deal_id: ev.details.deal_id,
            book_id: ev.details.book_id,
            counterparty: ev.details.counterparty,
            decision: ev.details.decision,
            hedge_id: ev.details.hedge_id,
            detail: ev.details.detail,
            position_id: ev.details.position_id,
        });
    }

    /// The ordered wire events of one trace (empty when unknown/evicted). Events
    /// are sorted by timestamp and re-`seq`-numbered 0..n so the client renders a
    /// deterministic, gap-free timeline regardless of producer interleaving.
    fn get_trace(&self, trace_id: u64) -> Vec<TraceEvent> {
        let Some(record) = self.traces.get(&trace_id) else {
            return Vec::new();
        };
        let mut events: Vec<&StoredEvent> = record.events.iter().collect();
        events.sort_by_key(|e| e.timestamp_ns);
        events
            .into_iter()
            .enumerate()
            .map(|(i, e)| TraceEvent {
                trace_id,
                seq: i as u32,
                stage: e.stage,
                timestamp_ns: e.timestamp_ns,
                symbol: record.symbol.clone(),
                side: e.side.clone(),
                price: e.price,
                notional: e.notional,
                quote_id: e.quote_id.clone(),
                deal_id: e.deal_id.clone(),
                book_id: e.book_id.clone(),
                counterparty: e.counterparty.clone(),
                decision: e.decision.clone(),
                hedge_id: e.hedge_id.clone(),
                detail: e.detail.clone(),
                position_id: e.position_id,
            })
            .collect()
    }

    /// Recent trace summaries, newest-first (by last-event timestamp), filtered
    /// and bounded.
    fn list_traces(
        &self,
        limit: usize,
        symbol: Option<&str>,
        counterparty: Option<&str>,
    ) -> Vec<TraceSummary> {
        let mut summaries: Vec<TraceSummary> = self
            .traces
            .iter()
            .filter_map(|(&id, record)| summarize(id, record))
            .filter(|s| symbol.is_none_or(|f| s.symbol == f))
            .filter(|s| counterparty.is_none_or(|f| s.counterparty.as_deref() == Some(f)))
            .collect();
        // Newest first by the last event's timestamp (most recently active trace).
        summaries.sort_by(|a, b| b.last_timestamp_ns.cmp(&a.last_timestamp_ns));
        summaries.truncate(limit);
        summaries
    }
}

/// Build a one-row summary of a trace, or `None` if it somehow has no events.
fn summarize(trace_id: u64, record: &TraceRecord) -> Option<TraceSummary> {
    if record.events.is_empty() {
        return None;
    }
    let first = record
        .events
        .iter()
        .min_by_key(|e| e.timestamp_ns)
        .expect("non-empty");
    let last = record
        .events
        .iter()
        .max_by_key(|e| e.timestamp_ns)
        .expect("non-empty");
    Some(TraceSummary {
        trace_id,
        first_stage: first.stage,
        last_stage: last.stage,
        first_timestamp_ns: first.timestamp_ns,
        last_timestamp_ns: last.timestamp_ns,
        total_latency_ns: last.timestamp_ns.saturating_sub(first.timestamp_ns),
        event_count: record.events.len() as u32,
        symbol: record.symbol.clone(),
        counterparty: record.counterparty.clone(),
        outcome: outcome_of(&record.events),
    })
}

/// Derive the terminal outcome from the captured stages: a fired hedge ⇒
/// "hedged"; a booked deal ⇒ "booked"; a rejecting last-look / acceptance ⇒
/// "rejected"; otherwise still "in_flight".
fn outcome_of(events: &[StoredEvent]) -> String {
    let has = |stage: TraceStage| events.iter().any(|e| e.stage == stage as i32);
    if has(TraceStage::HedgeFired) {
        return "hedged".to_owned();
    }
    if has(TraceStage::DealBooked) {
        return "booked".to_owned();
    }
    let rejected = events.iter().any(|e| {
        matches!(e.stage, s if s == TraceStage::AcceptanceDecided as i32 || s == TraceStage::LastLook as i32)
            && e
                .decision
                .as_deref()
                .is_some_and(|d| {
                    let d = d.to_ascii_lowercase();
                    d.starts_with("reject")
                        || d.contains("expired")
                        || d.contains("replayed")
                        || d.contains("superseded")
                })
    });
    if rejected {
        "rejected".to_owned()
    } else {
        "in_flight".to_owned()
    }
}

/// A bounded, FIFO-evicting `String → trace_id` correlation index.
#[derive(Debug)]
struct KeyIndex {
    capacity: usize,
    map: HashMap<String, u64>,
    order: VecDeque<String>,
}

impl KeyIndex {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, key: String, trace_id: u64) {
        if !self.map.contains_key(&key) {
            while self.map.len() >= self.capacity {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                } else {
                    break;
                }
            }
            self.order.push_back(key.clone());
        }
        self.map.insert(key, trace_id);
    }

    fn get(&self, key: &str) -> Option<u64> {
        self.map.get(key).copied()
    }
}

/// A bounded, FIFO-evicting `u64 → trace_id` correlation index (for position ids).
#[derive(Debug)]
struct PosIndex {
    capacity: usize,
    map: HashMap<u64, u64>,
    order: VecDeque<u64>,
}

impl PosIndex {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, key: u64, trace_id: u64) {
        if !self.map.contains_key(&key) {
            while self.map.len() >= self.capacity {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                } else {
                    break;
                }
            }
            self.order.push_back(key);
        }
        self.map.insert(key, trace_id);
    }

    fn get(&self, key: u64) -> Option<u64> {
        self.map.get(&key).copied()
    }
}

/// The shared trace hub (behind an `Arc`): the producer-facing capture + minting
/// + correlation indexes, plus the drain-side [`TraceStore`] the query RPCs read.
/// Constructed once at boot; the SAME `Arc` is cloned into the FIX + rates-book
/// services (producers) and into `AuthEdge` (reader).
#[derive(Debug)]
pub struct TraceHub {
    /// Next trace id (ids start at 1; 0 is reserved "no trace").
    next_trace_id: AtomicU64,
    /// Monotonic-clock epoch (process-start `Instant`).
    epoch: Instant,
    /// Last handed-out nanosecond timestamp — the strictly-increasing guard.
    last_ns: AtomicU64,
    /// The producer→drain offload sender (`try_send`, lossy).
    tx: mpsc::Sender<RawEvent>,
    /// Events the producer dropped because the offload was full (lossy telemetry).
    dropped: AtomicU64,
    /// `QuoteID(117) → trace_id` (bound at quote publish, read at order lift).
    quote_index: Mutex<KeyIndex>,
    /// `request_id → trace_id` (bound at quote publish, read at booking).
    request_index: Mutex<KeyIndex>,
    /// `position_id → trace_id` (bound at booking, read at hedge fire).
    position_index: Mutex<PosIndex>,
    /// The drain-side store.
    store: Mutex<TraceStore>,
}

impl TraceHub {
    /// Build a hub with explicit capacities and return it beside the offload
    /// receiver the [drain worker](Self::spawn_drain) consumes.
    #[must_use]
    pub(crate) fn with_capacities(
        store_capacity: usize,
        index_capacity: usize,
        offload_bound: usize,
    ) -> (std::sync::Arc<Self>, mpsc::Receiver<RawEvent>) {
        let (tx, rx) = mpsc::channel(offload_bound.max(1));
        let hub = std::sync::Arc::new(Self {
            next_trace_id: AtomicU64::new(1),
            epoch: Instant::now(),
            last_ns: AtomicU64::new(0),
            tx,
            dropped: AtomicU64::new(0),
            quote_index: Mutex::new(KeyIndex::new(index_capacity)),
            request_index: Mutex::new(KeyIndex::new(index_capacity)),
            position_index: Mutex::new(PosIndex::new(index_capacity)),
            store: Mutex::new(TraceStore::new(store_capacity)),
        });
        (hub, rx)
    }

    /// Build a hub with the default IB-scale capacities.
    #[must_use]
    pub(crate) fn new() -> (std::sync::Arc<Self>, mpsc::Receiver<RawEvent>) {
        Self::with_capacities(
            DEFAULT_STORE_CAPACITY,
            DEFAULT_INDEX_CAPACITY,
            DEFAULT_OFFLOAD_BOUND,
        )
    }

    /// Spawn the single drain worker: it recv-loops the offload and folds each
    /// event into the store on a non-critical core. Ends when all producers drop
    /// (process shutdown) or the shutdown flag is set after the channel closes.
    pub(crate) fn spawn_drain(
        hub: std::sync::Arc<Self>,
        mut rx: mpsc::Receiver<RawEvent>,
        shutdown: std::sync::Arc<AtomicBool>,
    ) {
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                hub.ingest(ev);
                if shutdown.load(Ordering::Acquire) {
                    // Drain whatever is already queued, then stop.
                    while let Ok(ev) = rx.try_recv() {
                        hub.ingest(ev);
                    }
                    break;
                }
            }
        });
    }

    /// Fold one event into the store (drain-tier; locks the store briefly).
    fn ingest(&self, ev: RawEvent) {
        self.store
            .lock()
            .expect("trace store mutex not poisoned")
            .insert(ev);
    }

    /// Mint a fresh trace id (monotonic, unique for the process).
    #[must_use]
    pub fn mint_trace(&self) -> u64 {
        self.next_trace_id.fetch_add(1, Ordering::Relaxed)
    }

    /// A monotonic, **strictly increasing** capture timestamp (ns from the epoch)
    /// — so events never tie and inter-stage latency is always well-defined.
    #[must_use]
    pub fn now_ns(&self) -> u64 {
        let raw = self.epoch.elapsed().as_nanos() as u64;
        loop {
            let prev = self.last_ns.load(Ordering::Relaxed);
            let next = raw.max(prev + 1);
            if self
                .last_ns
                .compare_exchange_weak(prev, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return next;
            }
        }
    }

    /// Bind a `QuoteID(117)` to a trace (at quote publish).
    pub fn bind_quote(&self, quote_id: impl Into<String>, trace_id: u64) {
        self.quote_index
            .lock()
            .expect("quote index mutex not poisoned")
            .insert(quote_id.into(), trace_id);
    }

    /// Bind an acceptance `request_id` to a trace (at quote publish).
    pub fn bind_request(&self, request_id: impl Into<String>, trace_id: u64) {
        self.request_index
            .lock()
            .expect("request index mutex not poisoned")
            .insert(request_id.into(), trace_id);
    }

    /// Bind a booked `position_id` to a trace (at booking) — the hop the hedge
    /// stage resolves through (`HedgeProvenance.parent_position_id`).
    pub fn bind_position(&self, position_id: u64, trace_id: u64) {
        self.position_index
            .lock()
            .expect("position index mutex not poisoned")
            .insert(position_id, trace_id);
    }

    /// Resolve the trace a `QuoteID(117)` belongs to (at order lift).
    #[must_use]
    pub fn resolve_quote(&self, quote_id: &str) -> Option<u64> {
        self.quote_index
            .lock()
            .expect("quote index mutex not poisoned")
            .get(quote_id)
    }

    /// Resolve the trace an acceptance `request_id` belongs to (at booking).
    #[must_use]
    pub fn resolve_request(&self, request_id: &str) -> Option<u64> {
        self.request_index
            .lock()
            .expect("request index mutex not poisoned")
            .get(request_id)
    }

    /// Resolve the trace a booked `position_id` belongs to (at hedge fire).
    #[must_use]
    pub fn resolve_position(&self, position_id: u64) -> Option<u64> {
        self.position_index
            .lock()
            .expect("position index mutex not poisoned")
            .get(position_id)
    }

    /// Capture one stage event: stamp a monotonic timestamp and `try_send` it onto
    /// the bounded offload. **Non-blocking, off the pinned core**; a full offload
    /// drops the event and bumps the lossy-drop counter rather than blocking the
    /// producing stage.
    pub fn record(&self, trace_id: u64, stage: TraceStage, symbol: &str, details: TraceDetails) {
        let ev = RawEvent {
            trace_id,
            stage: stage as i32,
            timestamp_ns: self.now_ns(),
            symbol: symbol.to_owned(),
            details,
        };
        if self.tx.try_send(ev).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The full ordered event list of one trace (reader RPC).
    #[must_use]
    pub fn get_trace(&self, trace_id: u64) -> Vec<TraceEvent> {
        self.store
            .lock()
            .expect("trace store mutex not poisoned")
            .get_trace(trace_id)
    }

    /// Recent trace summaries (reader RPC), newest first, clamped to the server
    /// cap; `limit == 0`/absent ⇒ the default.
    #[must_use]
    pub fn list_traces(
        &self,
        limit: Option<u32>,
        symbol: Option<&str>,
        counterparty: Option<&str>,
    ) -> Vec<TraceSummary> {
        let cap = match limit {
            Some(0) | None => DEFAULT_LIST_LIMIT,
            Some(n) => (n as usize).min(MAX_LIST_LIMIT),
        };
        self.store
            .lock()
            .expect("trace store mutex not poisoned")
            .list_traces(cap, symbol, counterparty)
    }

    /// Events dropped because the offload was full (lossy-capture accounting).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Traces evicted at store capacity (lifetime).
    #[must_use]
    pub fn evicted(&self) -> u64 {
        self.store
            .lock()
            .expect("trace store mutex not poisoned")
            .evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directly fold an event into the store, bypassing the async offload, so the
    /// store logic is tested deterministically.
    fn ingest(hub: &TraceHub, trace_id: u64, stage: TraceStage, symbol: &str, d: TraceDetails) {
        hub.ingest(RawEvent {
            trace_id,
            stage: stage as i32,
            timestamp_ns: hub.now_ns(),
            symbol: symbol.to_owned(),
            details: d,
        });
    }

    fn details(side: &str) -> TraceDetails {
        TraceDetails {
            side: side.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn mint_is_unique_and_monotonic() {
        let (hub, _rx) = TraceHub::new();
        let a = hub.mint_trace();
        let b = hub.mint_trace();
        let c = hub.mint_trace();
        assert_eq!(a, 1);
        assert!(b > a && c > b);
    }

    #[test]
    fn now_ns_is_strictly_increasing() {
        let (hub, _rx) = TraceHub::new();
        let mut prev = 0;
        for _ in 0..10_000 {
            let t = hub.now_ns();
            assert!(t > prev, "timestamps must strictly increase: {t} !> {prev}");
            prev = t;
        }
    }

    #[test]
    fn one_lift_produces_one_ordered_trace() {
        let (hub, _rx) = TraceHub::new();
        let tid = hub.mint_trace();
        // A full lift lifecycle, in stage order, all sharing `tid`.
        ingest(&hub, tid, TraceStage::PriceComputed, "EURUSD", {
            let mut d = details("buy");
            d.price = Some(1.0850);
            d.notional = Some(5_000_000.0);
            d
        });
        ingest(&hub, tid, TraceStage::QuotePublished, "EURUSD", {
            let mut d = details("buy");
            d.quote_id = Some("Q-1".into());
            d.counterparty = Some("cp-a".into());
            d
        });
        ingest(
            &hub,
            tid,
            TraceStage::OrderReceived,
            "EURUSD",
            details("buy"),
        );
        ingest(&hub, tid, TraceStage::LastLook, "EURUSD", {
            let mut d = details("buy");
            d.decision = Some("accepted".into());
            d
        });
        ingest(&hub, tid, TraceStage::AcceptanceDecided, "EURUSD", {
            let mut d = details("buy");
            d.decision = Some("accept".into());
            d
        });
        ingest(&hub, tid, TraceStage::RiskRouted, "EURUSD", {
            let mut d = details("buy");
            d.book_id = Some("BOOK-G10".into());
            d
        });
        ingest(&hub, tid, TraceStage::DealBooked, "EURUSD", {
            let mut d = details("buy");
            d.deal_id = Some("D-42".into());
            d
        });
        ingest(
            &hub,
            tid,
            TraceStage::HedgeDecided,
            "EURUSD",
            details("buy"),
        );
        ingest(&hub, tid, TraceStage::HedgeFired, "EURUSD", {
            let mut d = details("buy");
            d.hedge_id = Some("HDG-7".into());
            d.detail = Some("amber; lp=lp-2".into());
            d
        });

        let events = hub.get_trace(tid);
        assert_eq!(events.len(), 9, "all nine stages captured");
        // All share the trace id; seq is 0..n gap-free; timestamps strictly increase.
        let expected_stages = [
            TraceStage::PriceComputed,
            TraceStage::QuotePublished,
            TraceStage::OrderReceived,
            TraceStage::LastLook,
            TraceStage::AcceptanceDecided,
            TraceStage::RiskRouted,
            TraceStage::DealBooked,
            TraceStage::HedgeDecided,
            TraceStage::HedgeFired,
        ];
        let mut prev_ts = 0;
        for (i, ev) in events.iter().enumerate() {
            assert_eq!(ev.trace_id, tid);
            assert_eq!(ev.seq, i as u32);
            assert_eq!(ev.stage, expected_stages[i] as i32);
            assert!(ev.timestamp_ns > prev_ts, "timestamps strictly increase");
            prev_ts = ev.timestamp_ns;
            assert_eq!(ev.symbol, "EURUSD");
        }
        // Stage specifics carried through.
        assert_eq!(events[0].price, Some(1.0850));
        assert_eq!(events[1].quote_id.as_deref(), Some("Q-1"));
        assert_eq!(events[6].deal_id.as_deref(), Some("D-42"));
        assert_eq!(events[8].hedge_id.as_deref(), Some("HDG-7"));
    }

    #[test]
    fn list_traces_summarizes_newest_first_and_filters() {
        let (hub, _rx) = TraceHub::new();
        // Trace 1: EURUSD, booked, cp-a.
        let t1 = hub.mint_trace();
        ingest(&hub, t1, TraceStage::QuotePublished, "EURUSD", {
            let mut d = details("buy");
            d.counterparty = Some("cp-a".into());
            d
        });
        ingest(&hub, t1, TraceStage::DealBooked, "EURUSD", details("buy"));
        // Trace 2: GBPUSD, hedged, cp-b (later ⇒ newest).
        let t2 = hub.mint_trace();
        ingest(&hub, t2, TraceStage::QuotePublished, "GBPUSD", {
            let mut d = details("sell");
            d.counterparty = Some("cp-b".into());
            d
        });
        ingest(&hub, t2, TraceStage::HedgeFired, "GBPUSD", details("sell"));

        let all = hub.list_traces(None, None, None);
        assert_eq!(all.len(), 2);
        // Newest (t2) first.
        assert_eq!(all[0].trace_id, t2);
        assert_eq!(all[0].outcome, "hedged");
        assert_eq!(all[0].symbol, "GBPUSD");
        assert_eq!(all[0].event_count, 2);
        assert_eq!(all[0].first_stage, TraceStage::QuotePublished as i32);
        assert_eq!(all[0].last_stage, TraceStage::HedgeFired as i32);
        assert!(all[0].total_latency_ns > 0);
        assert_eq!(all[1].trace_id, t1);
        assert_eq!(all[1].outcome, "booked");

        // Symbol filter.
        let only_eur = hub.list_traces(None, Some("EURUSD"), None);
        assert_eq!(only_eur.len(), 1);
        assert_eq!(only_eur[0].trace_id, t1);
        // Counterparty filter.
        let only_cpb = hub.list_traces(None, None, Some("cp-b"));
        assert_eq!(only_cpb.len(), 1);
        assert_eq!(only_cpb[0].trace_id, t2);
        // Limit clamp.
        let limited = hub.list_traces(Some(1), None, None);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].trace_id, t2);
    }

    #[test]
    fn store_evicts_oldest_at_capacity() {
        let (hub, _rx) = TraceHub::with_capacities(3, 16, 16);
        for tid in 1..=5u64 {
            ingest(
                &hub,
                tid,
                TraceStage::QuotePublished,
                "EURUSD",
                details("buy"),
            );
        }
        // Capacity 3 ⇒ traces 1 and 2 evicted, 3/4/5 retained.
        assert_eq!(hub.evicted(), 2);
        assert!(hub.get_trace(1).is_empty());
        assert!(hub.get_trace(2).is_empty());
        assert_eq!(hub.get_trace(3).len(), 1);
        assert_eq!(hub.get_trace(5).len(), 1);
        assert_eq!(hub.list_traces(None, None, None).len(), 3);
    }

    #[test]
    fn correlation_indexes_bind_and_resolve_with_eviction() {
        let (hub, _rx) = TraceHub::with_capacities(16, 2, 16);
        hub.bind_quote("Q-1", 100);
        hub.bind_request("R-1", 100);
        hub.bind_position(7, 100);
        assert_eq!(hub.resolve_quote("Q-1"), Some(100));
        assert_eq!(hub.resolve_request("R-1"), Some(100));
        assert_eq!(hub.resolve_position(7), Some(100));
        assert_eq!(hub.resolve_quote("nope"), None);
        // Index cap 2: a third quote binding evicts the oldest ("Q-1").
        hub.bind_quote("Q-2", 200);
        hub.bind_quote("Q-3", 300);
        assert_eq!(hub.resolve_quote("Q-1"), None);
        assert_eq!(hub.resolve_quote("Q-2"), Some(200));
        assert_eq!(hub.resolve_quote("Q-3"), Some(300));
    }

    #[test]
    fn record_is_nonblocking_and_drops_on_full_offload() {
        // Tiny offload, no drain worker consuming ⇒ it fills, and `record` must
        // never block: it drops-and-counts (lossy capture, off the hot path).
        let (hub, _rx) = TraceHub::with_capacities(16, 16, 4);
        for _ in 0..100 {
            hub.record(1, TraceStage::PriceComputed, "EURUSD", details("buy"));
        }
        assert!(
            hub.dropped() > 0,
            "a size-4 offload with no drain must drop some events"
        );
    }

    #[test]
    fn capture_is_off_the_pinned_pricing_entrypoint() {
        // Guardrail 11: the pinned zero-alloc pricing entrypoint (`pricer.rs`,
        // `price_instrument`) must NEVER do trace capture — every trace call originates
        // on the async service edges (fix / rates_book), off the pricing thread. This
        // source-level guard fails if a future edit introduces any trace-hub capture
        // token into the pinned pricer. (The architectural backstop is stronger still:
        // the pinned core runs `celnet-engine`, which `celnet-server` depends on one-way,
        // so it *cannot* reference `TraceHub` at all.)
        let pricer_src = include_str!("../pricer.rs");
        for forbidden in [
            "TraceHub",
            "mint_trace",
            "bind_quote",
            "bind_request",
            "bind_position",
            "trace().record",
            "TraceDetails",
            "TraceStage",
        ] {
            assert!(
                !pricer_src.contains(forbidden),
                "pinned pricer entrypoint must not capture traces, found `{forbidden}`"
            );
        }
    }

    #[tokio::test]
    async fn record_flows_through_offload_to_store_via_worker() {
        let (hub, rx) = TraceHub::with_capacities(16, 16, 64);
        let shutdown = std::sync::Arc::new(AtomicBool::new(false));
        TraceHub::spawn_drain(hub.clone(), rx, shutdown);
        let tid = hub.mint_trace();
        hub.record(tid, TraceStage::PriceComputed, "EURUSD", {
            let mut d = details("buy");
            d.price = Some(1.10);
            d
        });
        hub.record(tid, TraceStage::QuotePublished, "EURUSD", {
            let mut d = details("buy");
            d.quote_id = Some("Q-9".into());
            d
        });
        // Give the worker a moment to drain.
        for _ in 0..50 {
            if hub.get_trace(tid).len() == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let events = hub.get_trace(tid);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].stage, TraceStage::PriceComputed as i32);
        assert_eq!(events[1].quote_id.as_deref(), Some("Q-9"));
    }
}
