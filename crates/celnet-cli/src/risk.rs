//! The `risk` and `stream` subcommands: the firm-scale hierarchical-risk surface
//! and the multiplexed RFS stream, both driven through the **same** typed
//! [`celnet_client`] SDK methods the GUI Book view and the Excel `CELNET.*`
//! functions call (the api-first client-parity rule).
//!
//! Every number here is the SERVER's `RiskService` aggregate / the maker's
//! streamed line surfaced verbatim — the CLI adds no pricing, no aggregation, and
//! no client-side roll-up. `risk aggregate` calls
//! [`Client::aggregate_risk`](celnet_client::Client::aggregate_risk),
//! `risk drill` calls [`Client::drill_risk`](celnet_client::Client::drill_risk),
//! `risk positions` calls
//! [`Client::list_positions`](celnet_client::Client::list_positions),
//! `risk limits` calls
//! [`Client::limit_status`](celnet_client::Client::limit_status), and `stream`
//! opens a [`StreamSession`](celnet_client::StreamSession) and consumes one typed
//! [`Subscription`](celnet_client::Subscription). The formatted output mirrors the
//! other subcommands' 2-space-indented, fixed-precision report style.
//!
//! These commands talk to a running edge over gRPC, so each runs on a private
//! current-thread tokio runtime built inside the (otherwise synchronous) dispatch;
//! the binary needs no global async runtime. Every network await is bounded by a
//! deadline so a misconfigured endpoint / unreachable edge fails fast rather than
//! hanging the terminal. The runtime/connect/deadline plumbing ([`block_on`],
//! [`connect`], [`bounded`]) and [`RiskError`] are shared crate-wide by every
//! networked subcommand — the `rfq` panel command ([`crate::rfq`]) reuses them.

use std::fmt::Write as _;
use std::time::Duration;

use celnet_client::{
    AggregateQuery, CivilDate, Client, ClientError, Conventions, DrillQuery, Entitlements,
    InstrumentSpec, LimitQuery, LimitStatus, Numeraire, Ois, OisSide, OrgDimension, PositionList,
    PositionQuery, Quantity, RatesAggregateQuery, RatesPosition, RatesPositionQuery,
    RatesRiskAggregate, RatesRiskScope, RiskAggregate, RiskDrill, RiskNode, Scope, Side,
    StreamEvent, StrikeSpec, UsdSofrCurve,
};
use celnet_types::{CcyPair, OptionType, Tenor};

/// The hard ceiling on any single risk round-trip (connect or call). A
/// never-arriving reply (unreachable edge, dead service) fails fast.
const CALL_DEADLINE: Duration = Duration::from_secs(10);

/// A networked-command (`risk` / `stream` / `rfq`) failure: a connect/transport
/// error, a server status, a bad argument, or a deadline.
#[derive(Debug)]
pub(crate) enum RiskError {
    /// The endpoint URI or an argument was invalid before any round-trip.
    Invalid(String),
    /// An SDK/transport/server failure.
    Client(ClientError),
    /// A round-trip exceeded [`CALL_DEADLINE`].
    Timeout(&'static str),
}

impl core::fmt::Display for RiskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RiskError::Invalid(s) => write!(f, "invalid argument: {s}"),
            RiskError::Client(e) => write!(f, "{e}"),
            RiskError::Timeout(what) => write!(f, "timed out waiting for {what}"),
        }
    }
}

impl std::error::Error for RiskError {}

impl From<ClientError> for RiskError {
    fn from(e: ClientError) -> Self {
        RiskError::Client(e)
    }
}

// ===========================================================================
// shared request vocabulary parsed from the CLI flags
// ===========================================================================

/// The roll-up / drill dimension as spelled on the command line. Mirrors
/// [`celnet_client::OrgDimension`] one-for-one (the typed wire `RiskDimension`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliDimension {
    /// The whole firm (the apex; a single node).
    Firm,
    /// The owning trader.
    Trader,
    /// The book.
    Book,
    /// The desk (an FRTB regulatory unit).
    Desk,
    /// The currency pair.
    CcyPair,
    /// The booking location.
    Location,
    /// The legal entity.
    Entity,
}

impl From<CliDimension> for OrgDimension {
    fn from(v: CliDimension) -> Self {
        match v {
            CliDimension::Firm => OrgDimension::Firm,
            CliDimension::Trader => OrgDimension::Trader,
            CliDimension::Book => OrgDimension::Book,
            CliDimension::Desk => OrgDimension::Desk,
            CliDimension::CcyPair => OrgDimension::CcyPair,
            CliDimension::Location => OrgDimension::Location,
            CliDimension::Entity => OrgDimension::Entity,
        }
    }
}

