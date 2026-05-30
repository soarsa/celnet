//! Structured logging/tracing setup, the human-facing error taxonomy, and the
//! **audit-log record** for the quote/trade lifecycle.
//!
//! All of this lives on the **async edge / drain side** — the hot core never
//! constructs a log line, formats a string, or builds an audit record (it pushes
//! POD samples through the SPSC ring; the drain/edge does the rest).
//!
//! Three concerns:
//! 1. [`init_json_subscriber`] / [`build_json_subscriber`] — a configurable,
//!    structured (line-delimited JSON) `tracing` subscriber.
//! 2. [`LogClass`] — a coarse, stable severity/category taxonomy for log events,
//!    distinct from the per-operation [`ErrorClass`](crate::record::ErrorClass)
//!    carried on the hot path.
//! 3. [`AuditRecord`] — a lossless, serialisable record of a quote/trade
//!    lifecycle event, committed on a *separate* path from lossy telemetry.

use std::io;

use serde::{Deserialize, Serialize};
use tracing::Level;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::time::UtcTime;

use crate::record::{ErrorClass, OpKind};

/// Coarse log taxonomy — a small, stable set of categories every Celnet log
/// event is tagged with, so operators can route/alert on category without
/// parsing free-text messages.
///
/// This is deliberately *not* the same as the hot-path
/// [`ErrorClass`](crate::record::ErrorClass): that classifies a pricing
/// *outcome* with an integer on the wire; this classifies a *log event* for
/// human/operational consumption at the edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogClass {
    /// Lifecycle / control-plane events (startup, blue-green cutover, shutdown).
    Lifecycle,
    /// A client request was rejected for a client-side reason (bad input,
    /// unknown instrument) — not our fault, no page.
    ClientError,
    /// A market-data / surface condition (stale feed, arbitrage flagged).
    MarketData,
    /// A degraded-but-serving condition (telemetry drops, backpressure).
    Degraded,
    /// An internal fault that needs attention (invariant violated, dependency
    /// down) — page-worthy.
    Fault,
    /// Security / audit-relevant access events.
    Security,
}

impl LogClass {
    /// The stable snake_case label used as the `class` field in JSON logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            LogClass::Lifecycle => "lifecycle",
            LogClass::ClientError => "client_error",
            LogClass::MarketData => "market_data",
            LogClass::Degraded => "degraded",
            LogClass::Fault => "fault",
            LogClass::Security => "security",
        }
    }

    /// The default tracing [`Level`] for this class (operators may still filter
    /// further via `EnvFilter`).
    #[must_use]
    pub const fn level(self) -> Level {
        match self {
            LogClass::Lifecycle => Level::INFO,
            LogClass::ClientError => Level::WARN,
            LogClass::MarketData => Level::WARN,
            LogClass::Degraded => Level::WARN,
            LogClass::Fault => Level::ERROR,
            LogClass::Security => Level::INFO,
        }
    }

    /// Map a hot-path [`ErrorClass`](crate::record::ErrorClass) outcome onto the
    /// log taxonomy, so the drain can log a drained sample's outcome coherently.
    #[must_use]
    pub const fn from_error_class(class: ErrorClass) -> Self {
        match class {
            ErrorClass::Ok => LogClass::Lifecycle,
            ErrorClass::InvalidInput => LogClass::ClientError,
            ErrorClass::StaleState | ErrorClass::Arbitrage => LogClass::MarketData,
            ErrorClass::NoConvergence => LogClass::Fault,
            ErrorClass::TelemetryDropped => LogClass::Degraded,
            ErrorClass::Internal => LogClass::Fault,
        }
    }
}

/// Configuration for the structured JSON subscriber.
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// The default max level when `RUST_LOG`/`filter` does not say otherwise.
    pub max_level: Level,
    /// An optional explicit filter directive (e.g. `"celnet=debug,info"`);
    /// when `None`, the `RUST_LOG` env var is honoured, defaulting to
    /// `max_level`.
    pub filter: Option<String>,
    /// Whether to include source file/line in each event (off in hot prod for
    /// volume; on in staging).
    pub with_location: bool,
    /// Whether to include the current span list with each event.
    pub with_spans: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            max_level: Level::INFO,
            filter: None,
            with_location: false,
            with_spans: true,
        }
    }
}

