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

use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::time::UtcTime;
use tracing_subscriber::layer::{Context, Filter, SubscriberExt};
use tracing_subscriber::registry::{LookupSpan, Registry};

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
    // --- order-lifecycle domain classes (route to the split file sinks) -------
    // These tag the async-edge order-lifecycle seams so operators get a per-domain
    // file (orders/executions/pricing) in addition to the combined log. The
    // finer order classes (`Risk`/`Hedge`/`Amend`/`Transfer`) all ROUTE to the
    // orders sink (see [`ORDER_SINK_CLASSES`]) but keep a distinct label so a
    // grep/JQ filter can still isolate them within `orders.log`.
    /// An incoming order / RFQ intake and its acceptance-rule / last-look
    /// decision. Routes to `orders.log`.
    Order,
    /// A risk-routing decision (the routed risk-book, the position, the routing
    /// context). Routes to `orders.log`.
    Risk,
    /// A hedging decision (internalise vs back-to-back, edge, warehouse-cap
    /// utilisation). Routes to `orders.log`.
    Hedge,
    /// An order/quote/deal amend, cancel or correction. Routes to `orders.log`.
    Amend,
    /// A risk-transfer (initiated/accepted/rejected, from/to book). Routes to
    /// `orders.log`.
    Transfer,
    /// A fill / booking — the execution report emitted and the booked deal.
    /// Routes to `executions.log`.
    Execution,
    /// Quote construction + outbound on the async edge (priced rate, PV, PV01,
    /// two-way bid/offer). Routes to `pricing.log`. **Never** emitted on the
    /// pinned pricing core.
    Pricing,
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
            LogClass::Order => "order",
            LogClass::Risk => "risk",
            LogClass::Hedge => "hedge",
            LogClass::Amend => "amend",
            LogClass::Transfer => "transfer",
            LogClass::Execution => "execution",
            LogClass::Pricing => "pricing",
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
            // Order-lifecycle classes default to INFO (normal flow); a rejection
            // / breach / hold is raised to WARN at the call site (`warn!`), so the
            // default here is the happy-path level.
            LogClass::Order
            | LogClass::Risk
            | LogClass::Hedge
            | LogClass::Amend
            | LogClass::Transfer
            | LogClass::Execution
            | LogClass::Pricing => Level::INFO,
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

/// The `class` labels routed to the **orders** file sink: the incoming-order
/// intake plus every finer order-lifecycle domain (risk-routing, hedging,
/// amends, transfers). Derived from [`LogClass::label`] so the routing can never
/// drift from the taxonomy.
pub const ORDER_SINK_CLASSES: &[&str] = &[
    LogClass::Order.label(),
    LogClass::Risk.label(),
    LogClass::Hedge.label(),
    LogClass::Amend.label(),
    LogClass::Transfer.label(),
];

/// The `class` labels routed to the **executions** file sink.
pub const EXECUTION_SINK_CLASSES: &[&str] = &[LogClass::Execution.label()];

/// The `class` labels routed to the **pricing** file sink.
pub const PRICING_SINK_CLASSES: &[&str] = &[LogClass::Pricing.label()];

/// The `class` labels routed to the **security** file sink — the per-decision
/// entitlement audit + the lossless trade-lifecycle audit. This is the highest-
/// volume class in a live edge (every gated RPC emits one), so giving it a
/// dedicated file keeps the combined log readable (it is EXCLUDED from combined
/// — see [`ClassFilter::combined`]).
pub const SECURITY_SINK_CLASSES: &[&str] = &[LogClass::Security.label()];

/// Every class that has its OWN dedicated file sink (orders ∪ executions ∪
/// pricing ∪ security). The combined stdout sink EXCLUDES exactly these so it
/// carries only the residue — startup/lifecycle, faults, degraded/market-data
/// and any unclassed event — instead of drowning in entitlement-audit volume.
pub const ROUTED_SINK_CLASSES: &[&str] = &[
    LogClass::Order.label(),
    LogClass::Risk.label(),
    LogClass::Hedge.label(),
    LogClass::Amend.label(),
    LogClass::Transfer.label(),
    LogClass::Execution.label(),
    LogClass::Pricing.label(),
    LogClass::Security.label(),
];