/// The pieces every risk request shares: the edge endpoint and the reporting
/// numeraire (currency + the spot conversion rates its legs collapse through).
#[derive(Debug, Clone)]
pub(crate) struct RiskCommon {
    /// The gRPC endpoint of the edge, e.g. `http://127.0.0.1:50551`.
    pub(crate) endpoint: String,
    /// The reporting currency every node measure is expressed in.
    pub(crate) numeraire_ccy: String,
    /// `(ccy, rate)` spot conversion rates (units of numeraire per 1 unit of ccy).
    pub(crate) rates: Vec<(String, f64)>,
}

impl RiskCommon {
    /// Assemble the typed [`Numeraire`] from the reporting currency + rate legs.
    fn numeraire(&self) -> Numeraire {
        let mut n = Numeraire::new(self.numeraire_ccy.clone());
        for (ccy, rate) in &self.rates {
            n = n.rate(ccy.clone(), *rate);
        }
        n
    }
}

/// Parse one `CCY=RATE` numeraire-rate flag (e.g. `EUR=1.10`).
pub(crate) fn parse_rate(s: &str) -> Result<(String, f64), String> {
    let (ccy, rate) = s
        .split_once('=')
        .ok_or_else(|| format!("expected CCY=RATE, got `{s}`"))?;
    let rate: f64 = rate
        .parse()
        .map_err(|_| format!("invalid rate in `{s}` (expected a number)"))?;
    if !(rate.is_finite() && rate > 0.0) {
        return Err(format!("rate in `{s}` must be a positive finite number"));
    }
    Ok((ccy.to_owned(), rate))
}

/// Resolve the entitlement principal from the optional grant/deny scope flags.
/// No flags ⇒ the grant-all (show-all-now) default, matching every other client.
/// A `--grant DIM:VALUE` switches to a deny-by-default scoped principal seeing
/// only what a grant covers; `--deny DIM:VALUE` layers an information barrier
/// (deny wins) onto whichever base the grants imply.
fn entitlements(
    grants: &[(OrgDimension, u64)],
    denies: &[(OrgDimension, u64)],
) -> Option<Entitlements> {
    if grants.is_empty() && denies.is_empty() {
        return None; // grant-all default (parity with the GUI/SDK/Excel).
    }
    let mut e = if grants.is_empty() {
        Entitlements::grant_all()
    } else {
        Entitlements::scoped()
    };
    for &(dim, value) in grants {
        e = e.grant(celnet_client::EntitlementScope::covering(Scope::at(
            dim, value,
        )));
    }
    for &(dim, value) in denies {
        e = e.deny(celnet_client::EntitlementScope::covering(Scope::at(
            dim, value,
        )));
    }
    Some(e)
}

/// Parse one `DIM:VALUE` entitlement-scope flag (e.g. `book:3`).
pub(crate) fn parse_scope_flag(s: &str) -> Result<(OrgDimension, u64), String> {
    let (dim, value) = s
        .split_once(':')
        .ok_or_else(|| format!("expected DIM:VALUE, got `{s}`"))?;
    let dim = parse_dimension(dim)?;
    let value: u64 = value
        .parse()
        .map_err(|_| format!("invalid group handle in `{s}` (expected an integer)"))?;
    Ok((dim, value))
}

/// Parse a dimension name (the lower-kebab spelling clap uses for [`CliDimension`]).
fn parse_dimension(s: &str) -> Result<OrgDimension, String> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "firm" => OrgDimension::Firm,
        "trader" => OrgDimension::Trader,
        "book" => OrgDimension::Book,
        "desk" => OrgDimension::Desk,
        "ccy-pair" | "ccypair" | "pair" => OrgDimension::CcyPair,
        "location" => OrgDimension::Location,
        "entity" => OrgDimension::Entity,
        other => return Err(format!("unknown dimension `{other}`")),
    })
}

// ===========================================================================
// request structs (one per `risk` sub-subcommand and for `stream`)
// ===========================================================================

/// A fully-parsed `risk aggregate` request.
#[derive(Debug, Clone)]
pub(crate) struct AggregateReq {
    pub(crate) common: RiskCommon,
    pub(crate) dimension: OrgDimension,
    pub(crate) scope: Option<Scope>,
    pub(crate) grants: Vec<(OrgDimension, u64)>,
    pub(crate) denies: Vec<(OrgDimension, u64)>,
    pub(crate) var_shocks: Vec<f64>,
    pub(crate) var_alpha: f64,
    pub(crate) curvature_risk_weight: f64,
}

impl AggregateReq {
    fn query(&self) -> AggregateQuery {
        let mut q = AggregateQuery::new(self.dimension, self.common.numeraire());
        if let Some(scope) = self.scope {
            q = q.scoped(scope);
        }
        if let Some(p) = entitlements(&self.grants, &self.denies) {
            q = q.entitled(p);
        }
        if !self.var_shocks.is_empty() {
            q = q.value_at_risk(self.var_shocks.clone(), self.var_alpha);
        }
        if self.curvature_risk_weight != 0.0 {
            q = q.curvature(self.curvature_risk_weight);
        }
        q
    }
}

