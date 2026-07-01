//! The `desk` subcommand: drive the dealer-side quoting desk (`RfqDeskService`)
//! against a running edge through the **same** typed [`celnet_client`] SDK the GUI
//! desk view consumes (the api-first client-parity rule).
//!
//! Every number here is the SERVER's desk state surfaced verbatim — the CLI adds no
//! logic of its own: `desk submit` calls
//! [`DeskClient::submit`](celnet_client::DeskClient::submit), `desk respond`/`decline`
//! call [`DeskClient::respond_quote`](celnet_client::DeskClient::respond_quote) /
//! [`DeskClient::decline`](celnet_client::DeskClient::decline), `desk accept` calls
//! [`DeskClient::accept`](celnet_client::DeskClient::accept), and `desk requests` /
//! `desk deals` read the inbox / blotter. The runtime/connect/deadline plumbing and
//! [`RiskError`] are shared with the [`crate::risk`] networked commands.

use std::fmt::Write as _;
use std::time::Duration;

use celnet_client::{
    Deal, DealFilter, DeskAcceptance, DeskClient, DeskQuote, DeskRequest, DeskRequestFilter,
    DeskRfq, Ois, OisSide,
};

use crate::risk::{RiskError, block_on, bounded, build_curve, connect_authed};

/// A fully-parsed `desk submit` request.
#[derive(Debug, Clone)]
pub(crate) struct SubmitReq {
    pub(crate) endpoint: String,
    pub(crate) session_token: Option<String>,
    pub(crate) counterparty: String,
    pub(crate) desk: String,
    pub(crate) tenor_years: u32,
    pub(crate) rate: f64,
    pub(crate) notional: f64,
    pub(crate) side: OisSide,
    pub(crate) ioi: bool,
    pub(crate) ttl_ms: u32,
    pub(crate) curve_date: Option<String>,
    pub(crate) pillars: Vec<(u32, f64)>,
}

/// A fully-parsed `desk respond` request.
#[derive(Debug, Clone)]
pub(crate) struct RespondReq {
    pub(crate) endpoint: String,
    pub(crate) session_token: Option<String>,
    pub(crate) request_id: String,
    pub(crate) price: f64,
    pub(crate) notional: f64,
    pub(crate) valid_ms: u32,
    pub(crate) trader: String,
}

/// A fully-parsed `desk decline` request.
#[derive(Debug, Clone)]
pub(crate) struct DeclineReq {
    pub(crate) endpoint: String,
    pub(crate) session_token: Option<String>,
    pub(crate) request_id: String,
    pub(crate) reason: String,
}

/// A fully-parsed `desk accept` request.
#[derive(Debug, Clone)]
pub(crate) struct AcceptReq {
    pub(crate) endpoint: String,
    pub(crate) session_token: Option<String>,
    pub(crate) request_id: String,
}

/// A fully-parsed `desk requests` / `desk deals` read request (an optional desk).
#[derive(Debug, Clone)]
pub(crate) struct ReadReq {
    pub(crate) endpoint: String,
    pub(crate) session_token: Option<String>,
    pub(crate) desk: Option<String>,
}

/// Build the OIS the counterparty wants priced from the submit flags.
fn submit_ois(req: &SubmitReq) -> Ois {
    let ois = match req.side {
        OisSide::PayFixed => Ois::pay_fixed(req.tenor_years, req.rate),
        OisSide::ReceiveFixed => Ois::receive_fixed(req.tenor_years, req.rate),
    };
    ois.notional(req.notional)
}