impl LogConfig {
    fn env_filter(&self) -> EnvFilter {
        match &self.filter {
            Some(directive) => EnvFilter::try_new(directive)
                .unwrap_or_else(|_| EnvFilter::new(self.max_level.to_string())),
            None => EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(self.max_level.to_string())),
        }
    }
}

/// Build (but do not install) a structured JSON `tracing` subscriber writing
/// line-delimited JSON to `writer`, with RFC-3339 UTC timestamps.
///
/// Returns a fully-typed subscriber the caller can install with
/// `tracing::subscriber::set_default` (scoped, e.g. in tests) or
/// [`init_json_subscriber`] (global). Using an explicit `writer` makes the
/// emitted JSON capturable for tests and routable (stdout, a file, a pipe to the
/// log shipper) in production.
pub fn build_json_subscriber<W>(
    cfg: &LogConfig,
    writer: W,
) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(cfg.with_spans)
        .with_span_list(cfg.with_spans)
        .with_timer(UtcTime::rfc_3339())
        .with_file(cfg.with_location)
        .with_line_number(cfg.with_location)
        .with_target(true)
        .with_env_filter(cfg.env_filter())
        .with_writer(writer)
        .finish()
}

/// Build and globally install the structured JSON subscriber writing to stdout.
///
/// Idempotent-ish: returns `Err` if a global subscriber was already installed
/// (the caller decides whether that is fatal). Intended to be called once at
/// process start by `celnet-server` — **never** from the hot path.
///
/// # Errors
/// Returns an error if a global default subscriber is already set.
pub fn init_json_subscriber(cfg: &LogConfig) -> Result<(), SubscriberInstallError> {
    let subscriber = build_json_subscriber(cfg, io::stdout);
    tracing::subscriber::set_global_default(subscriber).map_err(|_| SubscriberInstallError)
}

/// Returned when a global tracing subscriber is already installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriberInstallError;

impl std::fmt::Display for SubscriberInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a global tracing subscriber is already installed")
    }
}

impl std::error::Error for SubscriberInstallError {}

/// The lifecycle stage an [`AuditRecord`] captures.
///
/// These mark the legally/operationally significant transitions of a quote or
/// trade — the events that must **never** be dropped (unlike telemetry), and so
/// flow on a separate, lossless, backpressured path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStage {
    /// A request-for-quote was received from a client.
    QuoteRequested,
    /// A quote was issued to the client (with a `valid_until`).
    QuoteIssued,
    /// The client accepted the quote (last-look / idempotent).
    QuoteAccepted,
    /// The client rejected (or let lapse) the quote.
    QuoteRejected,
    /// A trade was booked into the downstream estate.
    TradeBooked,
    /// A booked trade was amended.
    TradeAmended,
    /// A booked trade was cancelled.
    TradeCancelled,
}

impl AuditStage {
    /// Stable snake_case label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AuditStage::QuoteRequested => "quote_requested",
            AuditStage::QuoteIssued => "quote_issued",
            AuditStage::QuoteAccepted => "quote_accepted",
            AuditStage::QuoteRejected => "quote_rejected",
            AuditStage::TradeBooked => "trade_booked",
            AuditStage::TradeAmended => "trade_amended",
            AuditStage::TradeCancelled => "trade_cancelled",
        }
    }
}