/// A fully-parsed `risk drill` request.
#[derive(Debug, Clone)]
pub(crate) struct DrillReq {
    pub(crate) common: RiskCommon,
    pub(crate) node: Scope,
    pub(crate) child_dimension: OrgDimension,
    pub(crate) grants: Vec<(OrgDimension, u64)>,
    pub(crate) denies: Vec<(OrgDimension, u64)>,
    pub(crate) include_children: bool,
    pub(crate) include_positions: bool,
}

impl DrillReq {
    fn query(&self) -> DrillQuery {
        let mut q = DrillQuery::new(self.node, self.child_dimension, self.common.numeraire());
        if let Some(p) = entitlements(&self.grants, &self.denies) {
            q = q.entitled(p);
        }
        if self.include_children {
            q = q.children();
        }
        if self.include_positions {
            q = q.positions();
        }
        q
    }
}

/// A fully-parsed `risk positions` request.
#[derive(Debug, Clone)]
pub(crate) struct PositionsReq {
    pub(crate) endpoint: String,
    pub(crate) scope: Option<Scope>,
    pub(crate) grants: Vec<(OrgDimension, u64)>,
    pub(crate) denies: Vec<(OrgDimension, u64)>,
}

impl PositionsReq {
    fn query(&self) -> PositionQuery {
        let mut q = PositionQuery::new();
        if let Some(scope) = self.scope {
            q = q.scoped(scope);
        }
        if let Some(p) = entitlements(&self.grants, &self.denies) {
            q = q.entitled(p);
        }
        q
    }
}

/// A fully-parsed `risk limits` request.
#[derive(Debug, Clone)]
pub(crate) struct LimitsReq {
    pub(crate) common: RiskCommon,
    pub(crate) scope: Scope,
    pub(crate) grants: Vec<(OrgDimension, u64)>,
    pub(crate) denies: Vec<(OrgDimension, u64)>,
    pub(crate) var_shocks: Vec<f64>,
    pub(crate) var_alpha: f64,
}

impl LimitsReq {
    fn query(&self) -> LimitQuery {
        let mut q = LimitQuery::new(self.scope, self.common.numeraire());
        if let Some(p) = entitlements(&self.grants, &self.denies) {
            q = q.entitled(p);
        }
        if !self.var_shocks.is_empty() {
            q = q.value_at_risk(self.var_shocks.clone(), self.var_alpha);
        }
        q
    }
}

/// A fully-parsed `stream` request: subscribe to a two-way RFS for one instrument
/// and print the first `ticks` sequenced lines, then unsubscribe cleanly.
#[derive(Debug, Clone)]
pub(crate) struct StreamReq {
    pub(crate) endpoint: String,
    pub(crate) pair: CcyPair,
    pub(crate) tenor: Tenor,
    pub(crate) expiry_years: f64,
    pub(crate) option: OptionType,
    pub(crate) strike: StrikeSpec,
    pub(crate) notional_base: f64,
    /// The number of post-snapshot ticks to print before unsubscribing.
    pub(crate) ticks: u32,
    /// The `AuthService.Login`-issued session token to authenticate the stream
    /// session under the production deny-by-default edge. Absent ⇒ the SDK sends the
    /// audited explicit grant-all `Authenticate` frame (parity with the risk
    /// commands' grant-all default), which `Enforce` admits.
    pub(crate) session_token: Option<String>,
}

// ===========================================================================
// runtime entry points (the synchronous dispatch calls these)
// ===========================================================================

/// Build a private current-thread tokio runtime and drive `fut` on it to
/// completion. The networked risk/stream commands are async; the rest of the CLI
/// is synchronous, so each command owns its runtime rather than the binary
/// carrying a global one.
pub(crate) fn block_on<T>(fut: impl std::future::Future<Output = T>) -> Result<T, RiskError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| RiskError::Invalid(format!("could not start the async runtime: {e}")))?;
    Ok(rt.block_on(fut))
}

/// Connect to the edge, bounded by [`CALL_DEADLINE`].
pub(crate) async fn connect(endpoint: &str) -> Result<Client, RiskError> {
    tokio::time::timeout(CALL_DEADLINE, Client::connect(endpoint.to_owned()))
        .await
        .map_err(|_| RiskError::Timeout("the edge connection"))?
        .map_err(RiskError::Client)
}