/// Run `desk submit`: capture an inbound RFQ/IOI on a desk and print it.
pub(crate) fn run_submit<W: std::io::Write>(req: &SubmitReq, out: &mut W) -> Result<(), RiskError> {
    let curve = build_curve(req.curve_date.as_deref(), &req.pillars)?;
    let instrument = submit_ois(req);
    let rfq = {
        let base = if req.ioi {
            DeskRfq::ioi(
                req.counterparty.clone(),
                req.desk.clone(),
                instrument,
                curve,
            )
        } else {
            DeskRfq::rfq(
                req.counterparty.clone(),
                req.desk.clone(),
                instrument,
                curve,
            )
        };
        if req.ttl_ms == 0 {
            base
        } else {
            base.ttl(Duration::from_millis(u64::from(req.ttl_ms)))
        }
    };
    let request = block_on(async {
        let desk = connect_desk(&req.endpoint, req.session_token.as_deref()).await?;
        bounded("submit_desk_request", desk.submit(rfq)).await
    })??;
    out.write_all(format_request("submitted", &request).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `desk respond`: quote a pending desk request at a firm level.
pub(crate) fn run_respond<W: std::io::Write>(
    req: &RespondReq,
    out: &mut W,
) -> Result<(), RiskError> {
    let quote = DeskQuote::new(
        req.price,
        req.notional,
        Duration::from_millis(u64::from(req.valid_ms)),
        req.trader.clone(),
    );
    let request = block_on(async {
        let desk = connect_desk(&req.endpoint, req.session_token.as_deref()).await?;
        bounded(
            "respond_desk_request",
            desk.respond_quote(req.request_id.clone(), quote),
        )
        .await
    })??;
    out.write_all(format_request("quoted", &request).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `desk decline`: decline to quote a pending desk request.
pub(crate) fn run_decline<W: std::io::Write>(
    req: &DeclineReq,
    out: &mut W,
) -> Result<(), RiskError> {
    let request = block_on(async {
        let desk = connect_desk(&req.endpoint, req.session_token.as_deref()).await?;
        bounded(
            "respond_desk_request",
            desk.decline(req.request_id.clone(), req.reason.clone()),
        )
        .await
    })??;
    out.write_all(format_request("declined", &request).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `desk accept`: lift a quoted request → book a deal + rates position.
pub(crate) fn run_accept<W: std::io::Write>(req: &AcceptReq, out: &mut W) -> Result<(), RiskError> {
    let accepted = block_on(async {
        let desk = connect_desk(&req.endpoint, req.session_token.as_deref()).await?;
        bounded("accept_desk_quote", desk.accept(req.request_id.clone())).await
    })??;
    out.write_all(format_acceptance(&accepted).as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `desk requests`: read the desk inbox.
pub(crate) fn run_requests<W: std::io::Write>(req: &ReadReq, out: &mut W) -> Result<(), RiskError> {
    let requests = block_on(async {
        let desk = connect_desk(&req.endpoint, req.session_token.as_deref()).await?;
        let mut filter = DeskRequestFilter::new();
        if let Some(d) = &req.desk {
            filter = filter.desk(d.clone());
        }
        bounded("list_desk_requests", desk.list_requests(&filter)).await
    })??;
    let mut report = String::new();
    let _ = writeln!(report, "desk requests  count={}", requests.len());
    for r in &requests {
        report.push_str(&format_request_line(r));
    }
    out.write_all(report.as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Run `desk deals`: read the received-deals blotter.
pub(crate) fn run_deals<W: std::io::Write>(req: &ReadReq, out: &mut W) -> Result<(), RiskError> {
    let deals = block_on(async {
        let desk = connect_desk(&req.endpoint, req.session_token.as_deref()).await?;
        let mut filter = DealFilter::new();
        if let Some(d) = &req.desk {
            filter = filter.desk(d.clone());
        }
        bounded("list_deals", desk.list_deals(&filter)).await
    })??;
    let mut report = String::new();
    let _ = writeln!(report, "desk deals  count={}", deals.len());
    for d in &deals {
        report.push_str(&format_deal_line(d));
    }
    out.write_all(report.as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Connect + build the typed desk handle (authenticated when a token is supplied).
async fn connect_desk(
    endpoint: &str,
    session_token: Option<&str>,
) -> Result<DeskClient, RiskError> {
    Ok(connect_authed(endpoint, session_token).await?.desk())
}

/// Format a single-request report (submit / respond / decline).
fn format_request(verb: &str, r: &DeskRequest) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "desk {verb}  request_id={}", r.request_id);
    out.push_str(&format_request_line(r));
    out
}

/// Format one desk-request line.
fn format_request_line(r: &DeskRequest) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "  {:<10} {:?} {:?}  cp={} desk={}  {}y {:?} rate={:.6} notional={:.2}",
        r.request_id,
        r.kind,
        r.state,
        r.counterparty,
        r.desk,
        r.instrument.tenor_years(),
        r.instrument.side(),
        r.instrument.fixed_rate(),
        r.notional,
    );
    if let Some(q) = &r.quote {
        let _ = writeln!(
            out,
            "    quote price={:.6} notional={:.2} valid_ms={} trader={}",
            q.price,
            q.notional,
            q.valid_for.as_millis(),
            q.trader,
        );
    }
    out
}

/// Format the acceptance report (the booked deal + terminal request).
fn format_acceptance(a: &DeskAcceptance) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "desk accepted  deal_id={}", a.deal.deal_id);
    out.push_str(&format_deal_line(&a.deal));
    out.push_str(&format_request_line(&a.request));
    out
}

/// Format one booked-deal line.
fn format_deal_line(d: &Deal) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "  {:<10} req={} {:?}  cp={} desk={}  {:?} price={:.6} notional={:.2}  trader={} position_id={}",
        d.deal_id,
        d.request_id,
        d.kind,
        d.counterparty,
        d.desk,
        d.side,
        d.price,
        d.notional,
        d.trader,
        d.position_id
            .map_or_else(|| "—".to_owned(), |p| p.to_string()),
    );
    out
}