/// A serialisable audit record for one quote/trade-lifecycle event.
///
/// Carries enough to reconstruct the lifecycle deterministically and idempotently
/// (the `idempotency_key` dedupes retries across a blue-green cutover). The
/// monetary fields are plain `f64` domestic-premium figures (per `celnet-types`
/// conventions); identifiers are owned `String`s because this is built on the
/// non-critical edge where allocation is fine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// The lifecycle stage this record marks.
    pub stage: AuditStage,
    /// The engine request id this event correlates to (echoed on the wire and in
    /// the hot-path [`HotSample`](crate::record::HotSample)).
    pub request_id: u64,
    /// A client-supplied idempotency key, used to dedupe retries and to survive
    /// a blue-green cutover without double-booking.
    pub idempotency_key: String,
    /// The tenant / counterparty the event belongs to.
    pub tenant: String,
    /// The instrument symbol/pair the event concerns (e.g. `"EURUSD"`).
    pub instrument: String,
    /// The operation kind that produced the priced figure, when applicable.
    pub op: Option<OpKind>,
    /// The quoted/booked premium (domestic), when applicable to the stage.
    pub premium: Option<f64>,
    /// A monotonic event sequence assigned by the lossless audit sink, for total
    /// ordering and gap detection in the audit stream.
    pub sequence: u64,
    /// Free-form, vendor-neutral note (e.g. a rejection reason). Never carries
    /// PII beyond the counterparty identifier above.
    pub note: Option<String>,
}

impl AuditRecord {
    /// Construct a minimal audit record for `stage` on `request_id`.
    #[must_use]
    pub fn new(
        stage: AuditStage,
        request_id: u64,
        idempotency_key: impl Into<String>,
        tenant: impl Into<String>,
        instrument: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            request_id,
            idempotency_key: idempotency_key.into(),
            tenant: tenant.into(),
            instrument: instrument.into(),
            op: None,
            premium: None,
            sequence: 0,
            note: None,
        }
    }

    /// Attach the priced premium and the operation kind that produced it.
    #[must_use]
    pub fn with_priced(mut self, op: OpKind, premium: f64) -> Self {
        self.op = Some(op);
        self.premium = Some(premium);
        self
    }

    /// Attach a free-form note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Emit this record as a structured `tracing` event under the `Security`
    /// log class (audit events are security-relevant). The drain/edge calls this
    /// after the lossless sink has assigned `sequence` and committed it.
    pub fn emit(&self) {
        tracing::info!(
            class = LogClass::Security.label(),
            audit_stage = self.stage.label(),
            request_id = self.request_id,
            idempotency_key = %self.idempotency_key,
            tenant = %self.tenant,
            instrument = %self.instrument,
            op = self.op.map(OpKind::label),
            premium = self.premium,
            sequence = self.sequence,
            note = self.note.as_deref(),
            "audit"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logclass_maps_from_error_class() {
        assert_eq!(
            LogClass::from_error_class(ErrorClass::InvalidInput),
            LogClass::ClientError
        );
        assert_eq!(
            LogClass::from_error_class(ErrorClass::TelemetryDropped),
            LogClass::Degraded
        );
        assert_eq!(
            LogClass::from_error_class(ErrorClass::Internal),
            LogClass::Fault
        );
        assert_eq!(LogClass::ClientError.level(), Level::WARN);
        assert_eq!(LogClass::Fault.level(), Level::ERROR);
    }

    #[test]
    fn audit_record_serializes_roundtrip() {
        let rec = AuditRecord::new(AuditStage::QuoteIssued, 99, "idem-1", "tenant-a", "EURUSD")
            .with_priced(OpKind::VanillaPrice, 0.0123)
            .with_note("indicative");
        let json = serde_json::to_string(&rec).expect("serializes");
        assert!(json.contains("quote_issued"));
        assert!(json.contains("EURUSD"));
        let back: AuditRecord = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, rec);
        assert_eq!(back.op, Some(OpKind::VanillaPrice));
    }

    #[test]
    fn audit_stage_labels_are_distinct() {
        let stages = [
            AuditStage::QuoteRequested,
            AuditStage::QuoteIssued,
            AuditStage::QuoteAccepted,
            AuditStage::QuoteRejected,
            AuditStage::TradeBooked,
            AuditStage::TradeAmended,
            AuditStage::TradeCancelled,
        ];
        let mut labels: Vec<&str> = stages.iter().map(|s| s.label()).collect();
        labels.sort_unstable();
        let n = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), n, "stage labels must be unique");
    }
}