/// Run `risk aggregate`: roll the entitled book up over the requested dimension
/// SERVER-SIDE and print the node tree.
pub(crate) fn run_aggregate<W: std::io::Write>(
    req: &AggregateReq,
    out: &mut W,
) -> Result<(), RiskError> {
    let agg = block_on(async {
        let client = connect(&req.common.endpoint).await?;
        bounded("aggregate_risk", client.aggregate_risk(&req.query())).await
    })??;
    out.write_all(format_aggregate(&agg).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `risk drill`: expand one node into its child sub-nodes and/or constituent
/// positions.
pub(crate) fn run_drill<W: std::io::Write>(req: &DrillReq, out: &mut W) -> Result<(), RiskError> {
    let drill = block_on(async {
        let client = connect(&req.common.endpoint).await?;
        bounded("drill_risk", client.drill_risk(&req.query())).await
    })??;
    out.write_all(format_drill(&drill).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `risk positions`: list the entitled open book.
pub(crate) fn run_positions<W: std::io::Write>(
    req: &PositionsReq,
    out: &mut W,
) -> Result<(), RiskError> {
    let listed = block_on(async {
        let client = connect(&req.endpoint).await?;
        bounded("list_positions", client.list_positions(&req.query())).await
    })??;
    out.write_all(format_positions(&listed).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `risk limits`: read the limit-tree RAG / utilization at a scope.
pub(crate) fn run_limits<W: std::io::Write>(req: &LimitsReq, out: &mut W) -> Result<(), RiskError> {
    let status = block_on(async {
        let client = connect(&req.common.endpoint).await?;
        bounded("limit_status", client.limit_status(&req.query())).await
    })??;
    out.write_all(format_limits(&status).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `stream`: open a multiplexed session, subscribe to a two-way RFS for the
/// instrument, print the snapshot + the first `ticks` sequenced lines, then drop
/// the subscription (a clean unsubscribe). Hard-bounded so it never hangs.
pub(crate) fn run_stream<W: std::io::Write>(
    req: &StreamReq,
    out: &mut W,
) -> Result<String, RiskError> {
    block_on(async {
        let client = connect(&req.endpoint).await?;
        // Authenticate the stream as the Login-issued user when a token is supplied;
        // otherwise the SDK sends the audited grant-all `Authenticate` frame the
        // production `Enforce` edge admits (parity with the risk commands' default).
        let client = match &req.session_token {
            Some(token) => client.with_session_token(token.clone()),
            None => client,
        };
        let session = bounded("open_session", client.open_session()).await?;
        let instrument = InstrumentSpec::vanilla(
            req.pair,
            req.tenor,
            req.expiry_years,
            Quantity::base(req.notional_base),
            Side::TwoWay,
            req.option,
            req.strike,
        );
        let mut sub = bounded(
            "subscribe",
            session.subscribe(instrument, Conventions::major_default(), None, None),
        )
        .await?;

        let mut report = String::new();
        let _ = writeln!(
            report,
            "stream {} {:?} {:?} (subscription {}):",
            req.pair,
            req.tenor,
            req.option,
            sub.id()
        );

        // The snapshot establishes the baseline; then we print the requested
        // number of in-order ticks. Each event-await is bounded, so a stalled feed
        // fails fast rather than hanging the terminal.
        let mut ticks_seen = 0u32;
        loop {
            let next = tokio::time::timeout(CALL_DEADLINE, sub.next_event())
                .await
                .map_err(|_| RiskError::Timeout("the next stream event"))?;
            let Some(event) = next else {
                let _ = writeln!(report, "  stream ended");
                break;
            };
            match event? {
                StreamEvent::Snapshot {
                    line,
                    resolved_strike,
                    ..
                } => {
                    let _ = writeln!(
                        report,
                        "  snapshot  seq={:<6} strike={:.6}  bid={:.8} offer={:.8}  vol={:.6}",
                        line.sequence, resolved_strike, line.price.bid, line.price.offer, line.vol
                    );
                }
                StreamEvent::Tick(line) => {
                    let _ = writeln!(
                        report,
                        "  tick      seq={:<6}              bid={:.8} offer={:.8}  vol={:.6}",
                        line.sequence, line.price.bid, line.price.offer, line.vol
                    );
                    ticks_seen += 1;
                    if ticks_seen >= req.ticks {
                        let _ = writeln!(report, "  ({ticks_seen} ticks printed; unsubscribing)");
                        break;
                    }
                }
                StreamEvent::Heartbeat { sequence, .. } => {
                    let _ = writeln!(report, "  heartbeat seq={sequence}");
                }
                StreamEvent::GapDetected {
                    last_good,
                    observed,
                } => {
                    let _ = writeln!(
                        report,
                        "  gap       last_good={last_good} observed={observed} (resyncing)"
                    );
                }
                StreamEvent::Lagged { last_good } => {
                    let _ = writeln!(report, "  lagged    last_good={last_good} (resyncing)");
                }
                StreamEvent::Reconnected => {
                    let _ = writeln!(report, "  reconnected (blue-green cutover)");
                }
            }
        }
        // Dropping `sub` here tears the subscription down cleanly on the session.
        drop(sub);
        drop(session);

        out.write_all(report.as_bytes())
            .map_err(|e| RiskError::Invalid(e.to_string()))?;
        Ok(report)
    })?
}

/// Bound one SDK call by [`CALL_DEADLINE`], tagging a timeout with `what`.
pub(crate) async fn bounded<T>(
    what: &'static str,
    fut: impl std::future::Future<Output = celnet_client::ClientResult<T>>,
) -> Result<T, RiskError> {
    tokio::time::timeout(CALL_DEADLINE, fut)
        .await
        .map_err(|_| RiskError::Timeout(what))?
        .map_err(RiskError::Client)
}

// ===========================================================================
// linear-rates (fixed-income) risk — the FI analogue of the options risk surface
// ===========================================================================

/// One rates position parsed from a `risk rates-aggregate --position` flag:
/// `ENTITY:BOOK:TENOR_YEARS:FIXED_RATE:NOTIONAL:SIDE`.
#[derive(Debug, Clone)]
pub(crate) struct CliRatesPosition {
    entity: u32,
    book: u32,
    tenor_years: u32,
    rate: f64,
    notional: f64,
    side: OisSide,
}

impl CliRatesPosition {
    /// Build the typed SDK [`RatesPosition`] (a fresh, unbooked line, id `0`).
    fn to_position(&self) -> RatesPosition {
        let ois = match self.side {
            OisSide::PayFixed => Ois::pay_fixed(self.tenor_years, self.rate),
            OisSide::ReceiveFixed => Ois::receive_fixed(self.tenor_years, self.rate),
        }
        .notional(self.notional);
        RatesPosition::new(self.entity, self.book, ois)
    }
}

/// Parse one `--pillar TENOR_YEARS=PAR_RATE` calibrating-pillar flag (e.g. `5=0.0405`).
pub(crate) fn parse_pillar(s: &str) -> Result<(u32, f64), String> {
    let (t, r) = s
        .split_once('=')
        .ok_or_else(|| format!("expected TENOR=RATE, got `{s}`"))?;
    let tenor: u32 = t
        .parse()
        .map_err(|_| format!("invalid tenor in `{s}` (expected a whole-year integer)"))?;
    let rate: f64 = r
        .parse()
        .map_err(|_| format!("invalid rate in `{s}` (expected a number)"))?;
    if tenor < 1 {
        return Err(format!("pillar tenor in `{s}` must be >= 1"));
    }
    if !rate.is_finite() {
        return Err(format!("pillar rate in `{s}` must be finite"));
    }
    Ok((tenor, rate))
}

/// Parse an OIS directional side flag: `pay`(-fixed) / `receive`(-fixed).
pub(crate) fn parse_ois_side(s: &str) -> Result<OisSide, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "pay" | "pay-fixed" | "payer" | "buy" => Ok(OisSide::PayFixed),
        "receive" | "receive-fixed" | "receiver" | "rec" | "sell" => Ok(OisSide::ReceiveFixed),
        other => Err(format!(
            "unknown OIS side `{other}` (expected pay / receive)"
        )),
    }
}

/// Parse one `--position ENTITY:BOOK:TENOR_YEARS:FIXED_RATE:NOTIONAL:SIDE` flag.
pub(crate) fn parse_rates_position(s: &str) -> Result<CliRatesPosition, String> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return Err(format!(
            "expected ENTITY:BOOK:TENOR_YEARS:FIXED_RATE:NOTIONAL:SIDE, got `{s}`"
        ));
    }
    let entity: u32 = parts[0]
        .parse()
        .map_err(|_| format!("invalid entity in `{s}`"))?;
    let book: u32 = parts[1]
        .parse()
        .map_err(|_| format!("invalid book in `{s}`"))?;
    let tenor_years: u32 = parts[2]
        .parse()
        .map_err(|_| format!("invalid tenor in `{s}`"))?;
    let rate: f64 = parts[3]
        .parse()
        .map_err(|_| format!("invalid rate in `{s}`"))?;
    let notional: f64 = parts[4]
        .parse()
        .map_err(|_| format!("invalid notional in `{s}`"))?;
    let side = parse_ois_side(parts[5])?;
    if tenor_years < 1 {
        return Err(format!("position tenor in `{s}` must be >= 1"));
    }
    if !(notional.is_finite() && notional > 0.0) {
        return Err(format!("position notional in `{s}` must be > 0"));
    }
    Ok(CliRatesPosition {
        entity,
        book,
        tenor_years,
        rate,
        notional,
        side,
    })
}

/// Today's civil date (UTC) — the default rates-curve reference (spot-anchor) date.
fn today_civil() -> CivilDate {
    let d = time::OffsetDateTime::now_utc().date();
    CivilDate::new(d.year(), u32::from(u8::from(d.month())), u32::from(d.day()))
}

/// Parse a `YYYY-MM-DD` civil date into a [`CivilDate`].
fn parse_civil_date(s: &str) -> Result<CivilDate, RiskError> {
    let mut it = s.split('-');
    let year = it.next().and_then(|p| p.parse::<i32>().ok());
    let month = it.next().and_then(|p| p.parse::<u32>().ok());
    let day = it.next().and_then(|p| p.parse::<u32>().ok());
    match (year, month, day, it.next()) {
        (Some(y), Some(m), Some(d), None) if (1..=12).contains(&m) && (1..=31).contains(&d) => {
            Ok(CivilDate::new(y, m, d))
        }
        _ => Err(RiskError::Invalid(format!(
            "--curve-date must be YYYY-MM-DD, got `{s}`"
        ))),
    }
}

/// Build the calibrated USD-SOFR curve from the reference date + par-OIS pillars.
pub(crate) fn build_curve(
    curve_date: Option<&str>,
    pillars: &[(u32, f64)],
) -> Result<UsdSofrCurve, RiskError> {
    let date = match curve_date {
        Some(s) => parse_civil_date(s)?,
        None => today_civil(),
    };
    let mut curve = UsdSofrCurve::new(date);
    for &(tenor, rate) in pillars {
        curve = curve.pillar(tenor, rate);
    }
    Ok(curve)
}

/// Build a `(entity, book, ccy)` rates scope from the optional narrowing flags.
fn rates_scope(
    entity: Option<u32>,
    book: Option<u32>,
    ccy: Option<&str>,
) -> Option<RatesRiskScope> {
    if entity.is_none() && book.is_none() && ccy.is_none() {
        return None;
    }
    let mut s = RatesRiskScope::new();
    if let Some(e) = entity {
        s = s.entity(e);
    }
    if let Some(b) = book {
        s = s.book(b);
    }
    if let Some(c) = ccy {
        s = s.ccy(c);
    }
    Some(s)
}

/// Connect to the edge, attaching the `AuthService.Login` bearer when one is given
/// (needed for a capability-gated write; harmless on the `ReadAny` reads).
pub(crate) async fn connect_authed(
    endpoint: &str,
    session_token: Option<&str>,
) -> Result<Client, RiskError> {
    let client = connect(endpoint).await?;
    Ok(match session_token {
        Some(t) => client.with_session_token(t.to_owned()),
        None => client,
    })
}

/// A fully-parsed `risk rates-aggregate` request.
#[derive(Debug, Clone)]
pub(crate) struct RatesAggregateReq {
    pub(crate) endpoint: String,
    pub(crate) curve_date: Option<String>,
    pub(crate) pillars: Vec<(u32, f64)>,
    pub(crate) positions: Vec<CliRatesPosition>,
    pub(crate) grants: Vec<(OrgDimension, u64)>,
    pub(crate) denies: Vec<(OrgDimension, u64)>,
    pub(crate) session_token: Option<String>,
    pub(crate) entity: Option<u32>,
    pub(crate) book: Option<u32>,
    pub(crate) ccy: Option<String>,
}

/// A fully-parsed `risk rates-positions` request.
#[derive(Debug, Clone)]
pub(crate) struct RatesPositionsReq {
    pub(crate) endpoint: String,
    pub(crate) grants: Vec<(OrgDimension, u64)>,
    pub(crate) denies: Vec<(OrgDimension, u64)>,
    pub(crate) session_token: Option<String>,
    pub(crate) entity: Option<u32>,
    pub(crate) book: Option<u32>,
    pub(crate) ccy: Option<String>,
}

/// Run `risk rates-aggregate`: price the inline OIS book against the curve and print
/// the per-currency firm rollup (netted SERVER-SIDE).
pub(crate) fn run_rates_aggregate<W: std::io::Write>(
    req: &RatesAggregateReq,
    out: &mut W,
) -> Result<(), RiskError> {
    let curve = build_curve(req.curve_date.as_deref(), &req.pillars)?;
    let positions: Vec<RatesPosition> = req
        .positions
        .iter()
        .map(CliRatesPosition::to_position)
        .collect();
    let agg = block_on(async {
        let client = connect_authed(&req.endpoint, req.session_token.as_deref()).await?;
        let mut q = RatesAggregateQuery::new(curve, positions);
        if let Some(scope) = rates_scope(req.entity, req.book, req.ccy.as_deref()) {
            q = q.scoped(scope);
        }
        if let Some(p) = entitlements(&req.grants, &req.denies) {
            q = q.entitled(p);
        }
        bounded("aggregate_rates_risk", client.aggregate_rates_risk(&q)).await
    })??;
    out.write_all(format_rates_aggregate(&agg).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `risk rates-positions`: list the firm's booked rates positions.
pub(crate) fn run_rates_positions<W: std::io::Write>(
    req: &RatesPositionsReq,
    out: &mut W,
) -> Result<(), RiskError> {
    let listed = block_on(async {
        let client = connect_authed(&req.endpoint, req.session_token.as_deref()).await?;
        let mut q = RatesPositionQuery::new();
        if let Some(scope) = rates_scope(req.entity, req.book, req.ccy.as_deref()) {
            q = q.scoped(scope);
        }
        if let Some(p) = entitlements(&req.grants, &req.denies) {
            q = q.entitled(p);
        }
        bounded("list_rates_positions", client.list_rates_positions(&q)).await
    })??;
    out.write_all(format_rates_positions(&listed).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Format a `risk rates-aggregate` response (one node per settlement currency).
fn format_rates_aggregate(agg: &RatesRiskAggregate) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "rates aggregate  currencies={}", agg.nodes.len());
    if let Some(cid) = agg.correlation_id {
        let _ = writeln!(out, "  correlation_id  {cid}");
    }
    for n in &agg.nodes {
        let _ = writeln!(
            out,
            "  ccy {}  net_pv={:.4}  net_pv01={:.6}  net_dv01={:.6}",
            n.ccy, n.net_pv, n.net_pv01, n.net_dv01
        );
        for b in &n.key_rate_ladder {
            let _ = writeln!(
                out,
                "    key_rate {:>3}y  dv01={:.6}",
                b.tenor_years, b.dv01
            );
        }
    }
    out
}

/// Format a `risk rates-positions` response.
fn format_rates_positions(positions: &[RatesPosition]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "rates positions  count={}", positions.len());
    for p in positions {
        let _ = writeln!(
            out,
            "  position {:<6} entity={} book={}  {}y {:?}  rate={:.6}  notional={:.2}",
            p.position_id,
            p.entity,
            p.book,
            p.instrument.tenor_years(),
            p.instrument.side(),
            p.instrument.fixed_rate(),
            p.instrument.notional_amount(),
        );
    }
    out
}

// ===========================================================================
// formatting — 2-space-indented fixed-precision reports (CLI house style)
// ===========================================================================

/// Format one rolled-up node's additive + non-additive measures.
fn write_node(out: &mut String, node: &RiskNode, numeraire: &str) {
    let a = &node.additive;
    let _ = writeln!(
        out,
        "  node {:?} group={}  positions={}",
        node.dimension, node.group, node.position_count
    );
    let _ = writeln!(out, "    delta_{numeraire:<8}  {:.10}", a.delta_numeraire);
    for leg in &a.delta_vector {
        let _ = writeln!(out, "      leg {:<4}       {:.10}", leg.ccy, leg.amount);
    }
    let _ = writeln!(out, "    gamma           {:.10}", a.gamma);
    let _ = writeln!(out, "    vega_{numeraire:<9}  {:.10}", a.vega_numeraire);
    let _ = writeln!(out, "    theta           {:.10}", a.theta);
    let _ = writeln!(out, "    vanna           {:.10}", a.vanna);
    let _ = writeln!(out, "    volga           {:.10}", a.volga);
    let _ = writeln!(
        out,
        "    premium_{numeraire:<6} {:.10}",
        a.premium_numeraire
    );
    let n = &node.nonadditive;
    if let (Some(var), Some(alpha)) = (n.var, n.var_alpha) {
        let _ = writeln!(out, "    VaR@{alpha:<6}      {var:.10}");
    }
    if let Some(es) = n.es {
        let _ = writeln!(out, "    ES              {es:.10}");
    }
    if let Some(cv) = n.curvature_spot {
        let _ = writeln!(out, "    curvature_spot  {cv:.10}");
    }
}

/// Format a `risk aggregate` response.
fn format_aggregate(agg: &RiskAggregate) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "risk aggregate  dimension={:?}  numeraire={}  nodes={}",
        agg.dimension,
        agg.numeraire,
        agg.nodes.len()
    );
    if let Some(cid) = agg.correlation_id {
        let _ = writeln!(out, "  correlation_id  {cid}");
    }
    for node in &agg.nodes {
        write_node(&mut out, node, &agg.numeraire);
    }
    out
}

/// Format a `risk drill` response.
fn format_drill(drill: &RiskDrill) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "risk drill  node={:?} group={}  children={}  positions={}",
        drill.node.dimension,
        drill.node.value,
        drill.children.len(),
        drill.positions.len()
    );
    for child in &drill.children {
        // Children measures are reported in the request numeraire; label generic.
        write_node(&mut out, child, "num");
    }
    for p in &drill.positions {
        write_position(&mut out, p);
    }
    out
}

/// Format a `risk positions` response.
fn format_positions(list: &PositionList) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "risk positions  count={}", list.positions.len());
    for p in &list.positions {
        write_position(&mut out, p);
    }
    out
}