/// A per-layer [`Filter`] that routes an event by its `class` field value. In
/// **include** mode (`negate=false`) it admits an event iff its `class` is in
/// `allowed`; in **exclude** mode (`negate=true`) it admits iff the `class` is
/// **not** in `allowed` (an unclassed event is admitted). This fans the single
/// event stream out to the domain files — the orders layer carries
/// [`ClassFilter::orders`], executions [`ClassFilter::executions`], pricing
/// [`ClassFilter::pricing`], security [`ClassFilter::security`] — while the
/// combined stdout sink carries [`ClassFilter::combined`] (exclude the four
/// routed classes) so it stays lean instead of drowning in entitlement audit.
///
/// The decision is a **field-value** match, not a metadata/target match, so it
/// runs in [`Filter::event_enabled`] (which receives the `&Event` and hence the
/// recorded fields) rather than [`Filter::enabled`] (metadata only). `enabled`
/// therefore returns `true` unconditionally — the class is only knowable once the
/// event's fields are visited. This all runs on the async edge; the pinned core
/// never constructs an event.
#[derive(Debug, Clone, Copy)]
pub struct ClassFilter {
    allowed: &'static [&'static str],
    negate: bool,
}

impl ClassFilter {
    /// An **include** filter: admit exactly the classes in `allowed`.
    #[must_use]
    pub const fn new(allowed: &'static [&'static str]) -> Self {
        Self {
            allowed,
            negate: false,
        }
    }

    /// An **exclude** filter: admit every event whose `class` is NOT in `allowed`
    /// (including an unclassed event).
    #[must_use]
    pub const fn excluding(allowed: &'static [&'static str]) -> Self {
        Self {
            allowed,
            negate: true,
        }
    }

    /// The orders-sink filter (`Order`/`Risk`/`Hedge`/`Amend`/`Transfer`).
    #[must_use]
    pub const fn orders() -> Self {
        Self::new(ORDER_SINK_CLASSES)
    }

    /// The executions-sink filter (`Execution`).
    #[must_use]
    pub const fn executions() -> Self {
        Self::new(EXECUTION_SINK_CLASSES)
    }

    /// The pricing-sink filter (`Pricing`).
    #[must_use]
    pub const fn pricing() -> Self {
        Self::new(PRICING_SINK_CLASSES)
    }

    /// The security-sink filter (`Security`).
    #[must_use]
    pub const fn security() -> Self {
        Self::new(SECURITY_SINK_CLASSES)
    }

    /// The combined-sink filter: admit everything EXCEPT the classes that have
    /// their own file ([`ROUTED_SINK_CLASSES`]) — so the combined log carries the
    /// residue (startup/lifecycle, faults, degraded, unclassed) and not the
    /// high-volume order/execution/pricing/security streams.
    #[must_use]
    pub const fn combined() -> Self {
        Self::excluding(ROUTED_SINK_CLASSES)
    }

    /// Whether an event tagged with `class` is admitted by this filter — the pure
    /// routing predicate, exposed for unit testing without a subscriber context.
    #[must_use]
    pub fn admits(&self, class: &str) -> bool {
        self.allowed.contains(&class) != self.negate
    }
}

/// Visits an event's fields looking for `class`, recording whether a `class`
/// field was present AND its value is in the allow-list. `class` is always
/// recorded as a `&str` (`LogClass::label` returns `&'static str`), so
/// [`Visit::record_str`] is the match arm; the `record_debug` fallback is inert.
struct ClassFieldVisitor<'a> {
    allowed: &'a [&'static str],
    found_in_allowed: bool,
}

impl Visit for ClassFieldVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "class" && self.allowed.contains(&value) {
            self.found_in_allowed = true;
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {
        // `class` is a `&str`; other fields are irrelevant to routing.
    }
}

impl<S> Filter<S> for ClassFilter {
    fn enabled(&self, _meta: &Metadata<'_>, _cx: &Context<'_, S>) -> bool {
        // The routing key is a field VALUE, not metadata — decide in
        // `event_enabled` once the fields are available.
        true
    }

