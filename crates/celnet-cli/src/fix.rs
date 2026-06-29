//! The `fix` subcommand: a FIX 4.4 **initiator** (price-taker) test client that
//! drives the Celnet FIX↔gRPC quoting gateway over a real socket.
//!
//! It connects to the gateway's acceptor, logs on, sends a rates
//! `QuoteRequest(R)` (a USD-SOFR OIS RFQ), prints the returned `Quote(S)`, lifts
//! it with a `NewOrderSingle(D)` against the `QuoteID`, and prints the resulting
//! `ExecutionReport(8)` — the same `celnet-fix` [`Initiator`] the engine's
//! loopback tests use, so the CLI exercises the real session + dialect, not a
//! re-implementation.
//!
//! The client targets the **gateway's FIX listener** (`--host`/`--port`), not the
//! gRPC edge directly: locally `127.0.0.1:9880`, and on UAT the address the
//! gateway process listens on (the gateway in turn bridges to the
//! `celnet@136.115.32.199` edge). No live counterparty venue is required —
//! `celnet-fix`'s offline conformance test is the correctness proof.

use std::io::Write;
use std::time::Duration;

use celnet_fix::dialect_rates::{self, RatesQuoteRequestParams, RatesSide, SubscriptionRequest};
use celnet_fix::gateway::now_fix_utc;
use celnet_fix::initiator::{Initiator, InitiatorResult, LiftPolicy};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use tokio::net::TcpStream;

/// The hard ceiling on the whole connect → logon → quote → lift cycle, so an
/// unreachable gateway fails fast rather than hanging the terminal.
const CYCLE_DEADLINE: Duration = Duration::from_secs(15);

/// The CLI-facing directional intent of a rates RFQ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliRatesSide {
    /// Pay the fixed leg (`Side(54)=1`).
    PayFixed,
    /// Receive the fixed leg (`Side(54)=2`).
    ReceiveFixed,
    /// Request a two-way market (no `Side`).
    TwoWay,
}

impl From<CliRatesSide> for RatesSide {
    fn from(v: CliRatesSide) -> Self {
        match v {
            CliRatesSide::PayFixed => RatesSide::PayFixed,
            CliRatesSide::ReceiveFixed => RatesSide::ReceiveFixed,
            CliRatesSide::TwoWay => RatesSide::TwoWay,
        }
    }
}

/// How the client reacts to the returned quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliLift {
    /// Lift the offer (BUY) — book a fill.
    Offer,
    /// Hit the bid (SELL) — book a fill.
    Bid,
    /// Observe the quote without lifting (indicative).
    Observe,
}

impl From<CliLift> for LiftPolicy {
    fn from(v: CliLift) -> Self {
        match v {
            CliLift::Offer => LiftPolicy::LiftOffer,
            CliLift::Bid => LiftPolicy::HitBid,
            CliLift::Observe => LiftPolicy::Observe,
        }
    }
}

/// The fully-parsed `fix` request.
#[derive(Debug, Clone)]
pub(crate) struct FixRequest {
    /// The gateway FIX listener host.
    pub(crate) host: String,
    /// The gateway FIX listener port.
    pub(crate) port: u16,
    /// Our `SenderCompID` (the initiator identity the gateway expects as its target).
    pub(crate) sender_comp_id: String,
    /// The gateway `TargetCompID` (the gateway's `SenderCompID`).
    pub(crate) target_comp_id: String,
    /// The OIS curve symbol.
    pub(crate) symbol: String,
    /// The OIS tenor in whole years (`>= 1`).
    pub(crate) tenor_years: u32,
    /// The notional in curve currency (`> 0`).
    pub(crate) notional: f64,
    /// The directional intent.
    pub(crate) side: CliRatesSide,
    /// The quote reaction.
    pub(crate) lift: CliLift,
    /// The session heartbeat interval (seconds).
    pub(crate) heartbeat: u32,
}

/// A `fix` subcommand failure.
#[derive(Debug)]
pub(crate) enum FixError {
    /// A bad argument before any round-trip.
    Invalid(String),
    /// A transport / protocol failure over the socket.
    Io(std::io::Error),
    /// The cycle exceeded [`CYCLE_DEADLINE`].
    Timeout,
}