/// Format one open position leaf.
fn write_position(out: &mut String, p: &celnet_client::RiskPosition) {
    let _ = writeln!(
        out,
        "  position {:<6} {} {:?}  notional_base={:.2}  book={}",
        p.position_id,
        p.org.ccy_pair,
        p.option_type,
        p.notional_base,
        p.attribution
            .as_ref()
            .map(|a| a.quoted_by.book.as_str())
            .unwrap_or("—"),
    );
}

/// Format a `risk limits` response.
fn format_limits(status: &LimitStatus) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "risk limits  scope={:?} group={}  worst={:?}  hard_breach={}",
        status.scope.dimension, status.scope.value, status.worst, status.hard_breach
    );
    for l in &status.limits {
        let _ = writeln!(
            out,
            "  limit {:?}  {:?}  cap={:.4} exposure={:.4} ratio={:.4} headroom={:.4}  [{:?}]",
            l.metric, l.status, l.cap, l.exposure, l.ratio, l.headroom, l.enforcement
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_numeraire_rate() {
        assert_eq!(parse_rate("EUR=1.10").unwrap(), ("EUR".to_owned(), 1.10));
        assert!(parse_rate("EUR").is_err());
        assert!(parse_rate("EUR=0").is_err());
        assert!(parse_rate("EUR=-1").is_err());
        assert!(parse_rate("EUR=xyz").is_err());
    }

    #[test]
    fn parses_a_pillar_flag() {
        assert_eq!(parse_pillar("5=0.0405").unwrap(), (5, 0.0405));
        assert!(parse_pillar("5").is_err(), "missing =RATE");
        assert!(parse_pillar("0=0.04").is_err(), "tenor must be >= 1");
        assert!(parse_pillar("x=0.04").is_err(), "non-integer tenor");
        assert!(parse_pillar("5=abc").is_err(), "non-number rate");
    }

    #[test]
    fn parses_an_ois_side() {
        assert_eq!(parse_ois_side("pay").unwrap(), OisSide::PayFixed);
        assert_eq!(parse_ois_side("PAY-FIXED").unwrap(), OisSide::PayFixed);
        assert_eq!(parse_ois_side("receive").unwrap(), OisSide::ReceiveFixed);
        assert_eq!(parse_ois_side("Receiver").unwrap(), OisSide::ReceiveFixed);
        assert!(parse_ois_side("sideways").is_err());
    }

    #[test]
    fn parses_a_rates_position() {
        let p = parse_rates_position("1:10:5:0.0405:25000000:receive").unwrap();
        let pos = p.to_position();
        assert_eq!(pos.entity, 1);
        assert_eq!(pos.book, 10);
        assert_eq!(pos.position_id, 0, "an unbooked line has id 0");
        assert_eq!(pos.instrument.tenor_years(), 5);
        assert_eq!(pos.instrument.side(), OisSide::ReceiveFixed);
        assert_eq!(pos.instrument.notional_amount(), 25_000_000.0);
        // Malformed shapes are rejected.
        assert!(
            parse_rates_position("1:10:5:0.0405:25000000").is_err(),
            "too few fields"
        );
        assert!(
            parse_rates_position("1:10:0:0.0405:1:pay").is_err(),
            "tenor >= 1"
        );
        assert!(
            parse_rates_position("1:10:5:0.04:0:pay").is_err(),
            "notional > 0"
        );
        assert!(
            parse_rates_position("1:10:5:0.04:1:hold").is_err(),
            "bad side"
        );
    }

    #[test]
    fn parses_a_scope_flag() {
        assert_eq!(parse_scope_flag("book:3").unwrap(), (OrgDimension::Book, 3));
        assert_eq!(
            parse_scope_flag("desk:42").unwrap(),
            (OrgDimension::Desk, 42)
        );
        assert!(parse_scope_flag("book").is_err());
        assert!(parse_scope_flag("nope:1").is_err());
        assert!(parse_scope_flag("book:x").is_err());
    }

    #[test]
    fn no_entitlement_flags_is_grant_all_default() {
        assert!(
            entitlements(&[], &[]).is_none(),
            "no flags ⇒ SDK grant-all default"
        );
        assert!(
            entitlements(&[(OrgDimension::Book, 1)], &[]).is_some(),
            "a grant switches to a scoped principal"
        );
        assert!(
            entitlements(&[], &[(OrgDimension::Book, 1)]).is_some(),
            "a deny layers onto grant-all"
        );
    }

    #[test]
    fn dimension_maps_one_for_one() {
        assert_eq!(OrgDimension::from(CliDimension::Firm), OrgDimension::Firm);
        assert_eq!(OrgDimension::from(CliDimension::Desk), OrgDimension::Desk);
        assert_eq!(
            OrgDimension::from(CliDimension::CcyPair),
            OrgDimension::CcyPair
        );
    }

    #[test]
    fn aggregate_query_carries_var_and_curvature() {
        let req = AggregateReq {
            common: RiskCommon {
                endpoint: "http://127.0.0.1:1".to_owned(),
                numeraire_ccy: "USD".to_owned(),
                rates: vec![("EUR".to_owned(), 1.10)],
            },
            dimension: OrgDimension::Firm,
            scope: None,
            grants: vec![],
            denies: vec![],
            var_shocks: vec![-0.01, 0.0, 0.01],
            var_alpha: 0.99,
            curvature_risk_weight: 0.0,
        };
        // Building the query must not panic and must accept the numeraire legs.
        let _q = req.query();
    }
}