    fn event_enabled(&self, event: &Event<'_>, _cx: &Context<'_, S>) -> bool {
        let mut visitor = ClassFieldVisitor {
            allowed: self.allowed,
            found_in_allowed: false,
        };
        event.record(&mut visitor);
        // include (negate=false): admit iff the class matched the allow-list.
        // exclude (negate=true): admit iff it did NOT (unclassed ⇒ admitted).
        visitor.found_in_allowed != self.negate
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
    /// The directory the split domain sinks (`orders.log` / `executions.log` /
    /// `pricing.log`) roll in. `None` ⇒ no split sinks (combined stdout only) —
    /// the posture for tests and the demo edge; the server deploy points this at
    /// `celnet_log_dir`. The directory is created if it does not exist.
    pub dir: Option<PathBuf>,
    /// Retention cap for the rolling domain sinks: keep at most this many rolled
    /// files per sink (`tracing_appender`'s `max_log_files`). `None` ⇒ no
    /// in-process cap (a `logrotate` policy governs retention instead).
    pub max_files: Option<usize>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            max_level: Level::INFO,
            filter: None,
            with_location: false,
            with_spans: true,
            dir: None,
            max_files: None,
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

/// Build one JSON-formatting `tracing` layer with the same shape as
/// [`build_json_subscriber`] (flattened event, RFC-3339 UTC time, target on,
/// optional file/line + span list), writing to `writer`, optionally gated by a
/// per-layer [`ClassFilter`]. Boxed so a heterogeneous stack of these (combined
/// stdout + the three domain files) composes into one `Vec<Box<dyn Layer>>`.
fn json_layer<S, W>(
    cfg: &LogConfig,
    writer: W,
    class_filter: Option<ClassFilter>,
) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(cfg.with_spans)
        .with_span_list(cfg.with_spans)
        .with_timer(UtcTime::rfc_3339())
        .with_file(cfg.with_location)
        .with_line_number(cfg.with_location)
        .with_target(true)
        .with_writer(writer);
    match class_filter {
        Some(filter) => layer.with_filter(filter).boxed(),
        None => layer.boxed(),
    }
}

/// Construct a daily-rolling file appender for one domain sink
/// (`{prefix}.log.YYYY-MM-DD` in `dir`), optionally capping retention to
/// `max_files` rolled files.
fn rolling_appender(
    dir: &Path,
    prefix: &str,
    max_files: Option<usize>,
) -> Result<RollingFileAppender, SplitLogError> {
    let mut builder = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(prefix)
        .filename_suffix("log");
    if let Some(n) = max_files {
        builder = builder.max_log_files(n);
    }
    builder.build(dir).map_err(SplitLogError::Appender)
}

/// Build (but do not install) the **multi-sink** subscriber: a lean combined
/// stdout sink plus four daily-rolling, non-blocking JSON file sinks routed by
/// the event `class` field:
///
/// * `orders.log` — [`ClassFilter::orders`] (`order`/`risk`/`hedge`/`amend`/`transfer`),
/// * `executions.log` — [`ClassFilter::executions`] (`execution`),
/// * `pricing.log` — [`ClassFilter::pricing`] (`pricing`),
/// * `security.log` — [`ClassFilter::security`] (`security` — the per-decision
///   entitlement audit + trade-lifecycle audit, the highest-volume class),
/// * combined stdout — [`ClassFilter::combined`], which EXCLUDES the four routed
///   classes above, so `celnet-server.out.log` carries only the residue
///   (startup/lifecycle, faults, degraded, unclassed) and is readable again.
///
/// A single global [`EnvFilter`] (from `RUST_LOG`/`cfg.filter`) is composed on
/// top so verbosity still tunes globally; the per-layer class filters then fan
/// the surviving events out to the domain files.
///
/// Returns the subscriber **and** the non-blocking [`WorkerGuard`]s — one per
/// file sink. **The caller MUST keep the guards alive** for as long as the
/// subscriber is installed: dropping a guard stops that file's worker thread and
/// silently ends its writes. Requires `cfg.dir` to be set.
///
/// # Errors
/// [`SplitLogError::NoDir`] if `cfg.dir` is `None`; [`SplitLogError::Io`] if the
/// log directory cannot be created; [`SplitLogError::Appender`] if a rolling
/// appender cannot be initialised.
pub fn build_split_subscriber(
    cfg: &LogConfig,
) -> Result<(impl tracing::Subscriber + Send + Sync, Vec<WorkerGuard>), SplitLogError> {
    let dir = cfg.dir.as_deref().ok_or(SplitLogError::NoDir)?;
    std::fs::create_dir_all(dir).map_err(SplitLogError::Io)?;

    let mut guards = Vec::with_capacity(4);
    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::with_capacity(5);

    // Combined sink: to stdout (celnetctl's `.out.log`), but EXCLUDING the four
    // classes that have their own file (orders/executions/pricing/security) so the
    // combined log carries only the residue — startup/lifecycle, faults, degraded,
    // unclassed — instead of drowning in entitlement-audit volume.
    layers.push(json_layer::<Registry, _>(
        cfg,
        io::stdout,
        Some(ClassFilter::combined()),
    ));

    // Domain sinks: daily-rolling non-blocking files, each class-filtered.
    for (prefix, filter) in [
        ("orders", ClassFilter::orders()),
        ("executions", ClassFilter::executions()),
        ("pricing", ClassFilter::pricing()),
        ("security", ClassFilter::security()),
    ] {
        let appender = rolling_appender(dir, prefix, cfg.max_files)?;
        let (non_blocking, guard) = tracing_appender::non_blocking(appender);
        guards.push(guard);
        layers.push(json_layer::<Registry, _>(cfg, non_blocking, Some(filter)));
    }

    // The domain-layer Vec is `Layer<Registry>`, so it must attach directly to
    // `Registry`; the global `EnvFilter` then composes on TOP as the outermost
    // layer — position-independent as a global gate (an event it disables is
    // disabled for every sink), it just cannot sit UNDER the `Layer<Registry>`
    // Vec without changing the Vec's `S`.
    let subscriber = Registry::default().with(layers).with(cfg.env_filter());
    Ok((subscriber, guards))
}

/// Build and globally install the multi-sink subscriber (lean combined stdout +
/// `orders.log`/`executions.log`/`pricing.log`/`security.log`). Intended to be
/// called once at process start by `celnet-server`; **never** from the hot path.
///
/// Returns the non-blocking [`WorkerGuard`]s — the caller **must** store them for
/// the process lifetime (a dropped guard silently stops that file's writer). A
/// good pattern is to bind them to a `let _guards = ...;` in `main` that lives
/// until shutdown.
///
/// # Errors
/// [`SplitLogError::AlreadyInstalled`] if a global subscriber is already set, or
/// any error [`build_split_subscriber`] returns.
pub fn install_split(cfg: &LogConfig) -> Result<Vec<WorkerGuard>, SplitLogError> {
    let (subscriber, guards) = build_split_subscriber(cfg)?;
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_| SplitLogError::AlreadyInstalled)?;
    Ok(guards)
}