impl core::fmt::Display for FixError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FixError::Invalid(s) => write!(f, "invalid argument: {s}"),
            FixError::Io(e) => write!(f, "FIX transport error: {e}"),
            FixError::Timeout => write!(f, "timed out talking to the gateway"),
        }
    }
}

impl std::error::Error for FixError {}

impl From<std::io::Error> for FixError {
    fn from(e: std::io::Error) -> Self {
        FixError::Io(e)
    }
}

/// Run the `fix` subcommand: connect to the gateway, run one RFQ cycle, and write
/// the formatted report to `out`.
///
/// # Errors
/// [`FixError`] on a bad argument, a transport/protocol failure, or a timeout.
pub(crate) fn run<W: Write>(req: &FixRequest, out: &mut W) -> Result<(), FixError> {
    if req.tenor_years < 1 {
        return Err(FixError::Invalid("--tenor-years must be >= 1".to_owned()));
    }
    if !(req.notional.is_finite() && req.notional > 0.0) {
        return Err(FixError::Invalid(
            "--notional must be finite and > 0".to_owned(),
        ));
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| FixError::Invalid(format!("could not start the async runtime: {e}")))?;

    let result = rt.block_on(async {
        tokio::time::timeout(CYCLE_DEADLINE, run_cycle(req))
            .await
            .map_err(|_| FixError::Timeout)?
    })?;

    write_report(req, &result, out).map_err(FixError::Io)
}

/// The async connect → logon → request → (lift) → report cycle.
async fn run_cycle(req: &FixRequest) -> Result<InitiatorResult, FixError> {
    let stream = TcpStream::connect((req.host.as_str(), req.port)).await?;

    let session = Session::new(
        SessionConfig {
            sender: req.sender_comp_id.clone().into_bytes(),
            target: req.target_comp_id.clone().into_bytes(),
            heart_bt_int: req.heartbeat,
            role: Role::Initiator,
        },
        InMemoryStore::new(),
    );
    let mut initiator = Initiator::new(session, req.lift.into());

    let symbol = req.symbol.clone().into_bytes();
    let quote_req_id = format!("CLI-{}", std::process::id()).into_bytes();
    let side: RatesSide = req.side.into();
    let tenor_years = req.tenor_years;
    let notional = req.notional;

    let result = initiator
        .request_and_lift(stream, now_fix_utc(), move |h, e| {
            dialect_rates::build_rates_quote_request(
                h,
                &RatesQuoteRequestParams {
                    quote_req_id: &quote_req_id,
                    symbol: &symbol,
                    tenor_years,
                    notional,
                    side,
                    subscription: SubscriptionRequest::Snapshot,
                },
                e,
            )
        })
        .await?;
    Ok(result)
}

/// Format the cycle outcome in the CLI's 2-space-indented house style.
fn write_report<W: Write>(
    req: &FixRequest,
    r: &InitiatorResult,
    out: &mut W,
) -> std::io::Result<()> {
    writeln!(
        out,
        "FIX rates RFQ → {}:{} (desk gateway {})",
        req.host, req.port, req.target_comp_id
    )?;
    writeln!(
        out,
        "  instrument      {} OIS {}Y  notional {:.2}  side {:?}",
        req.symbol, req.tenor_years, req.notional, req.side
    )?;
    match &r.quote_id {
        Some(qid) => writeln!(out, "  quote_id        {}", String::from_utf8_lossy(qid))?,
        None => {
            writeln!(
                out,
                "  quote           none received (request rejected or no desk response)"
            )?;
            return Ok(());
        }
    }
    if let Some(bid) = r.bid {
        writeln!(out, "  bid             {bid:.10}")?;
    }
    if let Some(offer) = r.offer {
        writeln!(out, "  offer           {offer:.10}")?;
    }
    match (r.filled, r.fill_px) {
        (true, Some(px)) => writeln!(out, "  execution       FILLED @ {px:.10}")?,
        (true, None) => writeln!(out, "  execution       FILLED")?,
        (false, _) => writeln!(out, "  execution       not lifted")?,
    }
    Ok(())
}