/// Failure modes of the multi-sink subscriber install.
#[derive(Debug)]
pub enum SplitLogError {
    /// `cfg.dir` was `None` — the split sinks need a target directory.
    NoDir,
    /// The log directory could not be created.
    Io(io::Error),
    /// A rolling file appender could not be initialised.
    Appender(InitError),
    /// A global tracing subscriber was already installed.
    AlreadyInstalled,
}

impl std::fmt::Display for SplitLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitLogError::NoDir => {
                f.write_str("no log directory configured for the split sinks (LogConfig.dir)")
            }
            SplitLogError::Io(e) => write!(f, "could not create the log directory: {e}"),
            SplitLogError::Appender(e) => write!(f, "could not initialise a rolling appender: {e}"),
            SplitLogError::AlreadyInstalled => {
                f.write_str("a global tracing subscriber is already installed")
            }
        }
    }
}

impl std::error::Error for SplitLogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SplitLogError::Io(e) => Some(e),
            SplitLogError::Appender(e) => Some(e),
            SplitLogError::NoDir | SplitLogError::AlreadyInstalled => None,
        }
    }
}

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

    /// All domain (`Order`/`Risk`/`Hedge`/`Amend`/`Transfer`/`Execution`/`Pricing`)
    /// and existing labels are pairwise distinct — the routing keys must not
    /// collide.
    #[test]
    fn logclass_labels_are_distinct() {
        let classes = [
            LogClass::Lifecycle,
            LogClass::ClientError,
            LogClass::MarketData,
            LogClass::Degraded,
            LogClass::Fault,
            LogClass::Security,
            LogClass::Order,
            LogClass::Risk,
            LogClass::Hedge,
            LogClass::Amend,
            LogClass::Transfer,
            LogClass::Execution,
            LogClass::Pricing,
        ];
        let mut labels: Vec<&str> = classes.iter().map(|c| c.label()).collect();
        let n = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), n, "class labels must be unique");
    }

    /// The pure routing predicate: each sink's filter admits exactly its own
    /// classes and rejects the others.
    #[test]
    fn class_filter_admits_only_its_own_classes() {
        let orders = ClassFilter::orders();
        assert!(orders.admits("order"));
        assert!(orders.admits("risk"));
        assert!(orders.admits("hedge"));
        assert!(orders.admits("amend"));
        assert!(orders.admits("transfer"));
        assert!(!orders.admits("execution"));
        assert!(!orders.admits("pricing"));
        assert!(!orders.admits("lifecycle"));

        let execs = ClassFilter::executions();
        assert!(execs.admits("execution"));
        assert!(!execs.admits("order"));
        assert!(!execs.admits("pricing"));

        let pricing = ClassFilter::pricing();
        assert!(pricing.admits("pricing"));
        assert!(!pricing.admits("order"));
        assert!(!pricing.admits("execution"));

        let security = ClassFilter::security();
        assert!(security.admits("security"));
        assert!(!security.admits("order"));
        assert!(!security.admits("pricing"));

        // The combined (exclude) filter admits everything EXCEPT the routed
        // classes — so the four dedicated streams are kept out, but residue
        // classes (lifecycle/fault/…) stay in the combined log.
        let combined = ClassFilter::combined();
        assert!(!combined.admits("order"));
        assert!(!combined.admits("risk"));
        assert!(!combined.admits("execution"));
        assert!(!combined.admits("pricing"));
        assert!(!combined.admits("security"));
        assert!(combined.admits("lifecycle"));
        assert!(combined.admits("fault"));
        assert!(combined.admits("degraded"));
    }

    /// An in-memory `MakeWriter` capturing everything written, for routing asserts.
    #[derive(Clone, Default)]
    struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl CaptureWriter {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).expect("utf8")
        }
    }

    impl io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// End-to-end routing over a real registry: a `class`-tagged event lands in
    /// exactly its domain sink (orders/executions/pricing/security); the combined
    /// sink EXCLUDES those four routed classes but still carries residue
    /// (lifecycle/fault) and unclassed events.
    #[test]
    fn class_routed_layers_fan_events_to_the_right_sink() {
        let combined = CaptureWriter::default();
        let orders = CaptureWriter::default();
        let execs = CaptureWriter::default();
        let pricing = CaptureWriter::default();
        let security = CaptureWriter::default();

        let mk = |w: CaptureWriter, filter: Option<ClassFilter>| {
            json_layer::<Registry, _>(&LogConfig::default(), w, filter)
        };
        let subscriber = Registry::default()
            .with(vec![
                mk(combined.clone(), Some(ClassFilter::combined())),
                mk(orders.clone(), Some(ClassFilter::orders())),
                mk(execs.clone(), Some(ClassFilter::executions())),
                mk(pricing.clone(), Some(ClassFilter::pricing())),
                mk(security.clone(), Some(ClassFilter::security())),
            ])
            .with(EnvFilter::new("info"));

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(class = LogClass::Order.label(), "incoming_order");
            tracing::info!(class = LogClass::Risk.label(), "routed_book");
            tracing::info!(class = LogClass::Execution.label(), "fill_report");
            tracing::info!(class = LogClass::Pricing.label(), "priced_quote");
            tracing::info!(class = LogClass::Security.label(), "entitlement_decision");
            tracing::info!(class = LogClass::Lifecycle.label(), "startup_event");
            tracing::info!("unclassed_event");
        });

        let o = orders.contents();
        assert!(o.contains("incoming_order"), "orders sink gets Order: {o}");
        assert!(o.contains("routed_book"), "orders sink gets Risk: {o}");
        assert!(!o.contains("fill_report"), "orders sink excludes Execution");
        assert!(!o.contains("priced_quote"), "orders sink excludes Pricing");
        assert!(
            !o.contains("entitlement_decision"),
            "orders sink excludes Security"
        );
        assert!(
            !o.contains("unclassed_event"),
            "orders sink excludes unclassed"
        );

        let e = execs.contents();
        assert!(
            e.contains("fill_report"),
            "executions sink gets Execution: {e}"
        );
        assert!(
            !e.contains("incoming_order"),
            "executions sink excludes Order"
        );
        assert!(
            !e.contains("priced_quote"),
            "executions sink excludes Pricing"
        );

        let p = pricing.contents();
        assert!(p.contains("priced_quote"), "pricing sink gets Pricing: {p}");
        assert!(!p.contains("incoming_order"), "pricing sink excludes Order");
        assert!(
            !p.contains("fill_report"),
            "pricing sink excludes Execution"
        );

        let s = security.contents();
        assert!(
            s.contains("entitlement_decision"),
            "security sink gets Security: {s}"
        );
        assert!(
            !s.contains("incoming_order"),
            "security sink excludes Order"
        );
        assert!(
            !s.contains("startup_event"),
            "security sink excludes Lifecycle"
        );

        // The combined sink EXCLUDES the four routed classes but KEEPS residue +
        // unclassed — the whole point of the exclude filter (a lean combined log).
        let c = combined.contents();
        assert!(
            c.contains("startup_event"),
            "combined keeps Lifecycle residue: {c}"
        );
        assert!(
            c.contains("unclassed_event"),
            "combined keeps unclassed: {c}"
        );
        for excluded in [
            "incoming_order",
            "routed_book",
            "fill_report",
            "priced_quote",
            "entitlement_decision",
        ] {
            assert!(
                !c.contains(excluded),
                "combined EXCLUDES routed class (found {excluded}): {c}"
            );
        }
    }

    /// The split subscriber builds with all sinks, returns one guard per file
    /// sink (orders/executions/pricing/security), and its domain files actually
    /// receive their events.
    #[test]
    fn split_subscriber_builds_and_writes_domain_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = LogConfig {
            filter: Some("info".to_string()),
            dir: Some(dir.path().to_path_buf()),
            max_files: Some(3),
            ..Default::default()
        };
        let (subscriber, guards) = build_split_subscriber(&cfg).expect("subscriber builds");
        assert_eq!(guards.len(), 4, "one worker guard per file sink");

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(class = LogClass::Order.label(), "o_evt");
            tracing::info!(class = LogClass::Execution.label(), "e_evt");
            tracing::info!(class = LogClass::Pricing.label(), "p_evt");
            tracing::info!(class = LogClass::Security.label(), "s_evt");
        });
        // Dropping the guards joins/flushes the non-blocking workers.
        drop(guards);

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read log dir")
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        // `tracing-appender` names rolled files `{prefix}.{date}.{suffix}`, e.g.
        // `orders.2026-08-06.log` — a `logrotate`/glob-friendly `orders.*.log`.
        let has = |prefix: &str| {
            names
                .iter()
                .any(|n| n.starts_with(prefix) && n.ends_with(".log"))
        };
        assert!(has("orders."), "orders.*.log created: {names:?}");
        assert!(has("executions."), "executions.*.log created: {names:?}");
        assert!(has("pricing."), "pricing.*.log created: {names:?}");
        assert!(has("security."), "security.*.log created: {names:?}");

        let read = |prefix: &str| -> String {
            let name = names
                .iter()
                .find(|n| n.starts_with(prefix) && n.ends_with(".log"))
                .unwrap();
            std::fs::read_to_string(dir.path().join(name)).unwrap()
        };
        assert!(read("orders.").contains("o_evt"));
        assert!(read("executions.").contains("e_evt"));
        assert!(read("pricing.").contains("p_evt"));
        assert!(read("security.").contains("s_evt"));
    }

    /// `build_split_subscriber` refuses to build without a target directory.
    #[test]
    fn split_subscriber_requires_a_dir() {
        let err = build_split_subscriber(&LogConfig::default())
            .err()
            .expect("no dir must fail");
        assert!(matches!(err, SplitLogError::NoDir));
    }
}
